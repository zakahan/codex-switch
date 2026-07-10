# codex-switch

A tiny Linux CLI to snapshot and switch [Codex](https://github.com/openai/codex)
credentials. It maintains multiple copies of your `~/.codex/auth.json` +
`~/.codex/config.toml` as named **profiles** and swaps them in and out with a
single command.

Think of it as a snapshot manager for your Codex config: each profile is a
complete, self-contained copy of the two files, and switching atomically
replaces the live files with the profile's version.

It is inspired by [cc-switch](https://github.com/farion1231/cc-switch) (a Tauri
desktop app), but is Codex-only, CLI-only, single-binary, and has no runtime
dependencies. Adding/editing providers is done by hand — this tool only handles
storing and switching snapshots.

## Why

- **cc-switch has no Linux CLI.** This fills that gap for Codex.
- You keep several Codex setups (official ChatGPT login, a third-party
  provider, a work key, a personal key, …) and want to flip between them without
  hand-editing `auth.json` / `config.toml` each time.

## How it works

```
~/.codex/                      # the LIVE config that Codex actually reads
  auth.json
  config.toml

<store>/                       # codex-switch's storage (see "Where things live")
  profiles/
    work/
      auth.json                # a full snapshot of both files
      config.toml
    personal/
      auth.json
      config.toml
  backup/                      # last live config, saved before each `use`
    auth.json
    config.toml
  state.json                   # { "active": "<profile>" }
```

- A **profile** is just a directory holding a copy of the two files.
- **Switching** (`use`) writes the profile's files over the live ones.
- It is a **pure snapshot** model: after switching, edits you make to the live
  files are *not* automatically pushed back into the profile. Use `save` to
  capture such changes (drift) back into a profile.

### Safety properties

- **Atomic writes.** Each file is written to a temp file in the same directory
  and then `rename`d over the target, so Codex never sees a half-written file.
- **Paired write with rollback.** `auth.json` is written first, then
  `config.toml`; if the second write fails, the first is rolled back so you are
  never left with a mismatched pair.
- **Credential permissions.** `auth.json` is always written with `0600`
  (owner read/write only).
- **Automatic backup.** Before every `use`, the current live config is copied to
  `<store>/backup/` so a bad switch is recoverable.
- **Symlink-safe.** Works correctly when `$HOME` (or `~/.codex`) is a symlink —
  writes resolve through the link to the real file.

## Where things live

**Live files** — resolved from `CODEX_HOME` if set, otherwise `~/.codex`:

- `$CODEX_HOME/auth.json`
- `$CODEX_HOME/config.toml`

**The store** — resolved in this order:

1. `$CODEX_SWITCH_HOME`, if set.
2. An existing `~/.codex-switch` or `$XDG_DATA_HOME/codex-switch`.
3. `~/.codex-switch`, if your home directory is writable.
4. `$XDG_DATA_HOME/codex-switch` (defaults to `~/.local/share/codex-switch`)
   as a fallback when home is read-only.

The fallback matters on setups where the home directory is a **read-only mount**
(e.g. `~` is a symlink into a read-only filesystem). Run `codex-switch paths` to
see exactly what is being used and why.

To pin the store to an explicit location:

```sh
export CODEX_SWITCH_HOME="$HOME/.local/share/codex-switch"
```

## Install

Requires a Rust toolchain (`cargo`). Don't have one? Install via
[rustup](https://rustup.rs):

```sh
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Recommended: install as a global command

From the repo root, this compiles and installs a `codex-switch` binary into
`~/.cargo/bin`, which rustup already puts on your `PATH`:

```sh
cargo install --path .
```

After that you can run `codex-switch` from anywhere — no need to be in the repo:

```sh
codex-switch --version
```

To update after pulling new changes, run `cargo install --path .` again (it
overwrites the installed binary). To uninstall: `cargo uninstall codex-switch`.

### Alternative: build and copy the binary

If you'd rather not use `cargo install`:

```sh
cargo build --release
install -Dm755 target/release/codex-switch ~/.local/bin/codex-switch
```

Make sure the target directory is on your `PATH`. For `~/.local/bin`, add this
to your `~/.bashrc` / `~/.zshrc` if it isn't already:

```sh
export PATH="$HOME/.local/bin:$PATH"
```

## Usage

```
codex-switch <command>

Commands:
  import <name> [--activate] [--force]   Save the current live config as a new profile
  list (ls)                              List profiles; the active one is marked with *
  current                                Print the active profile name
  use <name>                             Switch the live config to <name> (backs up live first)
  save [name]                            Write the current live config back into a profile
                                         (defaults to the active profile)
  diff [name]                            Show which files differ between a profile and live
                                         (defaults to the active profile)
  rm (remove) <name> [--force]           Delete a profile
  paths                                  Print resolved paths and the store location
```

### First-time setup

Capture whatever Codex config you have right now as your first profile:

```sh
codex-switch import default --activate
```

### Typical workflow

```sh
# 1. Snapshot your current setup.
codex-switch import work --activate

# 2. Manually edit ~/.codex/auth.json and ~/.codex/config.toml for another setup
#    (switch provider, swap the API key, etc.), then snapshot that too.
codex-switch import personal

# 3. Flip between them anytime.
codex-switch use work
codex-switch use personal

# See what you have; the active one is starred.
codex-switch list
# * personal
#   work

codex-switch current
# personal
```

### Capturing drift

Because this is a pure-snapshot tool, changes you make to the live files after
switching are not tracked automatically. To fold them back into the active
profile:

```sh
codex-switch diff          # what changed vs. the active profile?
# auth.json:   same
# config.toml: differs
codex-switch save          # write live back into the active profile
```

`save <name>` targets a specific profile instead of the active one.

### Recovering from a bad switch

The live config from just before your last `use` is in `<store>/backup/`. Find
the store with `codex-switch paths`, then copy the files back:

```sh
codex-switch paths        # note "store root"
cp <store>/backup/auth.json   ~/.codex/auth.json
cp <store>/backup/config.toml ~/.codex/config.toml
```

## Notes and edge cases

- **Config-only profiles.** If a profile has a `config.toml` but no `auth.json`
  (e.g. a third-party provider that authenticates via `config.toml`), switching
  to it will *remove* any stale live `auth.json` so the live state matches the
  snapshot exactly. Switching back to a profile that has `auth.json` restores it.
- **Profile names** map to directory names: no `/`, `\`, `..`, `.`, or NUL.
- **Removing the active profile** requires `--force`; afterwards there is no
  active profile (the live files are left untouched).
- This tool never contacts the network and never edits the *contents* of your
  config — it only copies whole files around.

## Environment variables

| Variable            | Purpose                                             |
| ------------------- | --------------------------------------------------- |
| `CODEX_HOME`        | Location of the live Codex config (default `~/.codex`). |
| `CODEX_SWITCH_HOME` | Force the store location.                            |
| `XDG_DATA_HOME`     | Base for the XDG fallback store (default `~/.local/share`). |

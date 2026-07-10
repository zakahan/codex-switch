# AGENTS.md

Guidance for AI agents and contributors working on this repository.

## What this is

`codex-switch` is a single-binary Linux CLI that snapshots and switches
[Codex](https://github.com/openai/codex) credentials. Each **profile** is a
directory holding a complete copy of `~/.codex/auth.json` and
`~/.codex/config.toml`; switching atomically replaces the live files with a
profile's copy. It is a **pure snapshot** tool — it copies whole files, never
edits their contents, and never touches the network.

Scope is deliberately small: store + switch. Provider setup is done by hand.
Do not add provider management, config templating, TOML/JSON content merging,
MCP/proxy features, or a GUI. The reference app under `ref/cc-switch/` has all
of that; we intentionally left it out.

## Layout

```
src/
  main.rs      # clap CLI: list, current, use, save, import, diff, rm, paths
  paths.rs     # resolve live files (CODEX_HOME) and the store (with fallback)
  profile.rs   # snapshot read/write, paired write + rollback, backup, remove
  state.rs     # state.json (active profile) + profile-name validation
  atomic.rs    # atomic_write: temp file in same dir -> rename, perms handling
ref/cc-switch/ # read-only reference (the Tauri app this is inspired by). Do not edit.
```

## Core invariants — do not break these

- **Atomic writes.** All file writes go through `atomic::atomic_write` (temp file
  in the *same directory*, then `rename`). Never write a target path in place.
  Same-directory temp keeps the rename on one filesystem and symlink-safe.
- **Paired write with rollback.** `auth.json` is written before `config.toml`;
  if `config.toml` fails, `auth.json` (and `config.toml`) are restored to their
  pre-write bytes. See `profile::write_pair`. Keep this ordering and rollback.
- **`auth.json` is `0600`.** It holds credentials. `AUTH_MODE` enforces this on
  every write, including profiles and backups.
- **`None` means "remove".** In a `Snapshot`, a `None` field means the file is
  absent; applying it deletes the corresponding target so live matches the
  snapshot exactly (e.g. config-only profiles clear a stale live `auth.json`).
- **Pure snapshot.** Never auto-write live changes back into a profile. Drift is
  captured only by an explicit `save`.
- **Backup before `use`.** Every `use` first copies live to `<store>/backup/`.
- **Name validation.** Profile names are directory names. Reject empty, `.`,
  `..`, and anything containing `/`, `\`, or NUL (`state::validate_profile_name`).

## Path resolution

- Live config: `CODEX_HOME` if set/non-empty, else `~/.codex`.
- Store: `CODEX_SWITCH_HOME` -> existing store -> `~/.codex-switch` (if home
  writable) -> `$XDG_DATA_HOME/codex-switch` fallback. The fallback exists for
  read-only home mounts (e.g. `~` symlinked into a read-only filesystem).
- `~` may be a symlink; everything must keep working through it. Do not call
  `canonicalize` on target paths in a way that would defeat writing through a
  symlink.

## Build, run, test

```sh
cargo build                 # debug
cargo build --release       # optimized single binary
cargo install --path .      # install to ~/.cargo/bin/codex-switch
```

There is no unit-test suite yet. Verify changes end-to-end against an **isolated
sandbox** so you never clobber the real `~/.codex`:

```sh
export CODEX_HOME=/tmp/cs-test/.codex
export CODEX_SWITCH_HOME=/tmp/cs-test/.codex-switch
rm -rf /tmp/cs-test && mkdir -p "$CODEX_HOME"
printf '{"OPENAI_API_KEY":"sk-a"}\n' > "$CODEX_HOME/auth.json"
printf 'model="gpt-5"\n'            > "$CODEX_HOME/config.toml"

codex-switch import a --activate
codex-switch use a
codex-switch diff
codex-switch save
codex-switch list
```

Always test with `CODEX_HOME`/`CODEX_SWITCH_HOME` pointed at a temp dir. Never
run mutating commands against the real store while developing.

## Style

- Keep it dependency-light: `clap`, `serde`, `serde_json`, `dirs`, `anyhow`.
- Errors use `anyhow` with `.context(...)`; user-facing messages are printed by
  `main` and the process exits non-zero on error.
- Comments explain *why*, not *what*. The build must stay warning-free.

## Local development note

If your `$HOME` is on a read-only mount, `rustup` and the store's default
location won't be writable there. Point `CARGO_HOME`/`RUSTUP_HOME` at a writable
directory for the toolchain, and use `CODEX_SWITCH_HOME` (and `CODEX_HOME`) to
redirect the store and live files when testing.

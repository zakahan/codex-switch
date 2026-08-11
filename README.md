# codex-switch

A tiny Linux CLI to switch [Codex](https://github.com/openai/codex)
**model / provider / API-key identities** without disturbing the rest of your
live config.

Most Codex settings are personal/environment preferences — sandbox mode,
approval policy, which directories you trust, MCP servers — and you do *not*
want those to change when you switch between, say, your OpenAI account and a
third-party provider. `codex-switch` stores only the pieces that describe a
provider identity and merges them over your live `config.toml`, leaving
everything else (including comments and formatting) untouched.

It is inspired by [cc-switch](https://github.com/farion1231/cc-switch) (a Tauri
desktop app), but is Codex-only, CLI-only, single-binary, and has no runtime
dependencies. Provider setup is done by hand — this tool only captures and
applies provider layers.

## What a profile owns

A profile is a directory under `<store>/profiles/<name>/` containing:

- `auth.json` (optional) — OpenAI/ChatGPT credentials, always written `0600`.
- `provider.toml` — a TOML fragment of the managed top-level keys:
  `model`, `model_provider`, and the active `[model_providers.<id>]` table.
- `model-catalog.json` (optional) — a model catalog installed into
  `$CODEX_HOME/catalogs/<name>.json`; `model_catalog_json` is pointed at it on
  apply.

Everything else in `~/.codex/config.toml` — `sandbox_mode`, `approval_policy`,
`[projects]` trust decisions, `[permissions]`, `mcp_servers`, ... — is **not**
stored and is left in place when you switch.

```
~/.codex/                      # the LIVE config that Codex actually reads
  auth.json
  config.toml
  catalogs/<name>.json         # per-profile model catalogs installed here

<store>/                       # codex-switch's storage
  profiles/
    work/
      auth.json
      provider.toml
    personal/
      auth.json
      provider.toml
      model-catalog.json
  backup/                      # last live auth+config, saved before each `use`
  state.json                   # { "active": "<profile>" }
```

> v0.1.0 (tag `v0.1.0`) was a whole-file snapshot tool. v0.2.0 replaced that
> with field-level layers; snapshot mode was removed.

## Why

- **cc-switch has no Linux CLI.** This fills that gap for Codex.
- You keep several Codex provider identities (official ChatGPT login, a
  third-party provider, a work key, a personal key) and want to flip the model /
  provider / API key / catalog between them while keeping your sandbox,
  approval, and trusted-directory settings constant.

## How it works

- `import`/`save` read the live config and **extract** the managed keys plus the
  active `[model_providers.<id>]` table. If `model_catalog_json` points at a
  readable file, that file is bundled into the profile and the path key is
  dropped (it is re-pointed at the installed copy on `use`).
- `use` parses the live `config.toml` with [`toml_edit`](https://crates.io/crates/toml_edit),
  merges the profile fragment over it (scalars replaced, `model_providers`
  merged per provider id so unrelated custom providers survive), installs the
  catalog, and writes it back atomically. `auth.json` is written first; if the
  config write fails, auth, config, and the catalog are rolled back.

### Safety properties

- **Atomic writes.** Each file is written to a temp file in the same directory
  and then `rename`d over the target, so Codex never sees a half-written file.
- **Paired write with rollback.** `auth.json` is written first, then
  `config.toml`; if the second write fails, both (and a freshly installed
  catalog) are restored to their pre-write bytes.
- **Least-privilege credentials.** `auth.json` is always written with `0600`.
- **Automatic backup.** Before every `use`, the current live auth + config are
  copied to `<store>/backup/`.
- **Symlink-safe.** Works when `$HOME` or `~/.codex` is a symlink.
- **No network.** This tool never contacts the network.

## Where things live

**Live files** — from `CODEX_HOME` if set, otherwise `~/.codex`:

- `$CODEX_HOME/auth.json`
- `$CODEX_HOME/config.toml`
- `$CODEX_HOME/catalogs/` (installed per-profile catalogs)

**The store** is resolved in this order:

1. `$CODEX_SWITCH_HOME`, if set.
2. An existing `~/.codex-switch` or `$XDG_DATA_HOME/codex-switch`.
3. `~/.codex-switch`, if home is writable.
4. `$XDG_DATA_HOME/codex-switch` (default `~/.local/share/codex-switch`) as a
   fallback for read-only homes.

Run `codex-switch paths` to see exactly what is used and why.

## Install

Requires a Rust toolchain. From the repo root:

```sh
cargo install --path .
```

This installs a `codex-switch` binary into `~/.cargo/bin` (already on `PATH`
with rustup). Re-run after pulling to update.

## Usage

```
codex-switch <command>

  import <name> [--activate] [--force]   Capture live provider settings as a new profile
  list (ls)                              List profiles; the active one is marked with *
  current                                Print the active profile name
  use <name>                             Merge a profile's layer over the live config
  save [name]                            Capture live provider settings into a profile
                                         (defaults to the active profile)
  diff [name]                            Compare a profile against the live config
  rm (remove) <name> [--force]           Delete a profile (and its installed catalog)
  paths                                  Print resolved paths and the store location
```

### Typical workflow

```sh
# 1. Capture your current provider setup.
codex-switch import work --activate

# 2. Edit ~/.codex/config.toml + auth.json for another provider (model, base_url,
#    API key, optional model_catalog_json), then capture that too.
codex-switch import personal

# 3. Flip between them. Sandbox/approval/trusted-project settings are untouched.
codex-switch use work
codex-switch use personal

codex-switch list
# * personal
#   work
```

### What gets switched vs. kept

Switched (stored per profile): `model`, `model_provider`, the active
`[model_providers.<id>]` entry, `model_catalog_json`/catalog, and `auth.json`.

Kept in live config (never overwritten): `sandbox_mode`, `approval_policy`,
`default_permissions`, `[permissions]`, `[projects]` trust entries,
`mcp_servers`, and all other unmanaged keys.

### Capturing drift

Changes to the live provider settings after switching are not tracked
automatically. Fold them back with:

```sh
codex-switch diff          # what changed vs. the active profile?
codex-switch save          # capture live provider settings into the active profile
```

### Recovering from a bad switch

The live auth + config from just before the last `use` are in
`<store>/backup/`:

```sh
codex-switch paths
cp <store>/backup/auth.json   ~/.codex/auth.json
cp <store>/backup/config.toml ~/.codex/config.toml
```

## Notes and edge cases

- **Auth-less profiles.** A profile without `auth.json` removes any stale live
  `auth.json` on `use`. Switching back to a profile that has one restores it.
- **Custom provider API keys** are normally supplied via the provider's
  `env_key` environment variable, not `auth.json`; `codex-switch` does not
  manage environment variables.
- **Catalogs.** A bundled catalog is installed to `$CODEX_HOME/catalogs/<name>.json`
  and `model_catalog_json` is set to that absolute path. If a profile has no
  catalog, the live `model_catalog_json` is left as-is.
- **Profile names** map to directory names: no `/`, `\`, `..`, `.`, or NUL.
- **Removing the active profile** requires `--force`.

## Environment variables

| Variable            | Purpose                                                  |
| ------------------- | -------------------------------------------------------- |
| `CODEX_HOME`        | Location of the live Codex config (default `~/.codex`).   |
| `CODEX_SWITCH_HOME` | Force the store location.                                |
| `XDG_DATA_HOME`     | Base for the XDG fallback store (default `~/.local/share`). |

# AGENTS.md

Guidance for AI agents and contributors working on this repository.

## What this is

`codex-switch` is a single-binary Linux CLI that switches
[Codex](https://github.com/openai/codex) **model/provider/API-key identities**
without disturbing the rest of the live config. Each **profile** is a *layer*
owning only:

- `auth.json` (optional, OpenAI/ChatGPT credentials),
- `provider.toml` — a TOML fragment of the managed top-level keys
  (`model`, `model_provider`, `model_catalog_json`) plus the active
  `[model_providers.<id>]` table,
- `model-catalog.json` (optional), installed into `$CODEX_HOME/catalogs/`
  and pointed at by `model_catalog_json` on apply.

Switching **merges** the fragment into the current live `config.toml` with
`toml_edit`, so settings the profile does not own — sandbox mode, approval
policy, trusted projects under `[projects]`, MCP servers, etc. — are left
exactly as they are (comments and formatting preserved). The tool never touches
the network.

Scope is deliberately small: capture + apply a provider layer. Provider setup
is done by hand. Do not add provider management UI, config templating beyond
the documented managed keys, MCP/proxy features, or a GUI. The reference app
under `ref/cc-switch/` has all of that; we intentionally left it out.

> Historical note: v0.1.0 was a whole-file snapshot tool (tag `v0.1.0`). v0.2.0
> replaced snapshots with field-level layers; snapshot mode was intentionally
> removed.

## Layout

```
src/
  main.rs      # clap CLI: list, current, use, save, import, diff, rm, paths
  paths.rs     # resolve live files (CODEX_HOME), catalogs dir, store (fallback)
  profile.rs   # layer capture/merge (toml_edit), catalog install, rollback, remove
  state.rs     # state.json (active profile) + profile-name validation
  atomic.rs    # atomic_write: temp file in same dir -> rename, perms handling
ref/cc-switch/ # read-only reference (the Tauri app this is inspired by). Do not edit.
```

## Core invariants — do not break these

- **Atomic writes.** All file writes go through `atomic::atomic_write` (temp file
  in the *same directory*, then `rename`). Never write a target path in place.
  Same-directory temp keeps the rename on one filesystem and symlink-safe.
- **Layer, not snapshot.** A profile only touches the managed keys
  (`model`, `model_provider`, `model_catalog_json`, and its own
  `[model_providers.<id>]` entry). `merge_fragment` must never rewrite or
  reorder unmanaged sections; it edits the live `DocumentMut` in place and
  leaves everything else (sandbox, approval, `[projects]` trust, MCP) intact.
  The `model_providers` map is merged per provider id so unrelated custom
  providers in live config are preserved.
- **Catalog follows the profile.** A bundled `model-catalog.json` is installed
  to `paths::catalog_path(name)` and `model_catalog_json` is set to that
  absolute path on apply. Removing a profile also removes its installed catalog.
- **Paired write with rollback.** `auth.json` is written before `config.toml`;
  if `config.toml` fails, `auth.json`, `config.toml`, and a freshly installed
  catalog are restored to their pre-write bytes. See `profile::write_live`.
- **`auth.json` is `0600`.** It holds credentials. `AUTH_MODE` enforces this on
  every write, including profiles and backups.
- **`None` means "remove".** A `None`/absent optional file in a layer means the
  corresponding target file is removed (so an auth-less profile clears a stale
  live `auth.json`). Catalog and `provider.toml` follow the same rule on write.
- **No auto-save.** Never auto-write live changes back into a profile. Drift is
  captured only by an explicit `save`/`import`.
- **Backup before `use`.** Every `use` first copies live auth+config to
  `<store>/backup/` (a full snapshot for manual recovery).
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
T=$(mktemp -d)
export CODEX_HOME="$T/.codex"
export CODEX_SWITCH_HOME="$T/.codex-switch"
mkdir -p "$CODEX_HOME"
printf '{"OPENAI_API_KEY":"sk-a"}\n' > "$CODEX_HOME/auth.json"
printf 'model="gpt-5"\nsandbox_mode="read-only"\n' > "$CODEX_HOME/config.toml"

codex-switch import a
codex-switch use a
codex-switch diff
codex-switch save
codex-switch list
```

When testing, confirm the layer promise: after `use`, unmanaged keys such as
`sandbox_mode`, `approval_policy`, and `[projects.*]` trust entries that were
in the live config are still present unchanged.

Always test with `CODEX_HOME`/`CODEX_SWITCH_HOME` pointed at a temp dir. Never
run mutating commands against the real store while developing.

## Style

- Keep it dependency-light: `clap`, `serde`, `serde_json`, `dirs`, `anyhow`,
  `toml_edit`. `toml_edit` (not `toml`) is required so merges preserve
  formatting and comments.
- Errors use `anyhow` with `.context(...)`; user-facing messages are printed by
  `main` and the process exits non-zero on error.
- Comments explain *why*, not *what*. The build must stay warning-free.

## Local development note

If your `$HOME` is on a read-only mount, `rustup` and the store's default
location won't be writable there. Point `CARGO_HOME`/`RUSTUP_HOME` at a writable
directory for the toolchain, and use `CODEX_SWITCH_HOME` (and `CODEX_HOME`) to
redirect the store and live files when testing.

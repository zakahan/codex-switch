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
- `config.provider.toml` — a TOML fragment of the managed top-level keys:
  `model`, `model_provider`, `review_model`, and the active
  `[model_providers.<id>]` table.
- `model-catalog.json` (optional) — a model catalog. Treated the same way as
  `auth.json`: on `use` it is written to the fixed live path
  `~/.codex/model-catalog.json` (or removed there if the target profile has
  no catalog), and `model_catalog_json` in the live config is set / cleared
  accordingly.

Everything else in `~/.codex/config.toml` — `sandbox_mode`, `approval_policy`,
`[projects]` trust decisions, `[permissions]`, `mcp_servers`, ... — is **not**
stored and is left in place when you switch.

```
~/.codex/                      # the LIVE config that Codex actually reads
  auth.json
  config.toml                  # model_catalog_json points at ./model-catalog.json
  model-catalog.json           # written/removed by `use`, same treatment as auth.json

<store>/                       # codex-switch's storage — one dir per profile
  profiles/
    work/
      auth.json
      config.provider.toml
    personal/
      auth.json
      config.provider.toml
      model-catalog.json
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
  active `[model_providers.<id>]` table. `auth.json` and `model-catalog.json`
  are captured from their fixed live paths (`~/.codex/auth.json`,
  `~/.codex/model-catalog.json`).
- `use` parses the live `config.toml` with [`toml_edit`](https://crates.io/crates/toml_edit),
  merges the profile fragment over it (scalars replaced, `model_providers`
  merged per provider id so unrelated custom providers survive), and writes
  the result back atomically. `auth.json` and `model-catalog.json` are
  overwritten (or removed) at their fixed live paths, and `model_catalog_json`
  in `config.toml` is synthesized / cleared accordingly. Live writes happen
  in order: auth, catalog, config. If any write fails all three are rolled
  back to their pre-write bytes. After a successful switch, the CLI reminds
  you to run `codex app-server daemon restart` so a running Codex daemon
  reloads the new profile.

### Safety properties

- **Atomic writes.** Each file is written to a temp file in the same directory
  and then `rename`d over the target, so Codex never sees a half-written file.
- **Paired write with rollback.** `auth.json` is written first, then
  `~/.codex/model-catalog.json`, then `config.toml`; if any write fails all
  three are restored to their pre-write bytes.
- **Least-privilege credentials.** `auth.json` is always written with `0600`.
- **Symlink-safe.** Works when `$HOME` or `~/.codex` is a symlink.
- **No network.** This tool never contacts the network.

## Where things live

**Live files** — from `CODEX_HOME` if set, otherwise `~/.codex`:

- `$CODEX_HOME/auth.json`
- `$CODEX_HOME/config.toml`
- `$CODEX_HOME/model-catalog.json` (present only when the active profile
  bundles one)

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

## Creating a profile (manual)

`codex-switch` intentionally has **no provider setup UI, no templates, no
wizards** — its job is capture + apply. So the first profile has to be written
by hand. There are two paths; pick one.

### Path A — write the profile directory directly

Create `<store>/profiles/<name>/config.provider.toml` with the managed keys
for your provider. Minimal example for a DeepSeek-style OpenAI-compatible API:

```toml
# ~/.codex-switch/profiles/deepseek/config.provider.toml
model = "deepseek-chat"
model_provider = "deepseek"
review_model = "deepseek-chat"

[model_providers.deepseek]
name = "DeepSeek"
base_url = "https://api.deepseek.com"
env_key = "DEEPSEEK_API_KEY"
wire_api = "chat"
```

Then activate it:

```sh
codex-switch use deepseek
```

Notes:

- The provider id (`deepseek` in `[model_providers.deepseek]`) **must match**
  `model_provider`.
- `base_url` should not have a trailing slash.
- `wire_api` is `"chat"` for OpenAI-compatible `/chat/completions` endpoints,
  or `"responses"` for OpenAI's Responses API.
- The API key itself is **not** stored in the profile. Export it as the
  environment variable named by `env_key` (e.g. `export DEEPSEEK_API_KEY=sk-...`).
- Only include `auth.json` for OpenAI/ChatGPT-style logins that Codex reads
  from `auth.json`. Third-party providers that use `env_key` do not need one.
- Do **not** set `model_catalog_json` yourself — `codex-switch` manages it. If
  you want to ship a catalog, drop the file at
  `<store>/profiles/<name>/model-catalog.json`. On `use` its bytes are copied
  to `~/.codex/model-catalog.json` and `model_catalog_json` in the live config
  is pointed at that fixed path.

### Path B — configure live first, then import

If you'd rather edit `~/.codex/config.toml` in place (adding `model`,
`model_provider`, and the `[model_providers.<id>]` table) and then snapshot
it:

```sh
codex-switch import deepseek --activate
```

`import` extracts only the managed keys plus the active provider table into a
new profile.

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
  rm (remove) <name> [--force]           Delete a profile
  paths                                  Print resolved paths and the store location
```

### Typical workflow

```sh
# 1. Capture your current provider setup.
codex-switch import work --activate

# 2. Edit ~/.codex/config.toml + auth.json for another provider (model, base_url,
#    API key, optionally drop a model-catalog.json in place), then capture that
#    too.
codex-switch import personal

# 3. Flip between them. Sandbox/approval/trusted-project settings are untouched.
codex-switch use work
codex-switch use personal

codex-switch list
# * personal
#   work
```

### What gets switched vs. kept

Switched (stored per profile): `model`, `model_provider`, `review_model`, the
active `[model_providers.<id>]` entry, `auth.json`, and `model-catalog.json`
(the last two are captured from and written to their fixed live paths).
`model_catalog_json` in `config.toml` is synthesized on `use` and is not
stored in the profile.

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

## Notes and edge cases

- **Auth-less profiles.** A profile without `auth.json` removes any stale live
  `auth.json` on `use`. Switching back to a profile that has one restores it.
- **Custom provider API keys** are normally supplied via the provider's
  `env_key` environment variable, not `auth.json`; `codex-switch` does not
  manage environment variables.
- **Catalogs.** When the active profile bundles a `model-catalog.json`, its
  bytes are written to `~/.codex/model-catalog.json` and live
  `model_catalog_json` points at that fixed path. Switching to a profile that
  has no catalog removes `~/.codex/model-catalog.json` and drops
  `model_catalog_json` from `config.toml`, exactly the way `auth.json` is
  handled.
- **Profile names** map to directory names: no `/`, `\`, `..`, `.`, or NUL.
- **Removing the active profile** requires `--force`.

## Environment variables

| Variable            | Purpose                                                  |
| ------------------- | -------------------------------------------------------- |
| `CODEX_HOME`        | Location of the live Codex config (default `~/.codex`).   |
| `CODEX_SWITCH_HOME` | Force the store location.                                |
| `XDG_DATA_HOME`     | Base for the XDG fallback store (default `~/.local/share`). |

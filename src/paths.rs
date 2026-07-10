use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Codex config directory: `CODEX_HOME` when set and non-empty, else `~/.codex`.
pub fn codex_home() -> Result<PathBuf> {
    if let Some(dir) = env_nonempty("CODEX_HOME") {
        return Ok(dir);
    }
    Ok(home_dir()?.join(".codex"))
}

/// Live `~/.codex/auth.json`.
pub fn codex_auth_path() -> Result<PathBuf> {
    Ok(codex_home()?.join("auth.json"))
}

/// Live `~/.codex/config.toml`.
pub fn codex_config_path() -> Result<PathBuf> {
    Ok(codex_home()?.join("config.toml"))
}

/// Where the codex-switch store lives, and why (for diagnostics).
pub enum StoreSource {
    /// `CODEX_SWITCH_HOME` was set.
    Env,
    /// An existing store directory was reused.
    Existing,
    /// Default `~/.codex-switch` (home is writable).
    Default,
    /// XDG data fallback because home is not writable.
    XdgFallback,
}

/// codex-switch store root, with the reason it was chosen.
///
/// Resolution order:
/// 1. `CODEX_SWITCH_HOME` when set and non-empty.
/// 2. An already-existing `~/.codex-switch` or `$XDG_DATA_HOME/codex-switch`.
/// 3. `~/.codex-switch` when home is writable.
/// 4. `$XDG_DATA_HOME/codex-switch` (defaults to `~/.local/share/codex-switch`)
///    as a fallback for read-only homes (e.g. symlinked read-only mounts).
pub fn store_root_with_source() -> Result<(PathBuf, StoreSource)> {
    if let Some(dir) = env_nonempty("CODEX_SWITCH_HOME") {
        return Ok((dir, StoreSource::Env));
    }
    let home = home_dir()?;
    let primary = home.join(".codex-switch");
    let fallback = xdg_data_home(&home).join("codex-switch");

    if primary.exists() {
        return Ok((primary, StoreSource::Existing));
    }
    if fallback.exists() {
        return Ok((fallback, StoreSource::Existing));
    }
    if creatable(&primary) {
        Ok((primary, StoreSource::Default))
    } else if creatable(&fallback) {
        Ok((fallback, StoreSource::XdgFallback))
    } else {
        // Nothing is writable; return the default and let the write surface a
        // clear OS error rather than guessing further.
        Ok((primary, StoreSource::Default))
    }
}

pub fn store_root() -> Result<PathBuf> {
    Ok(store_root_with_source()?.0)
}

pub fn profiles_dir() -> Result<PathBuf> {
    Ok(store_root()?.join("profiles"))
}

pub fn profile_dir(name: &str) -> Result<PathBuf> {
    Ok(profiles_dir()?.join(name))
}

pub fn state_path() -> Result<PathBuf> {
    Ok(store_root()?.join("state.json"))
}

/// Directory holding the pre-switch backup of the live files.
pub fn backup_dir() -> Result<PathBuf> {
    Ok(store_root()?.join("backup"))
}

fn home_dir() -> Result<PathBuf> {
    dirs::home_dir().context("could not determine home directory")
}

fn xdg_data_home(home: &Path) -> PathBuf {
    env_nonempty("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local").join("share"))
}

fn env_nonempty(key: &str) -> Option<PathBuf> {
    let val = std::env::var_os(key)?;
    if val.to_string_lossy().trim().is_empty() {
        None
    } else {
        Some(PathBuf::from(val))
    }
}

/// Can we create `dir`? Walk up to the nearest existing ancestor and test that
/// we can create an entry inside it.
fn creatable(dir: &Path) -> bool {
    let mut cur = dir;
    loop {
        if cur.exists() {
            return probe_writable(cur);
        }
        match cur.parent() {
            Some(parent) => cur = parent,
            None => return false,
        }
    }
}

fn probe_writable(dir: &Path) -> bool {
    let probe = dir.join(format!(".codex-switch-probe-{}", std::process::id()));
    match fs::create_dir(&probe) {
        Ok(()) => {
            let _ = fs::remove_dir(&probe);
            true
        }
        Err(_) => false,
    }
}

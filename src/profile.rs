use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

use crate::atomic::atomic_write;
use crate::paths;
use crate::state;

/// auth.json holds credentials, so keep it owner-only.
#[cfg(unix)]
const AUTH_MODE: Option<u32> = Some(0o600);
#[cfg(not(unix))]
const AUTH_MODE: Option<u32> = None;

/// A snapshot of the two Codex files. `None` means the file was absent.
pub struct Snapshot {
    pub auth: Option<Vec<u8>>,
    pub config: Option<Vec<u8>>,
}

impl Snapshot {
    pub fn is_empty(&self) -> bool {
        self.auth.is_none() && self.config.is_none()
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).with_context(|| format!("failed to read: {}", path.display()))?;
    Ok(Some(bytes))
}

/// Read the current live `~/.codex/{auth.json,config.toml}`.
pub fn read_live() -> Result<Snapshot> {
    Ok(Snapshot {
        auth: read_optional(&paths::codex_auth_path()?)?,
        config: read_optional(&paths::codex_config_path()?)?,
    })
}

/// Read a stored profile's snapshot.
pub fn read_profile(name: &str) -> Result<Snapshot> {
    let dir = paths::profile_dir(name)?;
    Ok(Snapshot {
        auth: read_optional(&dir.join("auth.json"))?,
        config: read_optional(&dir.join("config.toml"))?,
    })
}

pub fn profile_exists(name: &str) -> Result<bool> {
    Ok(paths::profile_dir(name)?.is_dir())
}

/// List profile names (directories under profiles/), sorted.
pub fn list_profiles() -> Result<Vec<String>> {
    let dir = paths::profiles_dir()?;
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut names = Vec::new();
    for entry in fs::read_dir(&dir)
        .with_context(|| format!("failed to read profiles dir: {}", dir.display()))?
    {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            if let Some(name) = entry.file_name().to_str() {
                names.push(name.to_string());
            }
        }
    }
    names.sort();
    Ok(names)
}

/// Write a snapshot into a profile directory (used by save/import).
pub fn write_profile(name: &str, snap: &Snapshot) -> Result<()> {
    let dir = paths::profile_dir(name)?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create profile dir: {}", dir.display()))?;
    write_pair(&dir.join("auth.json"), &dir.join("config.toml"), snap)
}

/// Apply a snapshot to the live Codex files with rollback: capture the current
/// live bytes first, write auth.json then config.toml, and if the second write
/// fails restore both to their pre-write state.
pub fn write_live(snap: &Snapshot) -> Result<()> {
    let auth_path = paths::codex_auth_path()?;
    let config_path = paths::codex_config_path()?;
    write_pair(&auth_path, &config_path, snap)
}

/// Core paired write: auth first, then config; rollback auth on config failure.
/// A `None` field means "remove that file so the target matches the snapshot".
///
/// If the config write fails, we attempt to restore both files to their
/// pre-write bytes. A rollback that *also* fails is serious — the live config
/// is now in an unknown, mismatched state — so those errors are surfaced
/// alongside the original failure rather than swallowed.
fn write_pair(auth_path: &Path, config_path: &Path, snap: &Snapshot) -> Result<()> {
    let old_auth = read_optional(auth_path)?;
    let old_config = read_optional(config_path)?;

    // Step 1: auth.json
    apply_file(auth_path, snap.auth.as_deref(), AUTH_MODE)
        .with_context(|| format!("failed to write {}", auth_path.display()))?;

    // Step 2: config.toml — on failure, roll back both files.
    if let Err(write_err) = apply_file(config_path, snap.config.as_deref(), None) {
        let write_err = write_err.context(format!("failed to write {}", config_path.display()));

        let mut rollback_errs = Vec::new();
        if let Err(e) = restore_file(auth_path, old_auth.as_deref(), AUTH_MODE) {
            rollback_errs.push(format!("could not restore {}: {e:#}", auth_path.display()));
        }
        if let Err(e) = restore_file(config_path, old_config.as_deref(), None) {
            rollback_errs.push(format!(
                "could not restore {}: {e:#}",
                config_path.display()
            ));
        }

        if rollback_errs.is_empty() {
            return Err(write_err);
        }
        return Err(write_err.context(format!(
            "ROLLBACK FAILED — live config may be inconsistent: {}",
            rollback_errs.join("; ")
        )));
    }
    Ok(())
}

/// Write `content` to `path`, or remove `path` when `content` is `None`.
fn apply_file(path: &Path, content: Option<&[u8]>, mode: Option<u32>) -> Result<()> {
    match content {
        Some(bytes) => atomic_write(path, bytes, mode),
        None => {
            if path.exists() {
                fs::remove_file(path)
                    .with_context(|| format!("failed to remove: {}", path.display()))?;
            }
            Ok(())
        }
    }
}

fn restore_file(path: &Path, content: Option<&[u8]>, mode: Option<u32>) -> Result<()> {
    apply_file(path, content, mode)
}

/// Copy the current live files into the backup directory before overwriting.
pub fn backup_live() -> Result<Option<PathBuf>> {
    let snap = read_live()?;
    if snap.is_empty() {
        return Ok(None);
    }
    let dir = paths::backup_dir()?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create backup dir: {}", dir.display()))?;
    write_pair(&dir.join("auth.json"), &dir.join("config.toml"), &snap)?;
    Ok(Some(dir))
}

/// Remove a profile directory. Refuses if it is the active profile unless forced.
pub fn remove_profile(name: &str, force: bool) -> Result<()> {
    if !profile_exists(name)? {
        bail!("profile not found: {name}");
    }
    let active = state::load_state()?.active;
    if active.as_deref() == Some(name) && !force {
        bail!("{name:?} is the active profile; pass --force to remove it anyway");
    }
    let dir = paths::profile_dir(name)?;
    fs::remove_dir_all(&dir)
        .with_context(|| format!("failed to remove profile dir: {}", dir.display()))?;
    if active.as_deref() == Some(name) {
        state::set_active(None)?;
    }
    Ok(())
}

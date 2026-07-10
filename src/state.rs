use std::fs;

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

use crate::atomic::atomic_write;
use crate::paths;

/// Persisted pointer to the currently active profile.
#[derive(Debug, Default, Serialize, Deserialize)]
pub struct State {
    /// Name of the profile last applied to the live Codex config, if any.
    #[serde(default)]
    pub active: Option<String>,
}

pub fn load_state() -> Result<State> {
    let path = paths::state_path()?;
    if !path.exists() {
        return Ok(State::default());
    }
    let text = fs::read_to_string(&path)
        .with_context(|| format!("failed to read state: {}", path.display()))?;
    if text.trim().is_empty() {
        return Ok(State::default());
    }
    serde_json::from_str(&text)
        .with_context(|| format!("failed to parse state: {}", path.display()))
}

pub fn save_state(state: &State) -> Result<()> {
    let path = paths::state_path()?;
    let json = serde_json::to_string_pretty(state)?;
    atomic_write(&path, json.as_bytes(), None)
}

pub fn set_active(name: Option<&str>) -> Result<()> {
    let mut state = load_state()?;
    state.active = name.map(str::to_string);
    save_state(&state)
}

/// Reject names that would escape the profiles directory or collide with
/// filesystem specials. Profiles map 1:1 to a directory under `profiles/`.
pub fn validate_profile_name(name: &str) -> Result<()> {
    if name.is_empty() {
        bail!("profile name must not be empty");
    }
    if name == "." || name == ".." {
        bail!("invalid profile name: {name:?}");
    }
    if name.contains('/') || name.contains('\\') || name.contains('\0') {
        bail!("profile name must not contain path separators: {name:?}");
    }
    Ok(())
}

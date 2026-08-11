use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use toml_edit::{value, DocumentMut, Item};

use crate::atomic::atomic_write;
use crate::paths;
use crate::state;

/// auth.json holds credentials, so keep it owner-only.
#[cfg(unix)]
const AUTH_MODE: Option<u32> = Some(0o600);
#[cfg(not(unix))]
const AUTH_MODE: Option<u32> = None;

/// File names inside a profile directory.
const AUTH_FILE: &str = "auth.json";
const FRAGMENT_FILE: &str = "provider.toml";
const CATALOG_FILE: &str = "model-catalog.json";

/// A profile is a *layer*, not a snapshot. It carries only the pieces that
/// describe a model/provider identity:
///
/// - `auth`:     optional `auth.json` bytes (OpenAI/ChatGPT credentials),
/// - `fragment`: a TOML fragment owning the top-level provider/model keys
///               (`model`, `model_provider`, and the matching
///               `[model_providers.<id>]` table),
/// - `catalog`:  optional model-catalog JSON copied into the live Codex home.
///
/// Everything else in the live `config.toml` (sandbox, approval policy,
/// trusted projects under `[projects]`, MCP servers, ...) is left untouched
/// when the layer is applied.
pub struct ProfileLayer {
    pub auth: Option<Vec<u8>>,
    pub fragment: Option<DocumentMut>,
    pub catalog: Option<Vec<u8>>,
}

impl ProfileLayer {
    pub fn is_empty(&self) -> bool {
        let fragment_empty = self
            .fragment
            .as_ref()
            .map(|d| d.iter().next().is_none())
            .unwrap_or(true);
        self.auth.is_none() && fragment_empty && self.catalog.is_none()
    }
}

fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(path).with_context(|| format!("failed to read: {}", path.display()))?;
    Ok(Some(bytes))
}

fn parse_config(bytes: &[u8], path: &Path) -> Result<DocumentMut> {
    let text = std::str::from_utf8(bytes)
        .with_context(|| format!("config is not valid UTF-8: {}", path.display()))?;
    text.parse::<DocumentMut>()
        .with_context(|| format!("failed to parse TOML: {}", path.display()))
}

/// Read and parse the live `~/.codex/config.toml` (empty doc if absent).
fn read_live_config() -> Result<DocumentMut> {
    let path = paths::codex_config_path()?;
    match read_optional(&path)? {
        Some(bytes) => parse_config(&bytes, &path),
        None => Ok(DocumentMut::new()),
    }
}

/// Top-level config keys that a profile is allowed to manage.
const MANAGED_KEYS: [&str; 3] = ["model", "model_provider", "model_catalog_json"];

/// Extract the managed subset of a config document into a standalone
/// fragment. The active provider's `[model_providers.<id>]` table is pulled in
/// alongside `model_provider` so the layer is self-contained.
fn extract_fragment(doc: &DocumentMut) -> DocumentMut {
    let mut out = DocumentMut::new();
    for key in MANAGED_KEYS {
        if let Some(item) = doc.get(key) {
            out.insert(key, item.clone());
        }
    }
    if let Some(provider_id) = doc.get("model_provider").and_then(|i| i.as_str()) {
        if let Some(provider) = doc
            .get("model_providers")
            .and_then(|t| t.as_table_like())
            .and_then(|t| t.get(provider_id))
        {
            let mut providers = toml_edit::Table::new();
            providers.set_implicit(true);
            providers.insert(provider_id, provider.clone());
            out["model_providers"] = Item::Table(providers);
        }
    }
    out
}

/// Merge a profile fragment into the live config document.
///
/// Scalar managed keys are replaced outright. The `model_providers` map is
/// merged per provider id so custom providers defined in the live config that
/// the profile does not mention are preserved.
fn merge_fragment(doc: &mut DocumentMut, fragment: &DocumentMut) {
    for (key, item) in fragment.iter() {
        if key == "model_providers" {
            if item.as_table_like().is_some() && !doc.contains_table("model_providers") {
                let mut providers = toml_edit::Table::new();
                providers.set_implicit(true);
                doc["model_providers"] = Item::Table(providers);
            }
            if let Some(providers) = item.as_table_like() {
                for (provider_id, provider_item) in providers.iter() {
                    doc["model_providers"][provider_id] = provider_item.clone();
                }
            }
        } else {
            doc.insert(key, item.clone());
        }
    }
}

/// Capture the live config as a layer. If `model_catalog_json` points at a
/// readable file, its bytes are bundled into the layer and the path key is
/// dropped from the fragment (it is re-pointed at the installed copy on use).
pub fn capture_live() -> Result<ProfileLayer> {
    let auth = read_optional(&paths::codex_auth_path()?)?;
    let mut fragment = extract_fragment(&read_live_config()?);

    let catalog = match fragment
        .get("model_catalog_json")
        .and_then(|i| i.as_str())
        .map(PathBuf::from)
    {
        Some(p) if p.is_file() => {
            let bytes =
                fs::read(&p).with_context(|| format!("failed to read catalog: {}", p.display()))?;
            fragment.remove("model_catalog_json");
            Some(bytes)
        }
        _ => None,
    };

    Ok(ProfileLayer {
        auth,
        fragment: Some(fragment),
        catalog,
    })
}

/// Read a stored profile's layer.
pub fn read_profile(name: &str) -> Result<ProfileLayer> {
    let dir = paths::profile_dir(name)?;
    let auth = read_optional(&dir.join(AUTH_FILE))?;
    let catalog = read_optional(&dir.join(CATALOG_FILE))?;
    let fragment = match read_optional(&dir.join(FRAGMENT_FILE))? {
        Some(bytes) => Some(parse_config(&bytes, &dir.join(FRAGMENT_FILE))?),
        None => None,
    };
    Ok(ProfileLayer {
        auth,
        fragment,
        catalog,
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

/// Write a single profile file, or remove it when `content` is `None` so an
/// overwrite does not leave stale files behind.
fn sync_file(path: &Path, content: Option<&[u8]>, mode: Option<u32>) -> Result<()> {
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

/// Persist a layer into a profile directory.
pub fn write_profile(name: &str, layer: &ProfileLayer) -> Result<()> {
    let dir = paths::profile_dir(name)?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create profile dir: {}", dir.display()))?;

    sync_file(&dir.join(AUTH_FILE), layer.auth.as_deref(), AUTH_MODE)?;

    let fragment_bytes = layer
        .fragment
        .as_ref()
        .filter(|d| d.iter().next().is_some())
        .map(|d| d.to_string().into_bytes());
    sync_file(&dir.join(FRAGMENT_FILE), fragment_bytes.as_deref(), None)?;

    sync_file(&dir.join(CATALOG_FILE), layer.catalog.as_deref(), None)?;
    Ok(())
}

fn restore_file(path: &Path, content: Option<&[u8]>, mode: Option<u32>) -> Result<()> {
    sync_file(path, content, mode)
}

/// Apply a layer to the live Codex config.
///
/// The fragment is merged into the *current* live `config.toml`, so settings
/// the profile does not own (sandbox, approval, trusted projects, ...) are
/// preserved. A bundled catalog is installed under `$CODEX_HOME/catalogs/`
/// and `model_catalog_json` is pointed at it.
///
/// auth.json is written before config.toml; if the config write fails both
/// files (and a freshly installed catalog) are rolled back to their pre-write
/// bytes.
pub fn write_live(name: &str, layer: &ProfileLayer) -> Result<()> {
    let auth_path = paths::codex_auth_path()?;
    let config_path = paths::codex_config_path()?;
    let catalog_path = paths::catalog_path(name)?;

    let old_auth = read_optional(&auth_path)?;
    let old_config = read_optional(&config_path)?;
    let old_catalog = read_optional(&catalog_path)?;

    // Build the merged config up front so a parse/IO failure here happens
    // before anything live is touched.
    let mut doc = match &old_config {
        Some(bytes) => parse_config(bytes, &config_path)?,
        None => DocumentMut::new(),
    };
    if let Some(fragment) = &layer.fragment {
        merge_fragment(&mut doc, fragment);
    }
    if let Some(catalog_bytes) = &layer.catalog {
        atomic_write(&catalog_path, catalog_bytes, None)
            .with_context(|| format!("failed to write catalog: {}", catalog_path.display()))?;
        doc["model_catalog_json"] = value(catalog_path.display().to_string());
    }
    let new_config = doc.to_string();

    let rollback = |err: anyhow::Error| -> Result<()> {
        let mut rollback_errs = Vec::new();
        if let Err(e) = restore_file(&auth_path, old_auth.as_deref(), AUTH_MODE) {
            rollback_errs.push(format!("could not restore {}: {e:#}", auth_path.display()));
        }
        if let Err(e) = restore_file(&config_path, old_config.as_deref(), None) {
            rollback_errs.push(format!(
                "could not restore {}: {e:#}",
                config_path.display()
            ));
        }
        if let Err(e) = restore_file(&catalog_path, old_catalog.as_deref(), None) {
            rollback_errs.push(format!(
                "could not restore {}: {e:#}",
                catalog_path.display()
            ));
        }
        if rollback_errs.is_empty() {
            Err(err)
        } else {
            Err(err.context(format!(
                "ROLLBACK FAILED — live config may be inconsistent: {}",
                rollback_errs.join("; ")
            )))
        }
    };

    if let Err(e) = sync_file(&auth_path, layer.auth.as_deref(), AUTH_MODE) {
        return rollback(e.context(format!("failed to write {}", auth_path.display())));
    }
    if let Err(e) = atomic_write(&config_path, new_config.as_bytes(), None) {
        return rollback(e.context(format!("failed to write {}", config_path.display())));
    }
    Ok(())
}

/// Copy the current live auth + config into the backup directory before
/// switching. The backup is a full snapshot so a user can manually recover.
pub fn backup_live() -> Result<Option<PathBuf>> {
    let auth = read_optional(&paths::codex_auth_path()?)?;
    let config = read_optional(&paths::codex_config_path()?)?;
    if auth.is_none() && config.is_none() {
        return Ok(None);
    }
    let dir = paths::backup_dir()?;
    fs::create_dir_all(&dir)
        .with_context(|| format!("failed to create backup dir: {}", dir.display()))?;
    sync_file(&dir.join(AUTH_FILE), auth.as_deref(), AUTH_MODE)?;
    sync_file(&dir.join("config.toml"), config.as_deref(), None)?;
    Ok(Some(dir))
}

/// Remove a profile directory and its installed live catalog. Refuses if it is
/// the active profile unless forced.
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
    let catalog = paths::catalog_path(name)?;
    if catalog.exists() {
        let _ = fs::remove_file(&catalog);
    }
    if active.as_deref() == Some(name) {
        state::set_active(None)?;
    }
    Ok(())
}

/// Result of diffing a profile against the live config.
pub struct DiffResult {
    pub auth: FileStatus,
    pub catalog: FileStatus,
    pub fragment: FileStatus,
}

pub enum FileStatus {
    Same,
    Differs,
    OnlyProfile,
    OnlyLive,
    Neither,
}

impl std::fmt::Display for FileStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            FileStatus::Same => "same",
            FileStatus::Differs => "differs",
            FileStatus::OnlyProfile => "only in profile (missing from live)",
            FileStatus::OnlyLive => "only in live (missing from profile)",
            FileStatus::Neither => "absent in both",
        };
        f.write_str(s)
    }
}

fn byte_status(profile: Option<&[u8]>, live: Option<&[u8]>) -> FileStatus {
    match (profile, live) {
        (Some(a), Some(b)) if a == b => FileStatus::Same,
        (Some(_), Some(_)) => FileStatus::Differs,
        (Some(_), None) => FileStatus::OnlyProfile,
        (None, Some(_)) => FileStatus::OnlyLive,
        (None, None) => FileStatus::Neither,
    }
}

/// Compare a profile's managed fields against the live config.
pub fn diff_profile(name: &str, layer: &ProfileLayer) -> Result<DiffResult> {
    let live_auth = read_optional(&paths::codex_auth_path()?)?;
    let auth = byte_status(layer.auth.as_deref(), live_auth.as_deref());

    let live_catalog_path = paths::catalog_path(name)?;
    let live_catalog = read_optional(&live_catalog_path)?;
    let catalog = byte_status(layer.catalog.as_deref(), live_catalog.as_deref());

    // Compare the profile fragment against the managed subset extracted from
    // live. A bundled catalog owns model_catalog_json, so exclude that key
    // from the live extract when comparing.
    let fragment = match &layer.fragment {
        Some(prof_frag) => {
            let mut live_frag = extract_fragment(&read_live_config()?);
            if layer.catalog.is_some() {
                live_frag.remove("model_catalog_json");
            }
            if docs_equal(prof_frag, &live_frag) {
                FileStatus::Same
            } else {
                FileStatus::Differs
            }
        }
        None => FileStatus::Neither,
    };

    Ok(DiffResult {
        auth,
        catalog,
        fragment,
    })
}

/// Recursively compare two TOML documents by value, ignoring formatting and
/// comments (which `toml_edit::Item` does not implement `PartialEq` for).
fn docs_equal(a: &DocumentMut, b: &DocumentMut) -> bool {
    items_equal(a.as_item(), b.as_item())
}

fn items_equal(a: &toml_edit::Item, b: &toml_edit::Item) -> bool {
    use toml_edit::Item;
    match (a, b) {
        (Item::None, Item::None) => true,
        (Item::Value(va), Item::Value(vb)) => values_equal(va, vb),
        (Item::Table(ta), Item::Table(tb)) => tables_equal(ta, tb),
        (Item::ArrayOfTables(aa), Item::ArrayOfTables(ab)) => {
            aa.iter().count() == ab.iter().count()
                && aa.iter().zip(ab.iter()).all(|(x, y)| tables_equal(x, y))
        }
        _ => false,
    }
}

fn tables_equal(a: &toml_edit::Table, b: &toml_edit::Table) -> bool {
    a.iter().count() == b.iter().count()
        && a.iter()
            .all(|(key, va)| b.get(key).map(|vb| items_equal(va, vb)).unwrap_or(false))
}

/// Recursively compare two TOML values (formatting/representation ignored).
fn values_equal(a: &toml_edit::Value, b: &toml_edit::Value) -> bool {
    use toml_edit::Value;
    match (a, b) {
        (Value::String(x), Value::String(y)) => x == y,
        (Value::Integer(x), Value::Integer(y)) => x == y,
        (Value::Float(x), Value::Float(y)) => x == y,
        (Value::Boolean(x), Value::Boolean(y)) => x == y,
        (Value::Datetime(x), Value::Datetime(y)) => x == y,
        (Value::Array(xa), Value::Array(xb)) => {
            xa.iter().count() == xb.iter().count()
                && xa.iter().zip(xb.iter()).all(|(a, b)| values_equal(a, b))
        }
        (Value::InlineTable(ta), Value::InlineTable(tb)) => {
            ta.iter().count() == tb.iter().count()
                && ta
                    .iter()
                    .all(|(key, va)| tb.get(key).map(|vb| values_equal(va, vb)).unwrap_or(false))
        }
        _ => false,
    }
}

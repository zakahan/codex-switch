use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

/// Write `data` to `path` atomically: write a temp file in the same directory,
/// then rename over the target so readers never see a half-written file.
///
/// When `mode` is given (Unix), the temp file is chmod'd before the rename so
/// the final file lands with those permissions (used to keep auth.json at 0600).
/// Otherwise an existing file's permissions are preserved.
pub fn atomic_write(path: &Path, data: &[u8], mode: Option<u32>) -> Result<()> {
    let parent = path
        .parent()
        .with_context(|| format!("path has no parent: {}", path.display()))?;
    fs::create_dir_all(parent)
        .with_context(|| format!("failed to create directory: {}", parent.display()))?;

    let file_name = path
        .file_name()
        .with_context(|| format!("path has no file name: {}", path.display()))?
        .to_string_lossy();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let tmp: PathBuf = parent.join(format!("{file_name}.tmp.{ts}"));

    {
        let mut f = fs::File::create(&tmp)
            .with_context(|| format!("failed to create temp file: {}", tmp.display()))?;
        f.write_all(data)
            .with_context(|| format!("failed to write temp file: {}", tmp.display()))?;
        f.flush()
            .with_context(|| format!("failed to flush temp file: {}", tmp.display()))?;
    }

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let target_mode = mode.or_else(|| {
            fs::metadata(path)
                .ok()
                .map(|meta| meta.permissions().mode())
        });
        if let Some(m) = target_mode {
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(m));
        }
    }
    #[cfg(not(unix))]
    {
        let _ = mode;
    }

    fs::rename(&tmp, path).map_err(|e| {
        let _ = fs::remove_file(&tmp);
        anyhow::anyhow!(
            "atomic replace failed: {} -> {}: {e}",
            tmp.display(),
            path.display()
        )
    })?;
    Ok(())
}

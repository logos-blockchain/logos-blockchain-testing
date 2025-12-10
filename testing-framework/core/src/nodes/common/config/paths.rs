use std::{fs, io, path::Path};

/// Ensure recovery-related directories and placeholder files exist under the
/// given base dir.
pub fn ensure_recovery_paths(base_dir: &Path) -> io::Result<()> {
    let recovery_dir = base_dir.join("recovery");
    fs::create_dir_all(&recovery_dir)?;

    let mempool_path = recovery_dir.join("mempool.json");
    if !mempool_path.exists() {
        fs::write(&mempool_path, "{}")?;
    }

    let blend_core_path = recovery_dir.join("blend").join("core.json");
    if let Some(parent) = blend_core_path.parent() {
        fs::create_dir_all(parent)?;
    }
    if !blend_core_path.exists() {
        fs::write(&blend_core_path, "{}")?;
    }

    Ok(())
}

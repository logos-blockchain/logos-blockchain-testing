mod api_client;
pub mod common;
pub mod node;

use std::sync::LazyLock;

pub use api_client::{ApiClient, ApiClientError};
use tempfile::TempDir;
use testing_framework_env as tf_env;
use tracing::info;

pub(crate) const LOGS_PREFIX: &str = "__logs";
static KEEP_NODE_TEMPDIRS: LazyLock<bool> = LazyLock::new(tf_env::nomos_tests_keep_logs);

pub(crate) fn create_tempdir() -> std::io::Result<TempDir> {
    // It's easier to use the current location instead of OS-default tempfile
    // location because Github Actions can easily access files in the current
    // location using wildcard to upload them as artifacts.
    TempDir::new_in(std::env::current_dir()?)
}

fn persist_tempdir(tempdir: &mut TempDir, label: &str) -> std::io::Result<()> {
    println!(
        "{}: persisting directory at {}",
        label,
        tempdir.path().display()
    );
    // we need ownership of the dir to persist it
    let dir = std::mem::replace(tempdir, tempfile::tempdir()?);
    let _ = dir.keep();
    Ok(())
}

pub(crate) fn persist_tempdir_to(
    tempdir: &mut TempDir,
    target_dir: &std::path::Path,
    label: &str,
) -> std::io::Result<()> {
    use std::fs;

    info!(
        label,
        from = %tempdir.path().display(),
        to = %target_dir.display(),
        "persisting directory"
    );

    // Create parent directory if it doesn't exist
    if let Some(parent) = target_dir.parent() {
        fs::create_dir_all(parent)?;
    }

    // Copy tempdir contents to target directory
    if target_dir.exists() {
        fs::remove_dir_all(target_dir)?;
    }

    /// Recursively copies all contents from src directory to dst directory.
    ///
    /// # Arguments
    /// * `src` - Source directory path to copy from
    /// * `dst` - Destination directory path to copy to
    fn copy_dir_all(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
        use std::fs;
        fs::create_dir_all(dst)?;
        for entry in fs::read_dir(src)? {
            let entry = entry?;
            let ty = entry.file_type()?;
            if ty.is_dir() {
                copy_dir_all(&entry.path(), &dst.join(entry.file_name()))?;
            } else {
                fs::copy(entry.path(), dst.join(entry.file_name()))?;
            }
        }
        Ok(())
    }

    copy_dir_all(tempdir.path(), target_dir)?;

    Ok(())
}

pub(crate) fn should_persist_tempdir() -> bool {
    std::thread::panicking() || *KEEP_NODE_TEMPDIRS
}

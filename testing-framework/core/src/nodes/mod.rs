mod api_client;
pub mod common;
pub mod node;

use std::{
    io::{Error, ErrorKind},
    path::PathBuf,
    sync::LazyLock,
};

pub use api_client::{ApiClient, ApiClientError};
use tempfile::TempDir;
use testing_framework_env as tf_env;

pub(crate) const LOGS_PREFIX: &str = "__logs";
static KEEP_NODE_TEMPDIRS: LazyLock<bool> = LazyLock::new(tf_env::nomos_tests_keep_logs);

pub(crate) fn create_tempdir(custom_work_dir: Option<PathBuf>) -> std::io::Result<TempDir> {
    if let Some(dir) = custom_work_dir {
        let final_dir_name = dir
            .components()
            .last()
            .ok_or(Error::new(ErrorKind::Other, "invalid final directory"))?
            .as_os_str()
            .display()
            .to_string()
            .to_owned()
            + "_";
        let parent_dir = dir
            .parent()
            .ok_or(Error::new(ErrorKind::Other, "invalid parent directory"))?;
        let mut temp_dir = TempDir::with_prefix_in(final_dir_name, parent_dir)?;
        if should_persist_tempdir() {
            temp_dir.disable_cleanup(true);
        }
        Ok(temp_dir)
    } else {
        // It's easier to use the current location instead of OS-default tempfile
        // location because Github Actions can easily access files in the current
        // location using wildcard to upload them as artifacts.
        let mut temp_dir = TempDir::new_in(std::env::current_dir()?)?;
        if should_persist_tempdir() {
            temp_dir.disable_cleanup(true);
        }
        Ok(temp_dir)
    }
}

fn persist_tempdir(tempdir: &mut TempDir, label: &str) -> std::io::Result<()> {
    println!(
        "{}: persisting directory at {}",
        label,
        tempdir.path().display()
    );
    if should_persist_tempdir() {
        return Ok(());
    }
    // we need ownership of the dir to persist it
    let dir = std::mem::replace(tempdir, tempfile::tempdir()?);
    let _ = dir.keep();
    Ok(())
}

pub(crate) fn should_persist_tempdir() -> bool {
    std::thread::panicking() || *KEEP_NODE_TEMPDIRS
}

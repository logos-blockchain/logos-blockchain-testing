//! Cross-process lock used by providers that materialize binaries.
//!
//! Cargo builds and downloads can be requested by multiple local test
//! processes at the same time. The lock keeps those processes from writing the
//! same target/cache path concurrently.

use std::{
    fs, io,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use super::types::BinaryProviderError;

const LOCK_RETRY_DELAY: Duration = Duration::from_millis(200);
const LOCK_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// File-backed lock removed automatically when dropped.
pub(super) struct BinaryProviderLock {
    /// Path of the lock file currently owned by this process.
    path: PathBuf,
}

impl BinaryProviderLock {
    pub(super) fn acquire(path: &Path) -> Result<Self, BinaryProviderError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|source| BinaryProviderError::Io {
                path: parent.to_owned(),
                source,
            })?;
        }

        let started = Instant::now();
        loop {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(path)
            {
                Ok(_) => {
                    return Ok(Self {
                        path: path.to_owned(),
                    });
                }
                Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                    if started.elapsed() >= LOCK_TIMEOUT {
                        return Err(BinaryProviderError::LockTimeout {
                            path: path.to_owned(),
                        });
                    }

                    thread::sleep(LOCK_RETRY_DELAY);
                }
                Err(source) => {
                    return Err(BinaryProviderError::Io {
                        path: path.to_owned(),
                        source,
                    });
                }
            }
        }
    }
}

impl Drop for BinaryProviderLock {
    fn drop(&mut self) {
        drop(fs::remove_file(&self.path));
    }
}

//! Download provider.
//!
//! This provider fetches an executable into a local cache, optionally validates
//! a SHA-256 checksum, and marks the downloaded file executable on Unix.

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;
use std::{
    collections::hash_map::DefaultHasher,
    env, fs,
    hash::{Hash as _, Hasher as _},
    path::{Path, PathBuf},
};

use reqwest::blocking;
use sha2::{Digest as _, Sha256};
use tracing::info;

use crate::binary::{
    BinaryProvider, BinaryProviderError, DownloadBinaryProvider, DownloadUrl,
    lock::BinaryProviderLock, optional_path_display,
};

impl BinaryProvider for DownloadBinaryProvider {
    fn try_resolve(&self) -> Result<Option<PathBuf>, BinaryProviderError> {
        let url = self.url.resolve()?;
        let path = self.cached_binary_path(&url)?;
        let _lock = BinaryProviderLock::acquire(&self.lock_path(&url))?;

        if path.is_file() {
            return Ok(Some(path));
        }

        let bytes = self.download_bytes(&url)?;
        self.verify_checksum(&path, &bytes)?;
        self.write_binary(&path, &bytes)?;

        Ok(Some(path))
    }

    fn display(&self) -> String {
        "download".to_owned()
    }

    fn cache_key(&self) -> String {
        format!(
            "download:{}:{}",
            self.url.cache_key(),
            optional_path_display(&self.cache_dir)
        )
    }
}

impl DownloadUrl {
    fn cache_key(&self) -> String {
        match self {
            Self::Fixed(url) => url.clone(),
            Self::Env(env_var) => format!("env:{env_var}"),
        }
    }
}

impl DownloadBinaryProvider {
    fn cached_binary_path(&self, url: &str) -> Result<PathBuf, BinaryProviderError> {
        let cache_dir = self.cache_dir();
        fs::create_dir_all(&cache_dir).map_err(|source| BinaryProviderError::Io {
            path: cache_dir.clone(),
            source,
        })?;

        Ok(cache_dir.join(self.download_file_name(url)))
    }

    fn download_bytes(&self, url: &str) -> Result<Vec<u8>, BinaryProviderError> {
        info!(url, "downloading binary");

        blocking::get(url)
            .map_err(|source| BinaryProviderError::Download {
                url: url.to_owned(),
                source,
            })?
            .error_for_status()
            .map_err(|source| BinaryProviderError::Download {
                url: url.to_owned(),
                source,
            })?
            .bytes()
            .map(|bytes| bytes.to_vec())
            .map_err(|source| BinaryProviderError::Download {
                url: url.to_owned(),
                source,
            })
    }

    fn write_binary(&self, path: &Path, bytes: &[u8]) -> Result<(), BinaryProviderError> {
        fs::write(path, bytes).map_err(|source| BinaryProviderError::Io {
            path: path.to_owned(),
            source,
        })?;

        self.make_executable(path)
    }

    fn verify_checksum(&self, path: &Path, bytes: &[u8]) -> Result<(), BinaryProviderError> {
        let Some(expected) = self.sha256.as_ref().and_then(|checksum| checksum.resolve()) else {
            return Ok(());
        };

        let actual = self.encode_sha256(bytes);
        if expected == actual {
            return Ok(());
        }

        Err(BinaryProviderError::ChecksumMismatch {
            path: path.to_owned(),
            expected,
            actual,
        })
    }

    fn cache_dir(&self) -> PathBuf {
        self.cache_dir.clone().unwrap_or_else(|| {
            env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .join("target")
                .join(".tf-binaries")
        })
    }

    fn lock_path(&self, url: &str) -> PathBuf {
        self.cache_dir()
            .join(format!("{}.lock", self.download_file_name(url)))
    }

    #[cfg(unix)]
    fn make_executable(&self, path: &Path) -> Result<(), BinaryProviderError> {
        let mut permissions = fs::metadata(path)
            .map_err(|source| BinaryProviderError::Io {
                path: path.to_owned(),
                source,
            })?
            .permissions();

        permissions.set_mode(0o755);
        fs::set_permissions(path, permissions).map_err(|source| BinaryProviderError::Io {
            path: path.to_owned(),
            source,
        })
    }

    #[cfg(not(unix))]
    fn make_executable(&self, _path: &Path) -> Result<(), BinaryProviderError> {
        Ok(())
    }

    fn encode_sha256(&self, bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    fn download_file_name(&self, url: &str) -> String {
        let mut hasher = DefaultHasher::new();
        url.hash(&mut hasher);

        format!("binary-{:x}", hasher.finish())
    }
}

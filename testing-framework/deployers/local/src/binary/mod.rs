//! Local binary resolution for process-based test deployments.
//!
//! A local node process has exactly one binary provider selected at launch
//! time. That provider owns its configuration and returns the executable path
//! used by [`LocalProcessSpec`](crate::LocalProcessSpec).

mod cache;
mod lock;
mod providers;
#[cfg(test)]
mod tests;
mod types;

use std::path::PathBuf;

use cache::BinaryCache;
pub(super) use types::optional_path_display;
pub use types::{
    BinaryProviderError, BinaryProviderRef, BuildBinaryProvider, BuildCommand,
    DownloadBinaryProvider, DownloadChecksum, DownloadProcessor, DownloadProcessorError,
    DownloadProcessorFn, DownloadUrl, EnvBinaryProvider, FallbackBinaryProvider,
    PathBinaryProvider,
};

/// Resolves an executable path for a local process.
///
/// Implementations return `Ok(None)` when they are valid but cannot resolve a
/// binary in the current environment. The default [`resolve`](Self::resolve)
/// method turns that into a launch error, while [`FallbackBinaryProvider`] uses
/// it to try several providers in order.
pub trait BinaryProvider: Send + Sync {
    fn try_resolve(&self) -> Result<Option<PathBuf>, BinaryProviderError>;

    fn display(&self) -> String;

    fn cache_key(&self) -> String;

    /// Resolves this provider into an executable path.
    ///
    /// Resolution is cached per process so repeated node starts using the same
    /// provider config do not rebuild, redownload, or rediscover the same
    /// binary.
    fn resolve(&self) -> Result<PathBuf, BinaryProviderError> {
        let cache_key = self.cache_key();

        if let Some(path) = BinaryCache::get(&cache_key) {
            return Ok(path);
        }

        let path = self.resolve_uncached()?;
        BinaryCache::insert(cache_key, path.clone());

        Ok(path)
    }

    fn resolve_uncached(&self) -> Result<PathBuf, BinaryProviderError> {
        if let Some(path) = self.try_resolve()? {
            return Ok(path);
        }

        Err(BinaryProviderError::NotFound {
            provider: self.display(),
        })
    }
}

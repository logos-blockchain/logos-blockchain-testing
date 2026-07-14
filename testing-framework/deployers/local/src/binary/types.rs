use std::{
    env, fmt, iter,
    path::{Path, PathBuf},
    sync::Arc,
};

use thiserror::Error;

/// Shared provider handle used by local process specs.
pub type BinaryProviderRef = Arc<dyn crate::binary::BinaryProvider>;

#[derive(Clone)]
pub struct FallbackBinaryProvider {
    /// Providers tried from first to last.
    ///
    /// This is useful when a test wants "prefer explicit override, otherwise
    /// build/download" behavior while still configuring one provider on the
    /// process spec.
    pub providers: Vec<BinaryProviderRef>,
}

impl FallbackBinaryProvider {
    #[must_use]
    pub fn new(providers: impl IntoIterator<Item = BinaryProviderRef>) -> Self {
        Self {
            providers: providers.into_iter().collect(),
        }
    }
}

impl fmt::Debug for FallbackBinaryProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("FallbackBinaryProvider")
            .field(
                "providers",
                &self
                    .providers
                    .iter()
                    .map(|provider| provider.display())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct EnvBinaryProvider {
    /// Env var expected to contain a full executable path.
    pub env_var: String,
}

impl EnvBinaryProvider {
    #[must_use]
    pub fn new(env_var: impl Into<String>) -> Self {
        Self {
            env_var: env_var.into(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct PathBinaryProvider {
    /// Absolute executable path selected by the test/app config.
    pub path: PathBuf,
}

impl PathBinaryProvider {
    #[must_use]
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }
}

/// User-supplied command used by [`BuildBinaryProvider`].
#[derive(Clone, Debug)]
pub struct BuildCommand {
    /// Program executed by the build provider.
    pub program: String,
    /// Arguments passed to `program`.
    pub args: Vec<String>,
}

impl BuildCommand {
    #[must_use]
    pub fn new(program: impl Into<String>) -> Self {
        Self {
            program: program.into(),
            args: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args = args.into_iter().map(Into::into).collect();
        self
    }

    pub(super) fn display(&self) -> String {
        iter::once(self.program.as_str())
            .chain(self.args.iter().map(String::as_str))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

#[derive(Clone, Debug)]
pub struct BuildBinaryProvider {
    /// Command that prepares the executable.
    ///
    /// The command is intentionally not Cargo-specific. Tests can point this at
    /// a shell script, Make target, Cargo invocation, remote cache fetch, or
    /// any other project-specific build step.
    pub command: BuildCommand,
    /// Executable path expected to exist after `command` completes.
    pub output_path: PathBuf,
    /// Working directory for the build command.
    ///
    /// If unset, the current process directory is used.
    pub working_dir: Option<PathBuf>,
    /// Directory used for the build lock file.
    ///
    /// If unset, the lock is placed under `.tf-binaries` in `working_dir`.
    pub lock_dir: Option<PathBuf>,
}

impl BuildBinaryProvider {
    #[must_use]
    pub fn new(command: BuildCommand, output_path: impl Into<PathBuf>) -> Self {
        Self {
            command,
            output_path: output_path.into(),
            working_dir: None,
            lock_dir: None,
        }
    }
}

/// Error returned by integration-specific download processors.
pub type DownloadProcessorError = Box<dyn std::error::Error + Send + Sync + 'static>;

/// Post-download preparation step for artifacts that are not directly
/// executable.
///
/// Implementations receive the checksum-verified downloaded artifact and must
/// materialize the executable at `output`. The stable cache key ensures that a
/// change in preparation logic invalidates the previously prepared binary.
pub trait DownloadProcessor: Send + Sync {
    fn process(&self, artifact: &Path, output: &Path) -> Result<(), DownloadProcessorError>;

    fn cache_key(&self) -> &str;
}

type DownloadProcessorCallback =
    dyn Fn(&Path, &Path) -> Result<(), DownloadProcessorError> + Send + Sync;

/// Named callback adapter for lightweight, integration-specific processing.
#[derive(Clone)]
pub struct DownloadProcessorFn {
    cache_key: String,
    callback: Arc<DownloadProcessorCallback>,
}

impl DownloadProcessorFn {
    #[must_use]
    pub fn new(
        cache_key: impl Into<String>,
        callback: impl Fn(&Path, &Path) -> Result<(), DownloadProcessorError> + Send + Sync + 'static,
    ) -> Self {
        Self {
            cache_key: cache_key.into(),
            callback: Arc::new(callback),
        }
    }
}

impl DownloadProcessor for DownloadProcessorFn {
    fn process(&self, artifact: &Path, output: &Path) -> Result<(), DownloadProcessorError> {
        (self.callback)(artifact, output)
    }

    fn cache_key(&self) -> &str {
        &self.cache_key
    }
}

impl fmt::Debug for DownloadProcessorFn {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DownloadProcessorFn")
            .field("cache_key", &self.cache_key)
            .finish_non_exhaustive()
    }
}

#[derive(Clone)]
pub struct DownloadBinaryProvider {
    /// Download source used to fetch the executable.
    pub url: DownloadUrl,
    /// Optional SHA-256 checksum validated before the binary is accepted.
    pub sha256: Option<DownloadChecksum>,
    /// Directory used to store downloaded binaries and provider lock files.
    ///
    /// If unset, downloads are cached under `target/.tf-binaries` in the
    /// current process directory.
    pub cache_dir: Option<PathBuf>,
    /// Optional preparation step for archives or other packaged artifacts.
    pub processor: Option<Arc<dyn DownloadProcessor>>,
}

impl DownloadBinaryProvider {
    #[must_use]
    pub fn from_url(url: impl Into<String>) -> Self {
        Self {
            url: DownloadUrl::Fixed(url.into()),
            sha256: None,
            cache_dir: None,
            processor: None,
        }
    }

    #[must_use]
    pub fn with_processor(mut self, processor: impl DownloadProcessor + 'static) -> Self {
        self.processor = Some(Arc::new(processor));
        self
    }

    #[must_use]
    pub fn with_processor_fn(
        self,
        cache_key: impl Into<String>,
        callback: impl Fn(&Path, &Path) -> Result<(), DownloadProcessorError> + Send + Sync + 'static,
    ) -> Self {
        self.with_processor(DownloadProcessorFn::new(cache_key, callback))
    }
}

impl fmt::Debug for DownloadBinaryProvider {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DownloadBinaryProvider")
            .field("url", &self.url)
            .field("sha256", &self.sha256)
            .field("cache_dir", &self.cache_dir)
            .field(
                "processor",
                &self
                    .processor
                    .as_ref()
                    .map(|processor| processor.cache_key()),
            )
            .finish()
    }
}

#[derive(Clone, Debug)]
pub enum DownloadUrl {
    /// Fixed URL embedded in the provider config.
    Fixed(String),
    /// Env var containing the URL.
    Env(String),
}

impl DownloadUrl {
    pub(super) fn resolve(&self) -> Result<String, BinaryProviderError> {
        match self {
            Self::Fixed(url) => Ok(url.clone()),
            Self::Env(env_var) => {
                env::var(env_var).map_err(|_| BinaryProviderError::MissingDownloadUrl {
                    env_var: env_var.clone(),
                })
            }
        }
    }
}

#[derive(Clone, Debug)]
pub enum DownloadChecksum {
    /// Fixed expected SHA-256 checksum.
    Fixed(String),
    /// Env var containing the expected SHA-256 checksum.
    Env(String),
}

impl DownloadChecksum {
    pub(super) fn resolve(&self) -> Option<String> {
        match self {
            Self::Fixed(checksum) => Some(checksum.to_ascii_lowercase()),
            Self::Env(env_var) => env::var(env_var)
                .ok()
                .map(|checksum| checksum.to_ascii_lowercase()),
        }
    }
}

pub(crate) fn optional_path_display(path: &Option<PathBuf>) -> String {
    path.as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_default()
}

#[derive(Debug, Error)]
pub enum BinaryProviderError {
    /// The selected provider completed without producing an executable path.
    #[error("binary could not be resolved by provider {provider}")]
    NotFound {
        /// Human-readable provider name or fallback provider chain.
        provider: String,
    },
    /// Build command returned a non-zero status.
    #[error("build command failed with status {status}")]
    BuildFailed {
        /// Build command exit status.
        status: String,
    },
    /// Build command succeeded but did not create the configured output file.
    #[error("build command did not produce configured binary output {path:?}")]
    MissingBuildOutput {
        /// Expected output path configured on the build provider.
        path: PathBuf,
    },
    /// Download provider was selected but no URL was configured.
    #[error("download provider requires env var {env_var} to contain a binary URL")]
    MissingDownloadUrl {
        /// Env var expected to contain the download URL.
        env_var: String,
    },
    /// Configured path provider received a relative path.
    #[error("binary path must be absolute: {path:?}")]
    RelativePath {
        /// Relative path rejected by the path provider.
        path: PathBuf,
    },
    /// HTTP download failed or returned an error status.
    #[error("failed to download binary from {url}: {source}")]
    Download {
        /// URL that failed.
        url: String,
        #[source]
        /// HTTP client error.
        source: reqwest::Error,
    },
    /// Downloaded bytes did not match the configured SHA-256 checksum.
    #[error("downloaded binary sha256 mismatch for {path:?}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        /// Cache path the downloaded binary would have been written to.
        path: PathBuf,
        /// Configured lowercase SHA-256 checksum.
        expected: String,
        /// Actual lowercase SHA-256 checksum of the downloaded bytes.
        actual: String,
    },
    /// A download processor completed without creating its configured output.
    #[error("download processor {processor} did not produce binary output {path:?}")]
    MissingProcessedOutput {
        /// Stable identifier of the processor that failed to produce output.
        processor: String,
        /// Expected temporary executable output path.
        path: PathBuf,
    },
    /// A configured download processor failed while preparing the executable.
    #[error("download processor {processor} failed: {source}")]
    DownloadProcessing {
        /// Stable identifier of the processor that failed.
        processor: String,
        #[source]
        /// Processor-specific error.
        source: DownloadProcessorError,
    },
    /// Filesystem operation failed while preparing or resolving a binary.
    #[error("failed to prepare binary path {path:?}: {source}")]
    Io {
        /// Path involved in the failing filesystem operation.
        path: PathBuf,
        #[source]
        /// Underlying filesystem error.
        source: std::io::Error,
    },
    /// Another test process held the provider lock for too long.
    #[error("timed out waiting for binary provider lock {path:?}")]
    LockTimeout {
        /// Lock file path that could not be acquired.
        path: PathBuf,
    },
}

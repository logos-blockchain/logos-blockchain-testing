use std::{env, path::PathBuf};

use tracing::{debug, info};

pub struct BinaryConfig {
    /// Env var that overrides binary path.
    pub env_var: &'static str,
    /// Binary name expected on PATH.
    pub binary_name: &'static str,
    /// Repository-local fallback path when PATH lookup fails.
    pub fallback_path: &'static str,
}

pub struct BinaryResolver;

impl BinaryResolver {
    #[must_use]
    pub fn resolve_path(config: &BinaryConfig) -> PathBuf {
        if let Some(path) = Self::resolve_from_env(config) {
            return path;
        }

        if let Some(path) = Self::resolve_from_path(config.binary_name) {
            return path;
        }

        Self::fallback_path(config.binary_name, config.fallback_path)
    }

    fn which_on_path(bin: &str) -> Option<PathBuf> {
        let path_env = env::var_os("PATH")?;
        env::split_paths(&path_env)
            .map(|p| p.join(bin))
            .find(|candidate| candidate.is_file())
    }

    fn resolve_from_env(config: &BinaryConfig) -> Option<PathBuf> {
        let path = env::var_os(config.env_var).map(PathBuf::from)?;

        info!(
            env = config.env_var,
            binary = config.binary_name,
            path = %path.display(),
            "resolved binary from env override"
        );

        Some(path)
    }

    fn resolve_from_path(binary_name: &str) -> Option<PathBuf> {
        let path = Self::which_on_path(binary_name)?;

        info!(
            binary = binary_name,
            path = %path.display(),
            "resolved binary from PATH"
        );

        Some(path)
    }

    fn fallback_path(binary_name: &str, fallback_path: &str) -> PathBuf {
        let fallback = PathBuf::from(fallback_path);

        debug!(
            binary = binary_name,
            path = %fallback.display(),
            "falling back to binary path"
        );

        fallback
    }
}

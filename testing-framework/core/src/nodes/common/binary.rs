use std::{env, path::PathBuf};

use tracing::{debug, info};

pub struct BinaryConfig {
    pub env_var: &'static str,
    pub binary_name: &'static str,
    pub fallback_path: &'static str,
    pub shared_bin_subpath: &'static str,
}

pub struct BinaryResolver;

impl BinaryResolver {
    pub fn resolve_path(config: &BinaryConfig) -> PathBuf {
        if let Some(path) = env::var_os(config.env_var) {
            let resolved = PathBuf::from(path);

            info!(
                env = config.env_var,
                binary = config.binary_name,
                path = %resolved.display(),
                "resolved binary from env override"
            );
            return resolved;
        }
        if let Some(path) = Self::which_on_path(config.binary_name) {
            info!(
                binary = config.binary_name,
                path = %path.display(),
                "resolved binary from PATH"
            );
            return path;
        }
        let shared_bin = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(config.shared_bin_subpath);
        if shared_bin.exists() {
            info!(
                binary = config.binary_name,
                path = %shared_bin.display(),
                "resolved binary from shared assets"
            );
            return shared_bin;
        }
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../");
        let fallback = root.join(config.fallback_path);

        debug!(
            binary = config.binary_name,
            path = %fallback.display(),
            "falling back to binary path"
        );
        fallback
    }

    fn which_on_path(bin: &str) -> Option<PathBuf> {
        let path_env = env::var_os("PATH")?;
        env::split_paths(&path_env)
            .map(|p| p.join(bin))
            .find(|candidate| candidate.is_file())
    }
}

use std::{
    env, fs,
    path::{Path, PathBuf},
};

use tracing_subscriber::{EnvFilter, fmt};

use crate::DeployerKind;

const DEFAULT_NODE_LOG_DIR_REL: &str = ".tmp/node-logs";
const DEFAULT_CONTAINER_NODE_LOG_DIR: &str = "/tmp/node-logs";

fn set_default_env(key: &str, value: &str) {
    if std::env::var_os(key).is_none() {
        // SAFETY: Used as an early-run default. Prefer setting env vars in the
        // shell for multi-threaded runs.
        unsafe {
            std::env::set_var(key, value);
        }
    }
}

pub fn init_logging_defaults() {
    set_default_env("POL_PROOF_DEV_MODE", "true");
    set_default_env("LOGOS_BLOCKCHAIN_TESTS_KEEP_LOGS", "1");
    set_default_env("LOGOS_BLOCKCHAIN_LOG_LEVEL", "info");
    set_default_env("RUST_LOG", "info");
}

pub fn init_node_log_dir_defaults(deployer: DeployerKind) {
    if env::var_os("LOGOS_BLOCKCHAIN_LOG_DIR").is_some() {
        return;
    }

    let host_dir = repo_root().join(DEFAULT_NODE_LOG_DIR_REL);
    let _ = fs::create_dir_all(&host_dir);

    match deployer {
        DeployerKind::Local => {
            set_default_env("LOGOS_BLOCKCHAIN_LOG_DIR", &host_dir.display().to_string())
        }
        DeployerKind::Compose => {
            set_default_env("LOGOS_BLOCKCHAIN_LOG_DIR", DEFAULT_CONTAINER_NODE_LOG_DIR)
        }
    }
}

pub fn init_tracing() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let _ = fmt().with_env_filter(filter).with_target(true).try_init();
}

fn repo_root() -> PathBuf {
    env::var("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .map(Path::to_path_buf)
        })
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")))
}

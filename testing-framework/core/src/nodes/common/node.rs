use std::{
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    time::Duration,
};

use nomos_tracing_service::LoggerLayer;
use reqwest::Url;
use serde::Serialize;
use tempfile::TempDir;
use tokio::time;
use tracing::{debug, info};

use super::lifecycle::monitor::is_running;
use crate::nodes::{
    ApiClient,
    common::{config::paths::ensure_recovery_paths, lifecycle::spawn::configure_logging},
    create_tempdir, persist_tempdir,
};

const EXIT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(100);
const STARTUP_TIMEOUT: Duration = Duration::from_secs(60);

pub type NodeAddresses = (SocketAddr, Option<SocketAddr>);
pub type PreparedNodeConfig<T> = (TempDir, T, SocketAddr, Option<SocketAddr>);

#[derive(Debug, thiserror::Error)]
pub enum SpawnNodeError {
    #[error("failed to create node tempdir: {source}")]
    TempDir {
        #[source]
        source: io::Error,
    },
    #[error("failed to prepare node recovery paths: {source}")]
    RecoveryPaths {
        #[source]
        source: io::Error,
    },
    #[error("failed to write node config at {path}: {source}")]
    WriteConfig {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to spawn node process '{binary}': {source}")]
    Spawn {
        binary: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("node did not become ready before timeout: {source}")]
    Readiness {
        #[source]
        source: tokio::time::error::Elapsed,
    },
}

/// Minimal interface to apply common node setup.
pub trait NodeConfigCommon {
    fn set_logger(&mut self, logger: LoggerLayer);
    fn set_paths(&mut self, base: &Path);
    fn addresses(&self) -> NodeAddresses;
}

/// Shared handle for spawned nodes that exposes common operations.
pub struct NodeHandle<T> {
    pub(crate) child: Child,
    pub(crate) tempdir: TempDir,
    pub(crate) config: T,
    pub(crate) api: ApiClient,
}

impl<T> NodeHandle<T> {
    pub fn new(child: Child, tempdir: TempDir, config: T, api: ApiClient) -> Self {
        Self {
            child,
            tempdir,
            config,
            api,
        }
    }

    #[must_use]
    pub fn url(&self) -> Url {
        self.api.base_url().clone()
    }

    #[must_use]
    pub fn testing_url(&self) -> Option<Url> {
        self.api.testing_url()
    }

    #[must_use]
    pub fn api(&self) -> &ApiClient {
        &self.api
    }

    #[must_use]
    pub const fn config(&self) -> &T {
        &self.config
    }

    /// Returns true if the process exited within the timeout, false otherwise.
    pub async fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        time::timeout(timeout, async {
            loop {
                if !is_running(&mut self.child) {
                    return;
                }
                time::sleep(EXIT_POLL_INTERVAL).await;
            }
        })
        .await
        .is_ok()
    }
}

/// Apply common setup (recovery paths, logging, data dirs) and return a ready
/// config plus API addrs.
pub fn prepare_node_config<T: NodeConfigCommon>(
    mut config: T,
    log_prefix: &str,
    enable_logging: bool,
) -> Result<PreparedNodeConfig<T>, SpawnNodeError> {
    let dir = create_tempdir().map_err(|source| SpawnNodeError::TempDir { source })?;

    debug!(dir = %dir.path().display(), log_prefix, enable_logging, "preparing node config");

    // Ensure recovery files/dirs exist so services that persist state do not fail
    // on startup.
    ensure_recovery_paths(dir.path()).map_err(|source| SpawnNodeError::RecoveryPaths { source })?;

    if enable_logging {
        configure_logging(dir.path(), log_prefix, |file_cfg| {
            config.set_logger(LoggerLayer::File(file_cfg));
        });
    }

    config.set_paths(dir.path());
    let (addr, testing_addr) = config.addresses();

    debug!(addr = %addr, testing_addr = ?testing_addr, "configured node addresses");

    Ok((dir, config, addr, testing_addr))
}

/// Spawn a node with shared setup, config writing, and readiness wait.
pub async fn spawn_node<C>(
    config: C,
    log_prefix: &str,
    config_filename: &str,
    binary_path: PathBuf,
    enable_logging: bool,
) -> Result<NodeHandle<C>, SpawnNodeError>
where
    C: NodeConfigCommon + Serialize,
{
    let (dir, config, addr, testing_addr) =
        prepare_node_config(config, log_prefix, enable_logging)?;
    let config_path = dir.path().join(config_filename);
    super::lifecycle::spawn::write_config_with_injection(&config, &config_path, |yaml| {
        crate::nodes::common::config::injection::inject_ibd_into_cryptarchia(yaml)
    })
    .map_err(|source| SpawnNodeError::WriteConfig {
        path: config_path.clone(),
        source,
    })?;

    debug!(config_file = %config_path.display(), binary = %binary_path.display(), "spawning node process");

    let child = Command::new(&binary_path)
        .arg(&config_path)
        .current_dir(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|source| SpawnNodeError::Spawn {
            binary: binary_path.clone(),
            source,
        })?;

    let mut handle = NodeHandle::new(child, dir, config, ApiClient::new(addr, testing_addr));

    // Wait for readiness via consensus_info
    let ready = time::timeout(STARTUP_TIMEOUT, async {
        loop {
            if handle.api.consensus_info().await.is_ok() {
                break;
            }
            time::sleep(STARTUP_POLL_INTERVAL).await;
        }
    })
    .await;

    if let Err(err) = ready {
        // Persist tempdir to aid debugging if readiness fails.
        let _ = persist_tempdir(&mut handle.tempdir, "nomos-node");
        return Err(SpawnNodeError::Readiness { source: err });
    }

    info!("node readiness confirmed via consensus_info");
    Ok(handle)
}

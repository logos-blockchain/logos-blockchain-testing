use std::{ops::Deref, path::PathBuf, time::Duration};

use nomos_node::Config;
use nomos_tracing_service::LoggerLayer;
pub use testing_framework_config::nodes::node::create_node_config;
use tracing::{debug, info};

use super::{persist_tempdir, should_persist_tempdir};
use crate::{
    IS_DEBUG_TRACING,
    nodes::{
        LOGS_PREFIX,
        common::{
            binary::{BinaryConfig, BinaryResolver},
            lifecycle::{kill::kill_child, monitor::is_running},
            node::{
                NodeAddresses, NodeConfigCommon, NodeHandle, SpawnNodeError, spawn_node,
                spawn_node_process, wait_for_consensus_readiness,
            },
        },
    },
    scenario::DynError,
    topology::config::NodeConfigPatch,
};

const BIN_PATH: &str = "target/debug/logos-blockchain-node";
const RESTART_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

fn binary_path() -> PathBuf {
    let cfg = BinaryConfig {
        env_var: "LOGOS_BLOCKCHAIN_NODE_BIN",
        binary_name: "logos-blockchain-node",
        fallback_path: BIN_PATH,
        shared_bin_subpath: "../assets/stack/bin/logos-blockchain-node",
    };
    BinaryResolver::resolve_path(&cfg)
}

pub struct Node {
    handle: NodeHandle<Config>,
}

pub fn apply_node_config_patches<'a>(
    mut config: Config,
    patches: impl IntoIterator<Item = &'a NodeConfigPatch>,
) -> Result<Config, DynError> {
    for patch in patches {
        config = patch(config)?;
    }
    Ok(config)
}

pub fn apply_node_config_patch(
    config: Config,
    patch: &NodeConfigPatch,
) -> Result<Config, DynError> {
    apply_node_config_patches(config, [patch])
}

impl Deref for Node {
    type Target = NodeHandle<Config>;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl Drop for Node {
    fn drop(&mut self) {
        if should_persist_tempdir()
            && let Err(e) = persist_tempdir(&mut self.handle.tempdir, "logos-blockchain-node")
        {
            debug!(error = ?e, "failed to persist node tempdir");
        }

        debug!("stopping node process");
        kill_child(&mut self.handle.child);
    }
}

impl Node {
    /// Return the current process id for the running node.
    #[must_use]
    pub fn pid(&self) -> u32 {
        self.handle.child.id()
    }

    /// Check if the node process is still running
    pub fn is_running(&mut self) -> bool {
        is_running(&mut self.handle.child)
    }

    /// Wait for the node process to exit, with a timeout
    /// Returns true if the process exited within the timeout, false otherwise
    pub async fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        self.handle.wait_for_exit(timeout).await
    }

    pub async fn spawn(config: Config, label: &str) -> Result<Self, SpawnNodeError> {
        let log_prefix = format!("{LOGS_PREFIX}-{label}");
        let handle = spawn_node(
            config,
            &log_prefix,
            "node.yaml",
            binary_path(),
            !*IS_DEBUG_TRACING,
        )
        .await?;

        info!("node spawned and ready");

        Ok(Self { handle })
    }

    /// Restart the node process using the existing config and data directory.
    pub async fn restart(&mut self) -> Result<(), SpawnNodeError> {
        let old_pid = self.pid();
        debug!(old_pid, "restarting node process");

        kill_child(&mut self.handle.child);
        let _ = self.wait_for_exit(RESTART_SHUTDOWN_TIMEOUT).await;

        let config_path = self.handle.tempdir.path().join("node.yaml");
        let child = spawn_node_process(&binary_path(), &config_path, self.handle.tempdir.path())?;
        self.handle.child = child;

        let new_pid = self.pid();
        wait_for_consensus_readiness(&self.handle.api)
            .await
            .map_err(|source| SpawnNodeError::Readiness { source })?;

        info!(
            old_pid,
            new_pid, "node restart readiness confirmed via consensus_info"
        );
        Ok(())
    }
}

impl NodeConfigCommon for Config {
    fn set_logger(&mut self, logger: LoggerLayer) {
        self.tracing.logger = logger;
    }

    fn set_paths(&mut self, base: &std::path::Path) {
        self.storage.db_path = base.join("db");
    }

    fn addresses(&self) -> NodeAddresses {
        (
            self.http.backend_settings.address,
            Some(self.testing_http.backend_settings.address),
        )
    }
}

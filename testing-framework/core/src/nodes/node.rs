use std::{ops::Deref, path::PathBuf, time::Duration};

use nomos_node::config::RunConfig;
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
            node::{NodeAddresses, NodeConfigCommon, NodeHandle, SpawnNodeError, spawn_node},
        },
    },
    scenario::DynError,
    topology::config::NodeConfigPatch,
};

const BIN_PATH: &str = "target/debug/logos-blockchain-node";

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
    handle: NodeHandle<RunConfig>,
}

pub fn apply_node_config_patches<'a>(
    mut config: RunConfig,
    patches: impl IntoIterator<Item = &'a NodeConfigPatch>,
) -> Result<RunConfig, DynError> {
    for patch in patches {
        config = patch(config)?;
    }
    Ok(config)
}

pub fn apply_node_config_patch(
    config: RunConfig,
    patch: &NodeConfigPatch,
) -> Result<RunConfig, DynError> {
    apply_node_config_patches(config, [patch])
}

impl Deref for Node {
    type Target = NodeHandle<RunConfig>;

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
    /// Check if the node process is still running
    pub fn is_running(&mut self) -> bool {
        is_running(&mut self.handle.child)
    }

    /// Wait for the node process to exit, with a timeout
    /// Returns true if the process exited within the timeout, false otherwise
    pub async fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        self.handle.wait_for_exit(timeout).await
    }

    pub async fn spawn(config: RunConfig, label: &str) -> Result<Self, SpawnNodeError> {
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
}

impl NodeConfigCommon for RunConfig {
    fn set_logger(&mut self, logger: LoggerLayer) {
        self.user.tracing.logger = logger;
    }

    fn set_paths(&mut self, base: &std::path::Path) {
        self.user.storage.db_path = base.join("db");
    }

    fn addresses(&self) -> NodeAddresses {
        (
            self.user.http.backend_settings.address,
            Some(self.user.testing_http.backend_settings.address),
        )
    }
}

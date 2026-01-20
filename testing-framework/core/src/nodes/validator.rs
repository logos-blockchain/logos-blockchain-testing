use std::{ops::Deref, path::PathBuf, time::Duration};

use nomos_node::Config;
use nomos_tracing_service::LoggerLayer;
pub use testing_framework_config::nodes::validator::create_validator_config;
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
};

const BIN_PATH: &str = "target/debug/logos-blockchain-node";

fn binary_path() -> PathBuf {
    let cfg = BinaryConfig {
        env_var: "NOMOS_NODE_BIN",
        binary_name: "logos-blockchain-node",
        fallback_path: BIN_PATH,
        shared_bin_subpath: "../assets/stack/bin/logos-blockchain-node",
    };
    BinaryResolver::resolve_path(&cfg)
}

pub enum Pool {
    Da,
    Mantle,
}

pub struct Validator {
    handle: NodeHandle<Config>,
}

impl Deref for Validator {
    type Target = NodeHandle<Config>;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl Drop for Validator {
    fn drop(&mut self) {
        if should_persist_tempdir()
            && let Err(e) = persist_tempdir(&mut self.handle.tempdir, "logos-blockchain-node")
        {
            debug!(error = ?e, "failed to persist validator tempdir");
        }

        debug!("stopping validator process");
        kill_child(&mut self.handle.child);
    }
}

impl Validator {
    /// Check if the validator process is still running
    pub fn is_running(&mut self) -> bool {
        is_running(&mut self.handle.child)
    }

    /// Wait for the validator process to exit, with a timeout
    /// Returns true if the process exited within the timeout, false otherwise
    pub async fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        self.handle.wait_for_exit(timeout).await
    }

    pub async fn spawn(config: Config) -> Result<Self, SpawnNodeError> {
        let handle = spawn_node(
            config,
            LOGS_PREFIX,
            "validator.yaml",
            binary_path(),
            !*IS_DEBUG_TRACING,
        )
        .await?;

        info!("validator spawned and ready");

        Ok(Self { handle })
    }
}

impl NodeConfigCommon for Config {
    fn set_logger(&mut self, logger: LoggerLayer) {
        self.tracing.logger = logger;
    }

    fn set_paths(&mut self, base: &std::path::Path) {
        self.storage.db_path = base.join("db");
        base.clone_into(
            &mut self
                .da_verifier
                .storage_adapter_settings
                .blob_storage_directory,
        );
    }

    fn addresses(&self) -> NodeAddresses {
        (
            self.http.backend_settings.address,
            Some(self.testing_http.backend_settings.address),
        )
    }
}

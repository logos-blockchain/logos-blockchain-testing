use std::{
    ops::Deref,
    path::{Path, PathBuf},
    time::Duration,
};

use logos_blockchain_executor::config::Config;
use nomos_tracing_service::LoggerLayer;
pub use testing_framework_config::nodes::executor::create_executor_config;
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

const BIN_PATH: &str = "target/debug/logos-blockchain-executor";

fn binary_path() -> PathBuf {
    let cfg = BinaryConfig {
        env_var: "LOGOS_BLOCKCHAIN_EXECUTOR_BIN",
        binary_name: "logos-blockchain-executor",
        fallback_path: BIN_PATH,
        shared_bin_subpath: "../assets/stack/bin/logos-blockchain-executor",
    };
    BinaryResolver::resolve_path(&cfg)
}

pub struct Executor {
    handle: NodeHandle<Config>,
}

impl Deref for Executor {
    type Target = NodeHandle<Config>;

    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}

impl Drop for Executor {
    fn drop(&mut self) {
        if should_persist_tempdir()
            && let Err(e) = persist_tempdir(&mut self.handle.tempdir, "logos-blockchain-executor")
        {
            debug!(error = ?e, "failed to persist executor tempdir");
        }

        debug!("stopping executor process");
        kill_child(&mut self.handle.child);
    }
}

impl Executor {
    pub async fn spawn(config: Config, label: &str) -> Result<Self, SpawnNodeError> {
        let log_prefix = format!("{LOGS_PREFIX}-{label}");
        let handle = spawn_node(
            config,
            &log_prefix,
            "executor.yaml",
            binary_path(),
            !*IS_DEBUG_TRACING,
        )
        .await?;

        info!("executor spawned and ready");

        Ok(Self { handle })
    }

    /// Check if the executor process is still running
    pub fn is_running(&mut self) -> bool {
        is_running(&mut self.handle.child)
    }

    /// Wait for the executor process to exit, with a timeout.
    pub async fn wait_for_exit(&mut self, timeout: Duration) -> bool {
        self.handle.wait_for_exit(timeout).await
    }
}

impl NodeConfigCommon for Config {
    fn set_logger(&mut self, logger: LoggerLayer) {
        self.tracing.logger = logger;
    }

    fn set_paths(&mut self, base: &Path) {
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

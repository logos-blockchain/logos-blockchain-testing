use std::{path::PathBuf, sync::Arc};

use testing_framework_core::scenario::DynError;
use testing_framework_runner_local::{
    BinaryProviderRef, BuildBinaryProvider, BuildCommand, EnvBinaryProvider,
    FallbackBinaryProvider, LocalBinaryApp, LocalBuildContext, LocalProcessSpec, PreparedNode,
    build_local_cluster_node_config, yaml_node_config,
};

use crate::{QueueEnv, QueueNodeConfig};

impl LocalBinaryApp for QueueEnv {
    fn build_node_config(
        context: LocalBuildContext<'_, Self>,
    ) -> Result<PreparedNode<QueueNodeConfig>, DynError> {
        let config =
            build_local_cluster_node_config::<Self>(context.index, context.ports, context.peers)?;
        Ok(PreparedNode {
            name: format!("queue-node-{}", context.index),
            config,
            network_port: context.ports.network_port(),
        })
    }

    fn local_process_spec(_config: &Self::NodeConfig) -> LocalProcessSpec {
        LocalProcessSpec::new("QUEUE_NODE_BIN")
            .with_binary_provider(queue_binary_provider())
            .with_rust_log("queue_node=info")
    }

    fn render_local_config(config: &QueueNodeConfig) -> Result<Vec<u8>, DynError> {
        yaml_node_config(config)
    }

    fn http_api_port(config: &QueueNodeConfig) -> u16 {
        config.http_port
    }
}

fn queue_binary_provider() -> FallbackBinaryProvider {
    let workspace = workspace_root();
    let providers: [BinaryProviderRef; 2] = [
        Arc::new(EnvBinaryProvider::new("QUEUE_NODE_BIN")),
        Arc::new(BuildBinaryProvider {
            command: BuildCommand::new("cargo").with_args([
                "build",
                "-p",
                "queue-node",
                "--bin",
                "queue-node",
            ]),
            output_path: PathBuf::from(format!(
                "target/debug/queue-node{}",
                std::env::consts::EXE_SUFFIX
            )),
            working_dir: Some(workspace),
            lock_dir: None,
        }),
    ];

    FallbackBinaryProvider::new(providers)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../..")
}

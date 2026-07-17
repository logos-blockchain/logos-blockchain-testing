use std::{collections::HashMap, path::PathBuf, sync::Arc};

use testing_framework_core::scenario::{DynError, StartNodeOptions};
use testing_framework_runner_local::{
    BinaryProviderRef, BuildBinaryProvider, BuildCommand, EnvBinaryProvider,
    FallbackBinaryProvider, LocalBinaryApp, LocalNodePorts, LocalPeerNode, LocalProcessSpec,
    build_local_cluster_node_config, yaml_node_config,
};

use crate::{KvEnv, KvNodeConfig};

impl LocalBinaryApp for KvEnv {
    fn initial_node_name_prefix() -> &'static str {
        "kv-node"
    }

    fn build_local_node_config_with_peers(
        _topology: &Self::Deployment,
        index: usize,
        ports: &LocalNodePorts,
        peers: &[LocalPeerNode],
        _peer_ports_by_name: &HashMap<String, u16>,
        _options: &StartNodeOptions<Self>,
        _template_config: Option<
            &<Self as testing_framework_core::scenario::Application>::NodeConfig,
        >,
    ) -> Result<<Self as testing_framework_core::scenario::Application>::NodeConfig, DynError> {
        build_local_cluster_node_config::<Self>(index, ports, peers)
    }

    fn local_process_spec() -> LocalProcessSpec {
        LocalProcessSpec::new("KVSTORE_NODE_BIN")
            .with_binary_provider(kvstore_binary_provider())
            .with_rust_log("kvstore_node=info")
    }

    fn render_local_config(config: &KvNodeConfig) -> Result<Vec<u8>, DynError> {
        yaml_node_config(config)
    }

    fn http_api_port(config: &KvNodeConfig) -> u16 {
        config.http_port
    }
}

fn kvstore_binary_provider() -> FallbackBinaryProvider {
    let workspace = workspace_root();
    let providers: [BinaryProviderRef; 2] = [
        Arc::new(EnvBinaryProvider::new("KVSTORE_NODE_BIN")),
        Arc::new(BuildBinaryProvider {
            command: BuildCommand::new("cargo").with_args([
                "build",
                "-p",
                "kvstore-node",
                "--bin",
                "kvstore-node",
            ]),
            output_path: PathBuf::from(format!(
                "target/debug/kvstore-node{}",
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

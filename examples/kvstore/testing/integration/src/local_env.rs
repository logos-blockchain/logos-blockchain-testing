use std::collections::HashMap;

use testing_framework_core::scenario::{DynError, StartNodeOptions};
use testing_framework_runner_local::{
    LocalBinaryApp, LocalNodePorts, LocalPeerNode, LocalProcessSpec,
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
        LocalProcessSpec::new("KVSTORE_NODE_BIN", "kvstore-node").with_rust_log("kvstore_node=info")
    }

    fn render_local_config(config: &KvNodeConfig) -> Result<Vec<u8>, DynError> {
        yaml_node_config(config)
    }

    fn http_api_port(config: &KvNodeConfig) -> u16 {
        config.http_port
    }
}

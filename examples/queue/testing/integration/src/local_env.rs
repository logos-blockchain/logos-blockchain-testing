use std::collections::HashMap;

use testing_framework_core::scenario::{DynError, StartNodeOptions};
use testing_framework_runner_local::{
    LocalBinaryApp, LocalNodePorts, LocalPeerNode, LocalProcessSpec,
    build_local_cluster_node_config, yaml_node_config,
};

use crate::{QueueEnv, QueueNodeConfig};

impl LocalBinaryApp for QueueEnv {
    fn initial_node_name_prefix() -> &'static str {
        "queue-node"
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
        LocalProcessSpec::new("QUEUE_NODE_BIN", "queue-node").with_rust_log("queue_node=info")
    }

    fn render_local_config(config: &QueueNodeConfig) -> Result<Vec<u8>, DynError> {
        yaml_node_config(config)
    }

    fn http_api_port(config: &QueueNodeConfig) -> u16 {
        config.http_port
    }
}

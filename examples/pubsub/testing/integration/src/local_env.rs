use std::collections::HashMap;

use testing_framework_core::scenario::{DynError, StartNodeOptions};
use testing_framework_runner_local::{
    LocalBinaryApp, LocalNodePorts, LocalPeerNode, LocalProcessSpec,
    build_local_cluster_node_config, yaml_node_config,
};

use crate::{PubSubEnv, PubSubNodeConfig};

impl LocalBinaryApp for PubSubEnv {
    fn initial_node_name_prefix() -> &'static str {
        "pubsub-node"
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
        LocalProcessSpec::new("PUBSUB_NODE_BIN").with_rust_log("pubsub_node=info")
    }

    fn render_local_config(config: &PubSubNodeConfig) -> Result<Vec<u8>, DynError> {
        yaml_node_config(config)
    }

    fn http_api_port(config: &PubSubNodeConfig) -> u16 {
        config.http_port
    }
}

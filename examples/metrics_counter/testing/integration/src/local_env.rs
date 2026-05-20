use std::collections::HashMap;

use testing_framework_core::scenario::{DynError, StartNodeOptions};
use testing_framework_runner_local::{
    LocalBinaryApp, LocalNodePorts, LocalPeerNode, LocalProcessSpec, yaml_node_config,
};

use crate::{MetricsCounterEnv, MetricsCounterNodeConfig};

impl LocalBinaryApp for MetricsCounterEnv {
    fn initial_node_name_prefix() -> &'static str {
        "metrics-counter-node"
    }

    fn build_local_node_config_with_peers(
        _topology: &Self::Deployment,
        index: usize,
        ports: &LocalNodePorts,
        _peers: &[LocalPeerNode],
        _peer_ports_by_name: &HashMap<String, u16>,
        _options: &StartNodeOptions<Self>,
        _template_config: Option<
            &<Self as testing_framework_core::scenario::Application>::NodeConfig,
        >,
    ) -> Result<<Self as testing_framework_core::scenario::Application>::NodeConfig, DynError> {
        Ok(MetricsCounterNodeConfig {
            node_id: index as u64,
            http_port: ports.network_port(),
        })
    }

    fn local_process_spec() -> LocalProcessSpec {
        LocalProcessSpec::new("METRICS_COUNTER_NODE_BIN").with_rust_log("metrics_counter_node=info")
    }

    fn render_local_config(config: &MetricsCounterNodeConfig) -> Result<Vec<u8>, DynError> {
        yaml_node_config(config)
    }

    fn http_api_port(config: &MetricsCounterNodeConfig) -> u16 {
        config.http_port
    }
}

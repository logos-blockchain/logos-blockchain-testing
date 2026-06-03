use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
};

use testing_framework_core::scenario::{DynError, StartNodeOptions};
use testing_framework_runner_local::{
    LocalDeployerEnv, LocalNodePorts, LocalPeerNode, LocalProcessSpec, NodeEndpointPort,
    NodeEndpoints, build_local_cluster_node_config, env::Node, text_node_config,
};

use crate::{CLUSTER_PORT_KEY, NatsEnv, NatsNodeConfig, render_nats_config};

impl LocalDeployerEnv for NatsEnv {
    fn initial_node_name_prefix() -> &'static str {
        "nats-node"
    }

    fn initial_local_port_names() -> &'static [&'static str] {
        &["client", "monitor"]
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

    fn local_process_spec() -> Option<LocalProcessSpec> {
        Some(LocalProcessSpec::new("NATS_SERVER_BIN").with_config_file("nats.conf", "-c"))
    }

    fn render_local_config(config: &NatsNodeConfig) -> Result<Vec<u8>, DynError> {
        Ok(text_node_config(render_nats_config(config)))
    }

    fn node_endpoints(config: &NatsNodeConfig) -> Result<NodeEndpoints, DynError> {
        let mut endpoints = NodeEndpoints {
            api: SocketAddr::from((Ipv4Addr::LOCALHOST, config.monitor_port)),
            extra_ports: HashMap::new(),
        };

        endpoints.insert_port(NodeEndpointPort::TestingApi, config.client_port);
        endpoints.insert_port(
            NodeEndpointPort::Custom(CLUSTER_PORT_KEY.to_owned()),
            config.cluster_port,
        );

        Ok(endpoints)
    }

    fn node_peer_port(node: &Node<Self>) -> u16 {
        node.endpoints()
            .port(&NodeEndpointPort::Custom(CLUSTER_PORT_KEY.to_owned()))
            .unwrap_or_else(|| node.config().cluster_port)
    }
}

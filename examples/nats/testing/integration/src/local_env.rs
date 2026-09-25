use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
};

use testing_framework_core::scenario::DynError;
use testing_framework_runner_local::{
    BuiltNodeConfig, LocalBuildContext, LocalDeployerEnv, LocalProcessSpec, NodeEndpointPort,
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

    fn build_node_config(
        context: LocalBuildContext<'_, Self>,
    ) -> Result<BuiltNodeConfig<NatsNodeConfig>, DynError> {
        Ok(BuiltNodeConfig {
            config: build_local_cluster_node_config::<Self>(
                context.index,
                context.ports,
                context.peers,
            )?,
            network_port: context.ports.network_port(),
        })
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

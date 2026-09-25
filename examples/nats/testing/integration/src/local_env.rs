use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
};

use testing_framework_core::scenario::DynError;
use testing_framework_runner_local::{
    LocalBuildContext, LocalDeployerEnv, LocalProcessSpec, NodeEndpointPort, NodeEndpoints,
    PreparedNode, build_local_cluster_node_config, text_node_config,
};

use crate::{CLUSTER_PORT_KEY, NatsEnv, NatsNodeConfig, render_nats_config};

impl LocalDeployerEnv for NatsEnv {
    fn build_node_config(
        context: LocalBuildContext<'_, Self>,
    ) -> Result<PreparedNode<NatsNodeConfig>, DynError> {
        context.ports.allocate("client")?;
        context.ports.allocate("monitor")?;

        Ok(PreparedNode {
            name: format!("nats-node-{}", context.index),
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
}

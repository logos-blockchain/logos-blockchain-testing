use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
    path::Path,
};

use testing_framework_core::scenario::DynError;
use testing_framework_runner_local::{
    LaunchSpec, LocalBuildContext, LocalDeployerEnv, LocalProcessSpec, NodeEndpointPort,
    NodeEndpoints, PreparedNode, build_local_cluster_node_config, text_config_launch_spec,
};

use crate::{CLUSTER_PORT_KEY, NatsEnv, NatsNodeConfig, render_nats_config};

#[async_trait::async_trait]
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

    async fn build_launch_spec(
        config: &NatsNodeConfig,
        _dir: &Path,
        _label: &str,
    ) -> Result<LaunchSpec, DynError> {
        let spec = LocalProcessSpec::new("NATS_SERVER_BIN").with_config_file("nats.conf", "-c");
        text_config_launch_spec(render_nats_config(config), &spec).await
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

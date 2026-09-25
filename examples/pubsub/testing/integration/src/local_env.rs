use testing_framework_core::scenario::DynError;
use testing_framework_runner_local::{
    LocalBinaryApp, LocalBuildContext, LocalProcessSpec, PreparedNode,
    build_local_cluster_node_config, yaml_node_config,
};

use crate::{PubSubEnv, PubSubNodeConfig};

impl LocalBinaryApp for PubSubEnv {
    fn build_node_config(
        context: LocalBuildContext<'_, Self>,
    ) -> Result<PreparedNode<PubSubNodeConfig>, DynError> {
        let config =
            build_local_cluster_node_config::<Self>(context.index, context.ports, context.peers)?;
        Ok(PreparedNode {
            name: format!("pubsub-node-{}", context.index),
            config,
            network_port: context.ports.network_port(),
        })
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

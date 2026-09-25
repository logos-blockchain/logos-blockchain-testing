use testing_framework_core::scenario::DynError;
use testing_framework_runner_local::{
    LocalBinaryApp, LocalBuildContext, LocalProcessSpec, build_local_cluster_node_config,
    yaml_node_config,
};

use crate::{PubSubEnv, PubSubNodeConfig};

impl LocalBinaryApp for PubSubEnv {
    fn initial_node_name_prefix() -> &'static str {
        "pubsub-node"
    }

    fn build_node_config(
        context: LocalBuildContext<'_, Self>,
    ) -> Result<PubSubNodeConfig, DynError> {
        build_local_cluster_node_config::<Self>(context.index, context.ports, context.peers)
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

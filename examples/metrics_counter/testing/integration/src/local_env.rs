use testing_framework_core::scenario::DynError;
use testing_framework_runner_local::{
    LocalBinaryApp, LocalBuildContext, LocalProcessSpec, yaml_node_config,
};

use crate::{MetricsCounterEnv, MetricsCounterNodeConfig};

impl LocalBinaryApp for MetricsCounterEnv {
    fn initial_node_name_prefix() -> &'static str {
        "metrics-counter-node"
    }

    fn build_node_config(
        context: LocalBuildContext<'_, Self>,
    ) -> Result<MetricsCounterNodeConfig, DynError> {
        Ok(MetricsCounterNodeConfig {
            node_id: context.index as u64,
            http_port: context.ports.network_port(),
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

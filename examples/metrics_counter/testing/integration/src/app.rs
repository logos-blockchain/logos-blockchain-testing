use async_trait::async_trait;
use metrics_counter_node::MetricsCounterHttpClient;
use serde::{Deserialize, Serialize};
use testing_framework_core::{
    cfgsync::{StaticNodeConfigProvider, serialize_yaml_config},
    scenario::{Application, DynError, NodeAccess},
};

pub type MetricsCounterTopology = testing_framework_core::topology::ClusterTopology;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MetricsCounterNodeConfig {
    pub node_id: u64,
    pub http_port: u16,
}

pub struct MetricsCounterEnv;

#[async_trait]
impl Application for MetricsCounterEnv {
    type Deployment = MetricsCounterTopology;
    type NodeClient = MetricsCounterHttpClient;
    type NodeConfig = MetricsCounterNodeConfig;
    fn build_node_client(access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        Ok(MetricsCounterHttpClient::new(access.api_base_url()?))
    }

    fn node_readiness_path() -> &'static str {
        "/health/ready"
    }
}

impl StaticNodeConfigProvider for MetricsCounterEnv {
    type Error = serde_yaml::Error;

    fn build_node_config(
        _deployment: &Self::Deployment,
        node_index: usize,
    ) -> Result<Self::NodeConfig, Self::Error> {
        Ok(MetricsCounterNodeConfig {
            node_id: node_index as u64,
            http_port: 8080,
        })
    }

    fn serialize_node_config(config: &Self::NodeConfig) -> Result<String, Self::Error> {
        serialize_yaml_config(config)
    }
}

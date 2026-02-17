use std::error::Error;

use thiserror::Error;

pub type DynCfgsyncError = Box<dyn Error + Send + Sync + 'static>;

#[derive(Debug, Clone)]
pub struct CfgsyncNodeConfig {
    pub identifier: String,
    pub config_yaml: String,
}

pub trait CfgsyncEnv {
    type Deployment;
    type Node;
    type NodeConfig;
    type Error: Error + Send + Sync + 'static;

    fn nodes(deployment: &Self::Deployment) -> &[Self::Node];

    fn node_identifier(index: usize, node: &Self::Node) -> String;

    fn build_node_config(
        deployment: &Self::Deployment,
        node: &Self::Node,
    ) -> Result<Self::NodeConfig, Self::Error>;

    fn rewrite_for_hostnames(
        deployment: &Self::Deployment,
        node_index: usize,
        hostnames: &[String],
        config: &mut Self::NodeConfig,
    ) -> Result<(), Self::Error>;

    fn serialize_node_config(config: &Self::NodeConfig) -> Result<String, Self::Error>;
}

#[derive(Debug, Error)]
pub enum BuildCfgsyncNodesError {
    #[error("cfgsync hostnames mismatch (nodes={nodes}, hostnames={hostnames})")]
    HostnameCountMismatch { nodes: usize, hostnames: usize },
    #[error("cfgsync adapter failed: {source}")]
    Adapter {
        #[source]
        source: DynCfgsyncError,
    },
}

fn adapter_error<E>(source: E) -> BuildCfgsyncNodesError
where
    E: Error + Send + Sync + 'static,
{
    BuildCfgsyncNodesError::Adapter {
        source: Box::new(source),
    }
}

pub fn build_cfgsync_node_configs<E: CfgsyncEnv>(
    deployment: &E::Deployment,
    hostnames: &[String],
) -> Result<Vec<CfgsyncNodeConfig>, BuildCfgsyncNodesError> {
    let nodes = E::nodes(deployment);
    if nodes.len() != hostnames.len() {
        return Err(BuildCfgsyncNodesError::HostnameCountMismatch {
            nodes: nodes.len(),
            hostnames: hostnames.len(),
        });
    }

    let mut output = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let mut node_config = E::build_node_config(deployment, node).map_err(adapter_error)?;
        E::rewrite_for_hostnames(deployment, index, hostnames, &mut node_config)
            .map_err(adapter_error)?;
        let config_yaml = E::serialize_node_config(&node_config).map_err(adapter_error)?;

        output.push(CfgsyncNodeConfig {
            identifier: E::node_identifier(index, node),
            config_yaml,
        });
    }

    Ok(output)
}

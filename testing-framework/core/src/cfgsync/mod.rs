use std::error::Error;

pub use cfgsync_adapter::*;
use cfgsync_artifacts::{ArtifactFile, ArtifactSet};
use thiserror::Error;

#[doc(hidden)]
pub type DynCfgsyncError = Box<dyn Error + Send + Sync + 'static>;

#[doc(hidden)]
pub trait StaticArtifactRenderer {
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

#[doc(hidden)]
pub use StaticArtifactRenderer as CfgsyncEnv;

#[derive(Debug, Error)]
pub enum BuildStaticArtifactsError {
    #[error("cfgsync hostnames mismatch (nodes={nodes}, hostnames={hostnames})")]
    HostnameCountMismatch { nodes: usize, hostnames: usize },
    #[error("cfgsync adapter failed: {source}")]
    Adapter {
        #[source]
        source: DynCfgsyncError,
    },
}

fn adapter_error<E>(source: E) -> BuildStaticArtifactsError
where
    E: Error + Send + Sync + 'static,
{
    BuildStaticArtifactsError::Adapter {
        source: Box::new(source),
    }
}

pub fn build_static_artifacts<E: StaticArtifactRenderer>(
    deployment: &E::Deployment,
    hostnames: &[String],
) -> Result<cfgsync_adapter::MaterializedArtifacts, BuildStaticArtifactsError> {
    let nodes = E::nodes(deployment);

    if nodes.len() != hostnames.len() {
        return Err(BuildStaticArtifactsError::HostnameCountMismatch {
            nodes: nodes.len(),
            hostnames: hostnames.len(),
        });
    }

    let mut output = std::collections::HashMap::with_capacity(nodes.len());

    for (index, node) in nodes.iter().enumerate() {
        let mut node_config = E::build_node_config(deployment, node).map_err(adapter_error)?;
        E::rewrite_for_hostnames(deployment, index, hostnames, &mut node_config)
            .map_err(adapter_error)?;
        let config_yaml = E::serialize_node_config(&node_config).map_err(adapter_error)?;

        output.insert(
            E::node_identifier(index, node),
            ArtifactSet::new(vec![ArtifactFile::new(
                "/config.yaml".to_string(),
                config_yaml.clone(),
            )]),
        );
    }

    Ok(cfgsync_adapter::MaterializedArtifacts::from_nodes(output))
}

#[doc(hidden)]
pub use build_static_artifacts as build_cfgsync_node_catalog;

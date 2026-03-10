use std::error::Error;

use cfgsync_artifacts::ArtifactFile;
use thiserror::Error;

use crate::{NodeArtifacts, NodeArtifactsCatalog};

/// Adapter contract for converting an application deployment model into
/// node-specific serialized config payloads.
pub trait DeploymentAdapter {
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

/// High-level failures while building adapter output for cfgsync.
#[derive(Debug, Error)]
pub enum BuildCfgsyncNodesError {
    #[error("cfgsync hostnames mismatch (nodes={nodes}, hostnames={hostnames})")]
    HostnameCountMismatch { nodes: usize, hostnames: usize },
    #[error("cfgsync adapter failed: {source}")]
    Adapter {
        #[source]
        source: super::DynCfgsyncError,
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

/// Builds cfgsync node configs for a deployment by:
/// 1) validating hostname count,
/// 2) building each node config,
/// 3) rewriting host references,
/// 4) serializing each node payload.
pub fn build_cfgsync_node_configs<E: DeploymentAdapter>(
    deployment: &E::Deployment,
    hostnames: &[String],
) -> Result<Vec<NodeArtifacts>, BuildCfgsyncNodesError> {
    Ok(build_node_artifact_catalog::<E>(deployment, hostnames)?.into_nodes())
}

/// Builds cfgsync node configs and indexes them by stable identifier.
pub fn build_node_artifact_catalog<E: DeploymentAdapter>(
    deployment: &E::Deployment,
    hostnames: &[String],
) -> Result<NodeArtifactsCatalog, BuildCfgsyncNodesError> {
    let nodes = E::nodes(deployment);
    ensure_hostname_count(nodes.len(), hostnames.len())?;

    let mut output = Vec::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        output.push(build_node_entry::<E>(deployment, node, index, hostnames)?);
    }

    Ok(NodeArtifactsCatalog::new(output))
}

fn ensure_hostname_count(nodes: usize, hostnames: usize) -> Result<(), BuildCfgsyncNodesError> {
    if nodes != hostnames {
        return Err(BuildCfgsyncNodesError::HostnameCountMismatch { nodes, hostnames });
    }

    Ok(())
}

fn build_node_entry<E: DeploymentAdapter>(
    deployment: &E::Deployment,
    node: &E::Node,
    index: usize,
    hostnames: &[String],
) -> Result<NodeArtifacts, BuildCfgsyncNodesError> {
    let node_config = build_rewritten_node_config::<E>(deployment, node, index, hostnames)?;
    let config_yaml = E::serialize_node_config(&node_config).map_err(adapter_error)?;

    Ok(NodeArtifacts {
        identifier: E::node_identifier(index, node),
        files: vec![ArtifactFile::new("/config.yaml", &config_yaml)],
    })
}

fn build_rewritten_node_config<E: DeploymentAdapter>(
    deployment: &E::Deployment,
    node: &E::Node,
    index: usize,
    hostnames: &[String],
) -> Result<E::NodeConfig, BuildCfgsyncNodesError> {
    let mut node_config = E::build_node_config(deployment, node).map_err(adapter_error)?;
    E::rewrite_for_hostnames(deployment, index, hostnames, &mut node_config)
        .map_err(adapter_error)?;

    Ok(node_config)
}

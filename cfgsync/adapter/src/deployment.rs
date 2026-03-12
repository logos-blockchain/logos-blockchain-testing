use std::{collections::HashMap, error::Error};

use cfgsync_artifacts::{ArtifactFile, ArtifactSet};
use thiserror::Error;

use crate::MaterializedArtifacts;

/// Adapter contract for converting an application deployment model into
/// node-specific serialized config payloads.
pub trait DeploymentAdapter {
    /// Application-specific deployment model that cfgsync renders from.
    type Deployment;
    /// One node entry inside the deployment model.
    type Node;
    /// In-memory node config type produced before serialization.
    type NodeConfig;
    /// Adapter-specific failure type raised while building or rewriting
    /// configs.
    type Error: Error + Send + Sync + 'static;

    /// Returns the ordered node list that cfgsync should materialize.
    fn nodes(deployment: &Self::Deployment) -> &[Self::Node];

    /// Returns the stable identifier cfgsync should use for this node.
    fn node_identifier(index: usize, node: &Self::Node) -> String;

    /// Builds the initial in-memory config for one node before hostname
    /// rewriting is applied.
    fn build_node_config(
        deployment: &Self::Deployment,
        node: &Self::Node,
    ) -> Result<Self::NodeConfig, Self::Error>;

    /// Rewrites any inter-node references so the config can be served through
    /// cfgsync using the provided hostnames.
    fn rewrite_for_hostnames(
        deployment: &Self::Deployment,
        node_index: usize,
        hostnames: &[String],
        config: &mut Self::NodeConfig,
    ) -> Result<(), Self::Error>;

    /// Serializes the final node config into the file content cfgsync should
    /// deliver.
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

/// Builds materialized cfgsync artifacts for a deployment by:
/// 1) validating hostname count,
/// 2) building each node config,
/// 3) rewriting host references,
/// 4) serializing each node payload.
pub fn build_materialized_artifacts<E: DeploymentAdapter>(
    deployment: &E::Deployment,
    hostnames: &[String],
) -> Result<MaterializedArtifacts, BuildCfgsyncNodesError> {
    let nodes = E::nodes(deployment);
    ensure_hostname_count(nodes.len(), hostnames.len())?;

    let mut output = HashMap::with_capacity(nodes.len());
    for (index, node) in nodes.iter().enumerate() {
        let (identifier, artifacts) = build_node_entry::<E>(deployment, node, index, hostnames)?;
        output.insert(identifier, artifacts);
    }

    Ok(MaterializedArtifacts::from_nodes(output))
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
) -> Result<(String, ArtifactSet), BuildCfgsyncNodesError> {
    let node_config = build_rewritten_node_config::<E>(deployment, node, index, hostnames)?;
    let config_yaml = E::serialize_node_config(&node_config).map_err(adapter_error)?;

    Ok((
        E::node_identifier(index, node),
        ArtifactSet::new(vec![ArtifactFile::new("/config.yaml", &config_yaml)]),
    ))
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

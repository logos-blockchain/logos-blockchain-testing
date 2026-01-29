use std::collections::HashSet;

use thiserror::Error;

use crate::{
    nodes::{
        common::node::SpawnNodeError,
        node::{Node, apply_node_config_patch, create_node_config},
    },
    scenario,
    topology::{
        config::{TopologyBuildError, TopologyBuilder, TopologyConfig},
        generation::{GeneratedNodeConfig, find_expected_peer_counts},
        readiness::{NetworkReadiness, ReadinessCheck, ReadinessError},
        utils::multiaddr_port,
    },
};

/// Runtime representation of a spawned topology with running nodes.
pub struct Topology {
    pub(crate) nodes: Vec<Node>,
}

pub type DeployedNodes = Vec<Node>;

#[derive(Debug, Error)]
pub enum SpawnTopologyError {
    #[error(transparent)]
    Build(#[from] TopologyBuildError),
    #[error(transparent)]
    Node(#[from] SpawnNodeError),
    #[error("node config patch failed for node-{index}: {source}")]
    ConfigPatch {
        index: usize,
        source: scenario::DynError,
    },
}

impl Topology {
    pub async fn spawn(config: TopologyConfig) -> Result<Self, SpawnTopologyError> {
        let generated = TopologyBuilder::new(config.clone()).build()?;
        let nodes = Self::spawn_nodes(generated.nodes()).await?;

        Ok(Self { nodes })
    }

    pub async fn spawn_with_empty_membership(
        config: TopologyConfig,
        ids: &[[u8; 32]],
        blend_ports: &[u16],
    ) -> Result<Self, SpawnTopologyError> {
        let generated = TopologyBuilder::new(config.clone())
            .with_ids(ids.to_vec())
            .with_blend_ports(blend_ports.to_vec())
            .build()?;

        let nodes = Self::spawn_nodes(generated.nodes()).await?;

        Ok(Self { nodes })
    }

    pub(crate) async fn spawn_nodes(
        nodes: &[GeneratedNodeConfig],
    ) -> Result<DeployedNodes, SpawnTopologyError> {
        let mut spawned = Vec::new();
        for node in nodes {
            let mut config = create_node_config(node.general.clone());

            if let Some(patch) = node.config_patch.as_ref() {
                config = apply_node_config_patch(config, patch).map_err(|source| {
                    SpawnTopologyError::ConfigPatch {
                        index: node.index,
                        source,
                    }
                })?;
            }

            let label = format!("node-{}", node.index);
            spawned.push(Node::spawn(config, &label).await?);
        }

        Ok(spawned)
    }

    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub async fn wait_network_ready(&self) -> Result<(), ReadinessError> {
        let listen_ports = self.node_listen_ports();
        if listen_ports.len() <= 1 {
            return Ok(());
        }

        let initial_peer_ports = self.node_initial_peer_ports();
        let expected_peer_counts = find_expected_peer_counts(&listen_ports, &initial_peer_ports);
        let labels = self.node_labels();

        let check = NetworkReadiness {
            topology: self,
            expected_peer_counts: &expected_peer_counts,
            labels: &labels,
        };

        check.wait().await?;
        Ok(())
    }

    fn node_listen_ports(&self) -> Vec<u16> {
        self.nodes
            .iter()
            .map(|node| node.config().user.network.backend.swarm.port)
            .collect()
    }

    fn node_initial_peer_ports(&self) -> Vec<HashSet<u16>> {
        self.nodes
            .iter()
            .map(|node| {
                node.config()
                    .user
                    .network
                    .backend
                    .initial_peers
                    .iter()
                    .filter_map(multiaddr_port)
                    .collect::<HashSet<u16>>()
            })
            .collect()
    }

    fn node_labels(&self) -> Vec<String> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(idx, node)| {
                format!(
                    "node#{idx}@{}",
                    node.config().user.network.backend.swarm.port
                )
            })
            .collect()
    }
}

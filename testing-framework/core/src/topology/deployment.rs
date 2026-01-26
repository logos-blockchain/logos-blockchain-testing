use std::collections::HashSet;

use thiserror::Error;

use crate::{
    nodes::{
        common::node::SpawnNodeError,
        node::{Node, create_node_config},
    },
    topology::{
        config::{TopologyBuildError, TopologyBuilder, TopologyConfig},
        configs::GeneralConfig,
        generation::find_expected_peer_counts,
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
}

impl Topology {
    pub async fn spawn(config: TopologyConfig) -> Result<Self, SpawnTopologyError> {
        let generated = TopologyBuilder::new(config.clone()).build()?;
        let n_nodes = config.n_nodes;
        let node_configs = generated
            .iter()
            .map(|node| node.general.clone())
            .collect::<Vec<_>>();

        let nodes = Self::spawn_nodes(node_configs, n_nodes).await?;

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

        let node_configs = generated
            .iter()
            .map(|node| node.general.clone())
            .collect::<Vec<_>>();

        let nodes = Self::spawn_nodes(node_configs, config.n_nodes).await?;

        Ok(Self { nodes })
    }

    pub(crate) async fn spawn_nodes(
        config: Vec<GeneralConfig>,
        n_nodes: usize,
    ) -> Result<DeployedNodes, SpawnTopologyError> {
        let mut nodes = Vec::new();
        for i in 0..n_nodes {
            let config = create_node_config(config[i].clone());
            let label = format!("node-{i}");
            nodes.push(Node::spawn(config, &label).await?);
        }

        Ok(nodes)
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
            .map(|node| node.config().network.backend.swarm.port)
            .collect()
    }

    fn node_initial_peer_ports(&self) -> Vec<HashSet<u16>> {
        self.nodes
            .iter()
            .map(|node| {
                node.config()
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
            .map(|(idx, node)| format!("node#{idx}@{}", node.config().network.backend.swarm.port))
            .collect()
    }
}

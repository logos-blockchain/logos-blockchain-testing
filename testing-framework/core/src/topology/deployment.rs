use std::collections::HashSet;

use crate::{
    nodes::node::Node,
    topology::{
        generation::find_expected_peer_counts,
        readiness::{NetworkReadiness, ReadinessCheck, ReadinessError},
        utils::multiaddr_port,
    },
};

/// Runtime representation of a spawned topology with running nodes.
pub struct Topology {
    pub(crate) nodes: Vec<Node>,
}

impl Topology {
    /// Construct a topology from already-spawned nodes.
    #[must_use]
    pub fn from_nodes(nodes: Vec<Node>) -> Self {
        Self { nodes }
    }

    #[must_use]
    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    #[must_use]
    pub fn into_nodes(self) -> Vec<Node> {
        self.nodes
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

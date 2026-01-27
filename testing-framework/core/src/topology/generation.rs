use std::{collections::HashSet, time::Duration};

use reqwest::{Client, Url};

use crate::topology::{
    config::{NodeConfigPatch, TopologyConfig},
    configs::{GeneralConfig, wallet::WalletAccount},
    deployment::{SpawnTopologyError, Topology},
    readiness::{HttpNetworkReadiness, ReadinessCheck, ReadinessError},
};

/// Fully generated configuration for an individual node.
#[derive(Clone)]
pub struct GeneratedNodeConfig {
    pub index: usize,
    pub id: [u8; 32],
    pub general: GeneralConfig,
    pub blend_port: u16,
    pub config_patch: Option<NodeConfigPatch>,
}

impl GeneratedNodeConfig {
    #[must_use]
    /// Zero-based index within the topology.
    pub const fn index(&self) -> usize {
        self.index
    }

    #[must_use]
    pub const fn network_port(&self) -> u16 {
        self.general.network_config.backend.swarm.port
    }

    #[must_use]
    pub const fn api_port(&self) -> u16 {
        self.general.api_config.address.port()
    }

    #[must_use]
    pub const fn testing_http_port(&self) -> u16 {
        self.general.api_config.testing_http_address.port()
    }
}

/// Collection of generated node configs and helpers to spawn or probe the
/// stack.
#[derive(Clone)]
pub struct GeneratedTopology {
    pub(crate) config: TopologyConfig,
    pub(crate) nodes: Vec<GeneratedNodeConfig>,
}

impl GeneratedTopology {
    #[must_use]
    /// Underlying configuration used to derive the generated nodes.
    pub const fn config(&self) -> &TopologyConfig {
        &self.config
    }

    #[must_use]
    /// All node configs.
    pub fn nodes(&self) -> &[GeneratedNodeConfig] {
        &self.nodes
    }

    /// Iterator over all node configs in topology order.
    pub fn iter(&self) -> impl Iterator<Item = &GeneratedNodeConfig> {
        self.nodes.iter()
    }

    #[must_use]
    /// Slot duration from the first node (assumes homogeneous configs).
    pub fn slot_duration(&self) -> Option<Duration> {
        self.nodes
            .first()
            .map(|node| node.general.time_config.slot_duration)
    }

    #[must_use]
    /// Wallet accounts configured for this topology.
    pub fn wallet_accounts(&self) -> &[WalletAccount] {
        &self.config.wallet_config.accounts
    }

    pub async fn spawn_local(&self) -> Result<Topology, SpawnTopologyError> {
        let nodes = Topology::spawn_nodes(self.nodes()).await?;

        Ok(Topology { nodes })
    }

    pub async fn wait_remote_readiness(
        &self,
        // Node endpoints
        node_endpoints: &[Url],
    ) -> Result<(), ReadinessError> {
        let total_nodes = self.nodes.len();
        if total_nodes == 0 {
            return Ok(());
        }

        let labels = self.labels();
        let client = Client::new();

        let endpoints = collect_node_endpoints(self, node_endpoints, total_nodes);

        wait_for_network_readiness(self, &client, &endpoints, &labels).await
    }

    fn listen_ports(&self) -> Vec<u16> {
        self.nodes
            .iter()
            .map(|node| node.general.network_config.backend.swarm.port)
            .collect()
    }

    fn initial_peer_ports(&self) -> Vec<HashSet<u16>> {
        self.nodes
            .iter()
            .map(|node| {
                node.general
                    .network_config
                    .backend
                    .initial_peers
                    .iter()
                    .filter_map(crate::topology::utils::multiaddr_port)
                    .collect::<HashSet<u16>>()
            })
            .collect()
    }

    fn labels(&self) -> Vec<String> {
        self.nodes
            .iter()
            .enumerate()
            .map(|(idx, node)| {
                format!(
                    "node#{idx}@{}",
                    node.general.network_config.backend.swarm.port
                )
            })
            .collect()
    }
}

fn collect_node_endpoints(
    topology: &GeneratedTopology,
    node_endpoints: &[Url],
    total_nodes: usize,
) -> Vec<Url> {
    assert_eq!(
        topology.nodes.len(),
        node_endpoints.len(),
        "node endpoints must match topology"
    );

    let mut endpoints = Vec::with_capacity(total_nodes);
    endpoints.extend_from_slice(node_endpoints);
    endpoints
}

async fn wait_for_network_readiness(
    topology: &GeneratedTopology,
    client: &Client,
    endpoints: &[Url],
    labels: &[String],
) -> Result<(), ReadinessError> {
    if endpoints.len() <= 1 {
        return Ok(());
    }

    let listen_ports = topology.listen_ports();
    let initial_peer_ports = topology.initial_peer_ports();
    let expected_peer_counts =
        crate::topology::generation::find_expected_peer_counts(&listen_ports, &initial_peer_ports);

    let network_check = HttpNetworkReadiness {
        client,
        endpoints,
        expected_peer_counts: &expected_peer_counts,
        labels,
    };

    network_check.wait().await
}

pub fn find_expected_peer_counts(
    listen_ports: &[u16],
    initial_peer_ports: &[HashSet<u16>],
) -> Vec<usize> {
    let mut expected: Vec<HashSet<usize>> = vec![HashSet::new(); initial_peer_ports.len()];

    for (idx, ports) in initial_peer_ports.iter().enumerate() {
        for port in ports {
            let Some(peer_idx) = listen_ports.iter().position(|p| p == port) else {
                continue;
            };

            if peer_idx == idx {
                continue;
            }

            expected[idx].insert(peer_idx);
            expected[peer_idx].insert(idx);
        }
    }

    expected.into_iter().map(|set| set.len()).collect()
}

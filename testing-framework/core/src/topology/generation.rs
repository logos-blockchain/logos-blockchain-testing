use std::{collections::HashSet, time::Duration};

use reqwest::{Client, Url};

use crate::topology::{
    config::TopologyConfig,
    configs::{GeneralConfig, wallet::WalletAccount},
    deployment::{SpawnTopologyError, Topology},
    readiness::{HttpMembershipReadiness, HttpNetworkReadiness, ReadinessCheck, ReadinessError},
};

/// Node role within the generated topology.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeRole {
    Validator,
    Executor,
}

/// Fully generated configuration for an individual node.
#[derive(Clone)]
pub struct GeneratedNodeConfig {
    pub role: NodeRole,
    pub index: usize,
    pub id: [u8; 32],
    pub general: GeneralConfig,
    pub da_port: u16,
    pub blend_port: u16,
}

impl GeneratedNodeConfig {
    #[must_use]
    /// Logical role of the node.
    pub const fn role(&self) -> NodeRole {
        self.role
    }

    #[must_use]
    /// Zero-based index within its role group.
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
    pub(crate) validators: Vec<GeneratedNodeConfig>,
    pub(crate) executors: Vec<GeneratedNodeConfig>,
}

impl GeneratedTopology {
    #[must_use]
    /// Underlying configuration used to derive the generated nodes.
    pub const fn config(&self) -> &TopologyConfig {
        &self.config
    }

    #[must_use]
    /// All validator configs.
    pub fn validators(&self) -> &[GeneratedNodeConfig] {
        &self.validators
    }

    #[must_use]
    /// All executor configs.
    pub fn executors(&self) -> &[GeneratedNodeConfig] {
        &self.executors
    }

    /// Iterator over all node configs in role order.
    pub fn nodes(&self) -> impl Iterator<Item = &GeneratedNodeConfig> {
        self.validators.iter().chain(self.executors.iter())
    }

    #[must_use]
    /// Slot duration from the first node (assumes homogeneous configs).
    pub fn slot_duration(&self) -> Option<Duration> {
        self.validators
            .first()
            .map(|node| node.general.time_config.slot_duration)
    }

    #[must_use]
    /// Wallet accounts configured for this topology.
    pub fn wallet_accounts(&self) -> &[WalletAccount] {
        &self.config.wallet_config.accounts
    }

    pub async fn spawn_local(&self) -> Result<Topology, SpawnTopologyError> {
        let configs = self
            .nodes()
            .map(|node| node.general.clone())
            .collect::<Vec<_>>();

        let (validators, executors) = Topology::spawn_validators_executors(
            configs,
            self.config.n_validators,
            self.config.n_executors,
        )
        .await?;

        Ok(Topology {
            validators,
            executors,
        })
    }

    pub async fn wait_remote_readiness(
        &self,
        // Node endpoints
        validator_endpoints: &[Url],
        executor_endpoints: &[Url],

        // Membership endpoints
        validator_membership_endpoints: Option<&[Url]>,
        executor_membership_endpoints: Option<&[Url]>,
    ) -> Result<(), ReadinessError> {
        let total_nodes = self.validators.len() + self.executors.len();
        if total_nodes == 0 {
            return Ok(());
        }

        let labels = self.labels();
        let client = Client::new();

        let endpoints =
            collect_node_endpoints(self, validator_endpoints, executor_endpoints, total_nodes);

        wait_for_network_readiness(self, &client, &endpoints, &labels).await?;

        if validator_membership_endpoints.is_none() && executor_membership_endpoints.is_none() {
            return Ok(());
        }

        let membership_endpoints = collect_membership_endpoints(
            self,
            total_nodes,
            validator_membership_endpoints,
            executor_membership_endpoints,
        );

        let membership_check = HttpMembershipReadiness {
            client: &client,
            endpoints: &membership_endpoints,
            session: nomos_core::sdp::SessionNumber::from(0u64),
            labels: &labels,
            expect_non_empty: true,
        };

        membership_check.wait().await
    }

    fn listen_ports(&self) -> Vec<u16> {
        self.validators
            .iter()
            .map(|node| node.general.network_config.backend.swarm.port)
            .chain(
                self.executors
                    .iter()
                    .map(|node| node.general.network_config.backend.swarm.port),
            )
            .collect()
    }

    fn initial_peer_ports(&self) -> Vec<HashSet<u16>> {
        self.validators
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
            .chain(self.executors.iter().map(|node| {
                node.general
                    .network_config
                    .backend
                    .initial_peers
                    .iter()
                    .filter_map(crate::topology::utils::multiaddr_port)
                    .collect::<HashSet<u16>>()
            }))
            .collect()
    }

    fn labels(&self) -> Vec<String> {
        self.validators
            .iter()
            .enumerate()
            .map(|(idx, node)| {
                format!(
                    "validator#{idx}@{}",
                    node.general.network_config.backend.swarm.port
                )
            })
            .chain(self.executors.iter().enumerate().map(|(idx, node)| {
                format!(
                    "executor#{idx}@{}",
                    node.general.network_config.backend.swarm.port
                )
            }))
            .collect()
    }
}

fn collect_node_endpoints(
    topology: &GeneratedTopology,
    validator_endpoints: &[Url],
    executor_endpoints: &[Url],
    total_nodes: usize,
) -> Vec<Url> {
    assert_eq!(
        topology.validators.len(),
        validator_endpoints.len(),
        "validator endpoints must match topology"
    );
    assert_eq!(
        topology.executors.len(),
        executor_endpoints.len(),
        "executor endpoints must match topology"
    );

    let mut endpoints = Vec::with_capacity(total_nodes);
    endpoints.extend_from_slice(validator_endpoints);
    endpoints.extend_from_slice(executor_endpoints);
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

fn collect_membership_endpoints(
    topology: &GeneratedTopology,
    total_nodes: usize,
    validator_membership_endpoints: Option<&[Url]>,
    executor_membership_endpoints: Option<&[Url]>,
) -> Vec<Url> {
    let mut membership_endpoints = Vec::with_capacity(total_nodes);

    membership_endpoints.extend(collect_role_membership_endpoints(
        &topology.validators,
        validator_membership_endpoints,
        "validator membership endpoints must match topology",
    ));
    membership_endpoints.extend(collect_role_membership_endpoints(
        &topology.executors,
        executor_membership_endpoints,
        "executor membership endpoints must match topology",
    ));

    membership_endpoints
}

fn collect_role_membership_endpoints(
    nodes: &[GeneratedNodeConfig],
    membership_endpoints: Option<&[Url]>,
    mismatch_message: &'static str,
) -> Vec<Url> {
    match membership_endpoints {
        Some(urls) => {
            assert_eq!(nodes.len(), urls.len(), "{mismatch_message}");
            urls.to_vec()
        }
        None => nodes
            .iter()
            .map(|node| testing_base_url(node.testing_http_port()))
            .collect(),
    }
}

fn testing_base_url(port: u16) -> Url {
    Url::parse(&format!("http://127.0.0.1:{port}/")).unwrap_or_else(|_| unsafe {
        // Safety: `port` is a valid u16 port.
        std::hint::unreachable_unchecked()
    })
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

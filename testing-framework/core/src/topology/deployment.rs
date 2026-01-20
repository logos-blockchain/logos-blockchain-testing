use std::collections::HashSet;

use nomos_core::sdp::SessionNumber;
use thiserror::Error;

use crate::{
    nodes::{
        common::node::SpawnNodeError,
        executor::{Executor, create_executor_config},
        validator::{Validator, create_validator_config},
    },
    topology::{
        config::{TopologyBuildError, TopologyBuilder, TopologyConfig},
        configs::GeneralConfig,
        generation::find_expected_peer_counts,
        readiness::{
            DaBalancerReadiness, MembershipReadiness, NetworkReadiness, ReadinessCheck,
            ReadinessError,
        },
        utils::multiaddr_port,
    },
};

/// Runtime representation of a spawned topology with running nodes.
pub struct Topology {
    pub(crate) validators: Vec<Validator>,
    pub(crate) executors: Vec<Executor>,
}

pub type DeployedNodes = (Vec<Validator>, Vec<Executor>);

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
        let n_validators = config.n_validators;
        let n_executors = config.n_executors;
        let node_configs = generated
            .nodes()
            .map(|node| node.general.clone())
            .collect::<Vec<_>>();

        let (validators, executors) =
            Self::spawn_validators_executors(node_configs, n_validators, n_executors).await?;

        Ok(Self {
            validators,
            executors,
        })
    }

    pub async fn spawn_with_empty_membership(
        config: TopologyConfig,
        ids: &[[u8; 32]],
        da_ports: &[u16],
        blend_ports: &[u16],
    ) -> Result<Self, SpawnTopologyError> {
        let generated = TopologyBuilder::new(config.clone())
            .with_ids(ids.to_vec())
            .with_da_ports(da_ports.to_vec())
            .with_blend_ports(blend_ports.to_vec())
            .build()?;

        let node_configs = generated
            .nodes()
            .map(|node| node.general.clone())
            .collect::<Vec<_>>();

        let (validators, executors) =
            Self::spawn_validators_executors(node_configs, config.n_validators, config.n_executors)
                .await?;

        Ok(Self {
            validators,
            executors,
        })
    }

    pub(crate) async fn spawn_validators_executors(
        config: Vec<GeneralConfig>,
        n_validators: usize,
        n_executors: usize,
    ) -> Result<DeployedNodes, SpawnTopologyError> {
        let mut validators = Vec::new();
        for i in 0..n_validators {
            let config = create_validator_config(config[i].clone());
            let label = format!("validator-{i}");
            validators.push(Validator::spawn(config, &label).await?);
        }

        let mut executors = Vec::new();
        for i in 0..n_executors {
            let config = create_executor_config(config[n_validators + i].clone());
            let label = format!("executor-{i}");
            executors.push(Executor::spawn(config, &label).await?);
        }

        Ok((validators, executors))
    }

    #[must_use]
    pub fn validators(&self) -> &[Validator] {
        &self.validators
    }

    #[must_use]
    pub fn executors(&self) -> &[Executor] {
        &self.executors
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

    pub async fn wait_da_balancer_ready(&self) -> Result<(), ReadinessError> {
        if self.validators.is_empty() && self.executors.is_empty() {
            return Ok(());
        }

        let labels = self.node_labels();
        let check = DaBalancerReadiness {
            topology: self,
            labels: &labels,
        };

        check.wait().await?;
        Ok(())
    }

    pub async fn wait_membership_ready(&self) -> Result<(), ReadinessError> {
        self.wait_membership_ready_for_session(SessionNumber::from(0u64))
            .await
    }

    pub async fn wait_membership_ready_for_session(
        &self,
        session: SessionNumber,
    ) -> Result<(), ReadinessError> {
        self.wait_membership_assignations(session, true).await
    }

    pub async fn wait_membership_empty_for_session(
        &self,
        session: SessionNumber,
    ) -> Result<(), ReadinessError> {
        self.wait_membership_assignations(session, false).await
    }

    async fn wait_membership_assignations(
        &self,
        session: SessionNumber,
        expect_non_empty: bool,
    ) -> Result<(), ReadinessError> {
        let total_nodes = self.validators.len() + self.executors.len();

        if total_nodes == 0 {
            return Ok(());
        }

        let labels = self.node_labels();
        let check = MembershipReadiness {
            topology: self,
            session,
            labels: &labels,
            expect_non_empty,
        };

        check.wait().await?;
        Ok(())
    }

    fn node_listen_ports(&self) -> Vec<u16> {
        self.validators
            .iter()
            .map(|node| node.config().network.backend.swarm.port)
            .chain(
                self.executors
                    .iter()
                    .map(|node| node.config().network.backend.swarm.port),
            )
            .collect()
    }

    fn node_initial_peer_ports(&self) -> Vec<HashSet<u16>> {
        self.validators
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
            .chain(self.executors.iter().map(|node| {
                node.config()
                    .network
                    .backend
                    .initial_peers
                    .iter()
                    .filter_map(multiaddr_port)
                    .collect::<HashSet<u16>>()
            }))
            .collect()
    }

    fn node_labels(&self) -> Vec<String> {
        self.validators
            .iter()
            .enumerate()
            .map(|(idx, node)| {
                format!(
                    "validator#{idx}@{}",
                    node.config().network.backend.swarm.port
                )
            })
            .chain(self.executors.iter().enumerate().map(|(idx, node)| {
                format!(
                    "executor#{idx}@{}",
                    node.config().network.backend.swarm.port
                )
            }))
            .collect()
    }
}

pub mod clients;
pub mod orchestrator;
pub mod ports;
pub mod readiness;
pub mod setup;

use async_trait::async_trait;
use testing_framework_core::scenario::{
    BlockFeedTask, CleanupGuard, Deployer, ObservabilityCapabilityProvider, RequiresNodeControl,
    Runner, Scenario,
};

use crate::{errors::ComposeRunnerError, lifecycle::cleanup::RunnerCleanup};

/// Docker Compose-based deployer for Logos test scenarios.
#[derive(Clone, Copy)]
pub struct ComposeDeployer {
    readiness_checks: bool,
}

impl Default for ComposeDeployer {
    fn default() -> Self {
        Self::new()
    }
}

impl ComposeDeployer {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            readiness_checks: true,
        }
    }

    #[must_use]
    pub const fn with_readiness(mut self, enabled: bool) -> Self {
        self.readiness_checks = enabled;
        self
    }
}

#[async_trait]
impl<Caps> Deployer<Caps> for ComposeDeployer
where
    Caps: RequiresNodeControl + ObservabilityCapabilityProvider + Send + Sync,
{
    type Error = ComposeRunnerError;

    async fn deploy(&self, scenario: &Scenario<Caps>) -> Result<Runner, Self::Error> {
        orchestrator::DeploymentOrchestrator::new(*self)
            .deploy(scenario)
            .await
    }
}

pub(super) struct ComposeCleanupGuard {
    environment: RunnerCleanup,
    block_feed: Option<BlockFeedTask>,
}

impl ComposeCleanupGuard {
    const fn new(environment: RunnerCleanup, block_feed: BlockFeedTask) -> Self {
        Self {
            environment,
            block_feed: Some(block_feed),
        }
    }
}

impl CleanupGuard for ComposeCleanupGuard {
    fn cleanup(mut self: Box<Self>) {
        if let Some(block_feed) = self.block_feed.take() {
            CleanupGuard::cleanup(Box::new(block_feed));
        }
        CleanupGuard::cleanup(Box::new(self.environment));
    }
}

pub(super) fn make_cleanup_guard(
    environment: RunnerCleanup,
    block_feed: BlockFeedTask,
) -> Box<dyn CleanupGuard> {
    Box::new(ComposeCleanupGuard::new(environment, block_feed))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, net::Ipv4Addr};

    use cfgsync_tf::{
        config::builder::create_node_configs,
        host::{Host, PortOverrides},
    };
    use groth16::Fr;
    use key_management_system_service::keys::ZkPublicKey;
    use nomos_core::{
        mantle::{GenesisTx as GenesisTxTrait, ledger::NoteId},
        sdp::{ProviderId, ServiceType},
    };
    use nomos_ledger::LedgerState;
    use nomos_tracing_service::TracingSettings;
    use testing_framework_core::{
        scenario::ScenarioBuilder,
        topology::{
            generation::{GeneratedNodeConfig, GeneratedTopology, NodeRole as TopologyNodeRole},
            utils::multiaddr_port,
        },
    };

    #[test]
    fn cfgsync_prebuilt_configs_preserve_genesis() {
        let scenario = ScenarioBuilder::topology_with(|t| t.validators(1).executors(1))
            .build()
            .expect("scenario build should succeed");
        let topology = scenario.topology().clone();
        let hosts = hosts_from_topology(&topology);
        let tracing_settings = tracing_settings(&topology);

        let configs = create_node_configs(
            &topology.config().consensus_params,
            &topology.config().da_params,
            &tracing_settings,
            &topology.config().wallet_config,
            Some(topology.nodes().map(|node| node.id).collect()),
            Some(topology.nodes().map(|node| node.da_port).collect()),
            Some(topology.nodes().map(|node| node.blend_port).collect()),
            hosts,
        )
        .expect("cfgsync config generation should succeed");
        let configs_by_identifier: HashMap<_, _> = configs
            .into_iter()
            .map(|(host, config)| (host.identifier, config))
            .collect();

        for node in topology.nodes() {
            let identifier = identifier_for(node.role(), node.index());
            let cfgsync_config = configs_by_identifier
                .get(&identifier)
                .unwrap_or_else(|| panic!("missing cfgsync config for {identifier}"));
            let expected_genesis = &node.general.consensus_config.genesis_tx;
            let actual_genesis = &cfgsync_config.consensus_config.genesis_tx;
            if std::env::var("PRINT_GENESIS").is_ok() {
                println!(
                    "[fingerprint {identifier}] expected={:?}",
                    declaration_fingerprint(expected_genesis)
                );
                println!(
                    "[fingerprint {identifier}] actual={:?}",
                    declaration_fingerprint(actual_genesis)
                );
            }
            assert_eq!(
                expected_genesis.mantle_tx().ledger_tx,
                actual_genesis.mantle_tx().ledger_tx,
                "ledger tx mismatch for {identifier}"
            );
            assert_eq!(
                declaration_fingerprint(expected_genesis),
                declaration_fingerprint(actual_genesis),
                "declaration entries mismatch for {identifier}"
            );
        }
    }

    #[test]
    fn cfgsync_genesis_proofs_verify_against_ledger() {
        let scenario = ScenarioBuilder::topology_with(|t| t.validators(1).executors(1))
            .build()
            .expect("scenario build should succeed");
        let topology = scenario.topology().clone();
        let hosts = hosts_from_topology(&topology);
        let tracing_settings = tracing_settings(&topology);

        let configs = create_node_configs(
            &topology.config().consensus_params,
            &topology.config().da_params,
            &tracing_settings,
            &topology.config().wallet_config,
            Some(topology.nodes().map(|node| node.id).collect()),
            Some(topology.nodes().map(|node| node.da_port).collect()),
            Some(topology.nodes().map(|node| node.blend_port).collect()),
            hosts,
        )
        .expect("cfgsync config generation should succeed");
        let configs_by_identifier: HashMap<_, _> = configs
            .into_iter()
            .map(|(host, config)| (host.identifier, config))
            .collect();

        for node in topology.nodes() {
            let identifier = identifier_for(node.role(), node.index());
            let cfgsync_config = configs_by_identifier
                .get(&identifier)
                .unwrap_or_else(|| panic!("missing cfgsync config for {identifier}"));
            LedgerState::from_genesis_tx::<()>(
                cfgsync_config.consensus_config.genesis_tx.clone(),
                &cfgsync_config.consensus_config.ledger_config,
                Fr::from(0u64),
            )
            .unwrap_or_else(|err| panic!("ledger rejected genesis for {identifier}: {err:?}"));
        }
    }

    #[test]
    fn cfgsync_docker_overrides_produce_valid_genesis() {
        let scenario = ScenarioBuilder::topology_with(|t| t.validators(1).executors(1))
            .build()
            .expect("scenario build should succeed");
        let topology = scenario.topology().clone();
        let tracing_settings = tracing_settings(&topology);
        let hosts = docker_style_hosts(&topology);

        let configs = create_node_configs(
            &topology.config().consensus_params,
            &topology.config().da_params,
            &tracing_settings,
            &topology.config().wallet_config,
            Some(topology.nodes().map(|node| node.id).collect()),
            Some(topology.nodes().map(|node| node.da_port).collect()),
            Some(topology.nodes().map(|node| node.blend_port).collect()),
            hosts,
        )
        .expect("cfgsync config generation should succeed");

        for (host, config) in configs {
            let genesis = &config.consensus_config.genesis_tx;
            LedgerState::from_genesis_tx::<()>(
                genesis.clone(),
                &config.consensus_config.ledger_config,
                Fr::from(0u64),
            )
            .unwrap_or_else(|err| {
                panic!("ledger rejected genesis for {}: {err:?}", host.identifier)
            });
        }
    }

    #[test]
    fn cfgsync_configs_match_topology_ports_and_genesis() {
        let scenario = ScenarioBuilder::topology_with(|t| t.validators(1).executors(1))
            .build()
            .expect("scenario build should succeed");
        let topology = scenario.topology().clone();
        let hosts = hosts_from_topology(&topology);
        let tracing_settings = tracing_settings(&topology);

        let configs = create_node_configs(
            &topology.config().consensus_params,
            &topology.config().da_params,
            &tracing_settings,
            &topology.config().wallet_config,
            Some(topology.nodes().map(|node| node.id).collect()),
            Some(topology.nodes().map(|node| node.da_port).collect()),
            Some(topology.nodes().map(|node| node.blend_port).collect()),
            hosts,
        )
        .expect("cfgsync config generation should succeed");
        let configs_by_identifier: HashMap<_, _> = configs
            .into_iter()
            .map(|(host, config)| (host.identifier, config))
            .collect();

        for node in topology.nodes() {
            let identifier = identifier_for(node.role(), node.index());
            let cfg = configs_by_identifier
                .get(&identifier)
                .unwrap_or_else(|| panic!("missing cfgsync config for {identifier}"));

            assert_eq!(
                declaration_fingerprint(&node.general.consensus_config.genesis_tx),
                declaration_fingerprint(&cfg.consensus_config.genesis_tx),
                "genesis declaration mismatch for {identifier}"
            );

            let expected_net_port = node.network_port();
            assert_eq!(
                cfg.network_config.backend.swarm.port, expected_net_port,
                "network port mismatch for {identifier}"
            );

            assert_eq!(
                multiaddr_port(&cfg.da_config.listening_address),
                Some(node.da_port),
                "DA listening port mismatch for {identifier}"
            );
            assert_eq!(
                multiaddr_port(&cfg.blend_config.backend_core.listening_address),
                Some(node.blend_port),
                "blend listening port mismatch for {identifier}"
            );

            assert_eq!(
                cfg.api_config.address.port(),
                node.general.api_config.address.port(),
                "api port mismatch for {identifier}"
            );
            assert_eq!(
                cfg.api_config.testing_http_address.port(),
                node.general.api_config.testing_http_address.port(),
                "testing http port mismatch for {identifier}"
            );
        }
    }

    fn hosts_from_topology(topology: &GeneratedTopology) -> Vec<Host> {
        topology.nodes().map(host_from_node).collect()
    }

    fn docker_style_hosts(topology: &GeneratedTopology) -> Vec<Host> {
        topology
            .nodes()
            .map(|node| docker_host(node, 10 + node.index() as u8))
            .collect()
    }

    fn host_from_node(node: &GeneratedNodeConfig) -> Host {
        let identifier = identifier_for(node.role(), node.index());
        let ip = Ipv4Addr::LOCALHOST;
        let mut host = make_host(node.role(), ip, identifier);
        host.network_port = node.network_port();
        host.da_network_port = node.da_port;
        host.blend_port = node.blend_port;
        host
    }

    fn docker_host(node: &GeneratedNodeConfig, octet: u8) -> Host {
        let identifier = identifier_for(node.role(), node.index());
        let ip = Ipv4Addr::new(172, 23, 0, octet);
        let mut host = make_host(node.role(), ip, identifier);
        host.network_port = node.network_port().saturating_add(1000);
        host.da_network_port = node.da_port.saturating_add(1000);
        host.blend_port = node.blend_port.saturating_add(1000);
        host
    }

    fn tracing_settings(topology: &GeneratedTopology) -> TracingSettings {
        topology
            .validators()
            .first()
            .or_else(|| topology.executors().first())
            .expect("topology must contain at least one node")
            .general
            .tracing_config
            .tracing_settings
            .clone()
    }

    fn identifier_for(role: TopologyNodeRole, index: usize) -> String {
        match role {
            TopologyNodeRole::Validator => format!("validator-{index}"),
            TopologyNodeRole::Executor => format!("executor-{index}"),
        }
    }

    fn make_host(role: TopologyNodeRole, ip: Ipv4Addr, identifier: String) -> Host {
        let ports = PortOverrides {
            network_port: None,
            da_network_port: None,
            blend_port: None,
            api_port: None,
            testing_http_port: None,
        };
        match role {
            TopologyNodeRole::Validator => Host::validator_from_ip(ip, identifier, ports),
            TopologyNodeRole::Executor => Host::executor_from_ip(ip, identifier, ports),
        }
    }

    fn declaration_fingerprint<G>(
        genesis: &G,
    ) -> Vec<(ServiceType, ProviderId, NoteId, ZkPublicKey)>
    where
        G: GenesisTxTrait,
    {
        genesis
            .sdp_declarations()
            .map(|(op, _)| (op.service_type, op.provider_id, op.locked_note_id, op.zk_id))
            .collect()
    }
}

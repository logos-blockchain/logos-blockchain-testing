use std::{
    env,
    net::{Ipv4Addr, TcpListener as StdTcpListener},
    sync::Arc,
};

use async_trait::async_trait;
use testing_framework_core::{
    scenario::{
        BlockFeed, BlockFeedTask, CleanupGuard, Deployer, NodeClients, NodeControlHandle,
        RequiresNodeControl, RunContext, Runner, Scenario,
    },
    topology::generation::GeneratedTopology,
};
use tracing::{debug, info};

use crate::{
    block_feed::spawn_block_feed_with_retry,
    cleanup::RunnerCleanup,
    compose::HostPortMapping,
    control::ComposeNodeControl,
    docker::ensure_docker_available,
    environment::{
        PortReservation, StackEnvironment, ensure_supported_topology, prepare_environment,
    },
    errors::ComposeRunnerError,
    ports::{compose_runner_host, discover_host_ports, ensure_remote_readiness_with_ports},
    readiness::{
        build_node_clients_with_ports, ensure_executors_ready_with_ports,
        ensure_validators_ready_with_ports, maybe_sleep_for_disabled_readiness,
        metrics_handle_from_port,
    },
};

/// Docker Compose-based deployer for Nomos test scenarios.
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

    async fn prepare_ports(
        &self,
        environment: &mut StackEnvironment,
        descriptors: &GeneratedTopology,
    ) -> Result<HostPortMapping, ComposeRunnerError> {
        debug!("resolving host ports for compose services");
        match discover_host_ports(environment, descriptors).await {
            Ok(mapping) => {
                info!(
                    validator_ports = ?mapping.validator_api_ports(),
                    executor_ports = ?mapping.executor_api_ports(),
                    prometheus_port = environment.prometheus_port(),
                    "resolved container host ports"
                );
                Ok(mapping)
            }
            Err(err) => {
                environment
                    .fail("failed to determine container host ports")
                    .await;
                Err(err)
            }
        }
    }

    async fn wait_for_readiness(
        &self,
        descriptors: &GeneratedTopology,
        host_ports: &HostPortMapping,
        environment: &mut StackEnvironment,
    ) -> Result<(), ComposeRunnerError> {
        info!(
            ports = ?host_ports.validator_api_ports(),
            "waiting for validator HTTP endpoints"
        );
        if let Err(err) =
            ensure_validators_ready_with_ports(&host_ports.validator_api_ports()).await
        {
            environment.fail("validator readiness failed").await;
            return Err(err.into());
        }

        info!(
            ports = ?host_ports.executor_api_ports(),
            "waiting for executor HTTP endpoints"
        );
        if let Err(err) = ensure_executors_ready_with_ports(&host_ports.executor_api_ports()).await
        {
            environment.fail("executor readiness failed").await;
            return Err(err.into());
        }

        info!("waiting for remote service readiness");
        if let Err(err) = ensure_remote_readiness_with_ports(descriptors, host_ports).await {
            environment.fail("remote readiness probe failed").await;
            return Err(err.into());
        }

        Ok(())
    }

    async fn build_node_clients(
        &self,
        descriptors: &GeneratedTopology,
        host_ports: &HostPortMapping,
        host: &str,
        environment: &mut StackEnvironment,
    ) -> Result<NodeClients, ComposeRunnerError> {
        match build_node_clients_with_ports(descriptors, host_ports, host) {
            Ok(clients) => Ok(clients),
            Err(err) => {
                environment
                    .fail("failed to construct node api clients")
                    .await;
                Err(err.into())
            }
        }
    }

    async fn start_block_feed(
        &self,
        node_clients: &NodeClients,
        environment: &mut StackEnvironment,
    ) -> Result<(BlockFeed, BlockFeedTask), ComposeRunnerError> {
        match spawn_block_feed_with_retry(node_clients).await {
            Ok(pair) => {
                info!("block feed connected to validator");
                Ok(pair)
            }
            Err(err) => {
                environment.fail("failed to initialize block feed").await;
                Err(err)
            }
        }
    }

    fn maybe_node_control<Caps>(
        &self,
        environment: &StackEnvironment,
    ) -> Option<Arc<dyn NodeControlHandle>>
    where
        Caps: RequiresNodeControl + Send + Sync,
    {
        Caps::REQUIRED.then(|| {
            Arc::new(ComposeNodeControl {
                compose_file: environment.compose_path().to_path_buf(),
                project_name: environment.project_name().to_owned(),
            }) as Arc<dyn NodeControlHandle>
        })
    }
}

pub(crate) const PROMETHEUS_PORT_ENV: &str = "TEST_FRAMEWORK_PROMETHEUS_PORT";
pub(crate) const DEFAULT_PROMETHEUS_PORT: u16 = 9090;

fn allocate_prometheus_port() -> Option<PortReservation> {
    reserve_port(DEFAULT_PROMETHEUS_PORT).or_else(|| reserve_port(0))
}

fn reserve_port(port: u16) -> Option<PortReservation> {
    let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, port)).ok()?;
    let actual_port = listener.local_addr().ok()?.port();
    Some(PortReservation::new(actual_port, Some(listener)))
}

#[async_trait]
impl<Caps> Deployer<Caps> for ComposeDeployer
where
    Caps: RequiresNodeControl + Send + Sync,
{
    type Error = ComposeRunnerError;

    async fn deploy(&self, scenario: &Scenario<Caps>) -> Result<Runner, Self::Error> {
        ensure_docker_available().await?;
        let descriptors = scenario.topology().clone();
        ensure_supported_topology(&descriptors)?;

        info!(
            validators = descriptors.validators().len(),
            executors = descriptors.executors().len(),
            "starting compose deployment"
        );

        let prometheus_env = env::var(PROMETHEUS_PORT_ENV)
            .ok()
            .and_then(|raw| raw.parse::<u16>().ok());
        if prometheus_env.is_some() {
            info!(port = prometheus_env, "using prometheus port from env");
        }
        let prometheus_port = prometheus_env
            .and_then(|port| reserve_port(port))
            .or_else(|| allocate_prometheus_port())
            .unwrap_or_else(|| PortReservation::new(DEFAULT_PROMETHEUS_PORT, None));
        let mut environment =
            prepare_environment(&descriptors, prometheus_port, prometheus_env.is_some()).await?;
        info!(
            compose_file = %environment.compose_path().display(),
            project = environment.project_name(),
            root = %environment.root().display(),
            "compose workspace prepared"
        );

        let host_ports = self.prepare_ports(&mut environment, &descriptors).await?;

        if self.readiness_checks {
            self.wait_for_readiness(&descriptors, &host_ports, &mut environment)
                .await?;
        } else {
            info!("readiness checks disabled; giving the stack a short grace period");
            maybe_sleep_for_disabled_readiness(false).await;
        }

        info!("compose stack ready; building node clients");
        let host = compose_runner_host();
        let node_clients = self
            .build_node_clients(&descriptors, &host_ports, &host, &mut environment)
            .await?;
        let telemetry = metrics_handle_from_port(environment.prometheus_port(), &host)?;
        let node_control = self.maybe_node_control::<Caps>(&environment);

        let (block_feed, block_feed_guard) = self
            .start_block_feed(&node_clients, &mut environment)
            .await?;
        let cleanup_guard: Box<dyn CleanupGuard> = Box::new(ComposeCleanupGuard::new(
            environment.into_cleanup(),
            block_feed_guard,
        ));
        let context = RunContext::new(
            descriptors,
            None,
            node_clients,
            scenario.duration(),
            telemetry,
            block_feed,
            node_control,
        );

        Ok(Runner::new(context, Some(cleanup_guard)))
    }
}

struct ComposeCleanupGuard {
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

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, net::Ipv4Addr};

    use cfgsync::config::{Host, PortOverrides, create_node_configs};
    use groth16::Fr;
    use nomos_core::{
        mantle::{GenesisTx as GenesisTxTrait, ledger::NoteId},
        sdp::{ProviderId, ServiceType},
    };
    use nomos_ledger::LedgerState;
    use nomos_tracing_service::TracingSettings;
    use testing_framework_core::{
        scenario::ScenarioBuilder,
        topology::generation::{
            GeneratedNodeConfig, GeneratedTopology, NodeRole as TopologyNodeRole,
        },
    };
    use zksign::PublicKey;

    #[test]
    fn cfgsync_prebuilt_configs_preserve_genesis() {
        let scenario = ScenarioBuilder::topology_with(|t| t.validators(1).executors(1)).build();
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
        );
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
        let scenario = ScenarioBuilder::topology_with(|t| t.validators(1).executors(1)).build();
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
        );
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
        let scenario = ScenarioBuilder::topology_with(|t| t.validators(1).executors(1)).build();
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
        );

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

    fn declaration_fingerprint<G>(genesis: &G) -> Vec<(ServiceType, ProviderId, NoteId, PublicKey)>
    where
        G: GenesisTxTrait,
    {
        genesis
            .sdp_declarations()
            .map(|(op, _)| (op.service_type, op.provider_id, op.locked_note_id, op.zk_id))
            .collect()
    }
}

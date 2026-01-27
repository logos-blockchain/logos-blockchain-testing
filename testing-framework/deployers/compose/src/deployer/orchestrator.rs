use std::sync::Arc;

use testing_framework_core::scenario::{
    NodeControlHandle, ObservabilityCapabilityProvider, ObservabilityInputs, RequiresNodeControl,
    RunContext, Runner, Scenario,
};
use tracing::info;

use super::{
    ComposeDeployer,
    clients::ClientBuilder,
    make_cleanup_guard,
    ports::PortManager,
    readiness::ReadinessChecker,
    setup::{DeploymentContext, DeploymentSetup},
};
use crate::{
    docker::control::ComposeNodeControl,
    errors::ComposeRunnerError,
    infrastructure::{
        environment::StackEnvironment,
        ports::{HostPortMapping, compose_runner_host},
    },
};

pub struct DeploymentOrchestrator {
    deployer: ComposeDeployer,
}

impl DeploymentOrchestrator {
    pub const fn new(deployer: ComposeDeployer) -> Self {
        Self { deployer }
    }

    pub async fn deploy<Caps>(
        &self,
        scenario: &Scenario<Caps>,
    ) -> Result<Runner, ComposeRunnerError>
    where
        Caps: RequiresNodeControl + ObservabilityCapabilityProvider + Send + Sync,
    {
        let setup = DeploymentSetup::new(scenario.topology());
        setup.validate_environment().await?;

        let observability = resolve_observability_inputs(scenario)?;

        let DeploymentContext {
            mut environment,
            descriptors,
        } = setup.prepare_workspace(&observability).await?;

        tracing::info!(
            nodes = descriptors.nodes().len(),
            duration_secs = scenario.duration().as_secs(),
            readiness_checks = self.deployer.readiness_checks,
            metrics_query_url = observability.metrics_query_url.as_ref().map(|u| u.as_str()),
            metrics_otlp_ingest_url = observability
                .metrics_otlp_ingest_url
                .as_ref()
                .map(|u| u.as_str()),
            grafana_url = observability.grafana_url.as_ref().map(|u| u.as_str()),
            "compose deployment starting"
        );

        let node_count = descriptors.nodes().len();
        let host_ports = PortManager::prepare(&mut environment, &descriptors).await?;

        wait_for_readiness_or_grace_period(
            self.deployer.readiness_checks,
            &descriptors,
            &host_ports,
            &mut environment,
        )
        .await?;

        let host = compose_runner_host();
        let client_builder = ClientBuilder::new();
        let node_clients = client_builder
            .build_node_clients(&descriptors, &host_ports, &host, &mut environment)
            .await?;
        let telemetry = observability.telemetry_handle()?;
        let node_control = self.maybe_node_control::<Caps>(&environment);

        log_observability_endpoints(&observability);
        log_profiling_urls(&host, &host_ports);

        maybe_print_endpoints(&observability, &host, &host_ports);

        let (block_feed, block_feed_guard) = client_builder
            .start_block_feed(&node_clients, &mut environment)
            .await?;
        let cleanup_guard = make_cleanup_guard(environment.into_cleanup()?, block_feed_guard);

        let context = RunContext::new(
            descriptors,
            None,
            node_clients,
            scenario.duration(),
            telemetry,
            block_feed,
            node_control,
        );

        info!(
            nodes = node_count,
            duration_secs = scenario.duration().as_secs(),
            readiness_checks = self.deployer.readiness_checks,
            host,
            "compose deployment ready; handing control to scenario runner"
        );

        Ok(Runner::new(context, Some(cleanup_guard)))
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

fn resolve_observability_inputs<Caps>(
    scenario: &Scenario<Caps>,
) -> Result<ObservabilityInputs, ComposeRunnerError>
where
    Caps: ObservabilityCapabilityProvider,
{
    let env_inputs = ObservabilityInputs::from_env()?;
    let cap_inputs = scenario
        .capabilities()
        .observability_capability()
        .map(ObservabilityInputs::from_capability)
        .unwrap_or_default();
    Ok(env_inputs.with_overrides(cap_inputs))
}

async fn wait_for_readiness_or_grace_period(
    readiness_checks: bool,
    descriptors: &testing_framework_core::topology::generation::GeneratedTopology,
    host_ports: &HostPortMapping,
    environment: &mut StackEnvironment,
) -> Result<(), ComposeRunnerError> {
    if readiness_checks {
        ReadinessChecker::wait_all(descriptors, host_ports, environment).await?;
        return Ok(());
    }

    info!("readiness checks disabled; giving the stack a short grace period");
    crate::lifecycle::readiness::maybe_sleep_for_disabled_readiness(false).await;
    Ok(())
}

fn log_observability_endpoints(observability: &ObservabilityInputs) {
    if let Some(url) = observability.metrics_query_url.as_ref() {
        info!(
            metrics_query_url = %url.as_str(),
            "metrics query endpoint configured"
        );
    }
    if let Some(url) = observability.grafana_url.as_ref() {
        info!(grafana_url = %url.as_str(), "grafana url configured");
    }
}

fn maybe_print_endpoints(observability: &ObservabilityInputs, host: &str, ports: &HostPortMapping) {
    if std::env::var("TESTNET_PRINT_ENDPOINTS").is_err() {
        return;
    }

    let prometheus = observability
        .metrics_query_url
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_else(|| "<disabled>".to_string());
    let grafana = observability
        .grafana_url
        .as_ref()
        .map(|u| u.as_str().to_string())
        .unwrap_or_else(|| "<disabled>".to_string());

    println!(
        "TESTNET_ENDPOINTS prometheus={} grafana={}",
        prometheus, grafana
    );
    print_profiling_urls(host, ports);
}

fn log_profiling_urls(host: &str, ports: &HostPortMapping) {
    for (idx, node) in ports.nodes.iter().enumerate() {
        tracing::info!(
            node = idx,
            profiling_url = %format!(
                "http://{}:{}/debug/pprof/profile?seconds=15&format=proto",
                host, node.api
            ),
            "node profiling endpoint (profiling feature required)"
        );
    }
}

fn print_profiling_urls(host: &str, ports: &HostPortMapping) {
    for (idx, node) in ports.nodes.iter().enumerate() {
        println!(
            "TESTNET_PPROF node_{}=http://{}:{}/debug/pprof/profile?seconds=15&format=proto",
            idx, host, node.api
        );
    }
}

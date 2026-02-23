use std::{env, sync::Arc, time::Duration};

use reqwest::Url;
use testing_framework_core::{
    scenario::{
        DeploymentPolicy, FeedHandle, FeedRuntime, HttpReadinessRequirement, Metrics, NodeClients,
        NodeControlHandle, ObservabilityCapabilityProvider, ObservabilityInputs,
        RequiresNodeControl, RunContext, Runner, Scenario, build_source_orchestration_plan,
        orchestrate_sources,
    },
    topology::DeploymentDescriptor,
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
    env::ComposeDeployEnv,
    errors::ComposeRunnerError,
    infrastructure::{
        environment::StackEnvironment,
        ports::{HostPortMapping, compose_runner_host},
    },
};

const PRINT_ENDPOINTS_ENV: &str = "TESTNET_PRINT_ENDPOINTS";

pub struct DeploymentOrchestrator<E: ComposeDeployEnv> {
    deployer: ComposeDeployer<E>,
}

impl<E: ComposeDeployEnv> DeploymentOrchestrator<E> {
    pub const fn new(deployer: ComposeDeployer<E>) -> Self {
        Self { deployer }
    }

    pub async fn deploy<Caps>(
        &self,
        scenario: &Scenario<E, Caps>,
    ) -> Result<Runner<E>, ComposeRunnerError>
    where
        Caps: RequiresNodeControl + ObservabilityCapabilityProvider + Send + Sync,
    {
        // Source planning is currently resolved here before deployer-specific setup.
        let source_plan = build_source_orchestration_plan(scenario).map_err(|source| {
            ComposeRunnerError::SourceOrchestration {
                source: source.into(),
            }
        })?;

        let deployment = scenario.deployment();
        let setup = DeploymentSetup::<E>::new(deployment);
        setup.validate_environment().await?;

        let observability = resolve_observability_inputs(scenario)?;
        let mut prepared = prepare_deployment::<E>(setup, &observability).await?;
        let deployment_policy = scenario.deployment_policy();
        let readiness_enabled =
            self.deployer.readiness_checks && deployment_policy.readiness_enabled;

        self.log_deploy_start(
            scenario,
            &prepared.descriptors,
            deployment_policy,
            &observability,
        );

        let mut deployed = deploy_nodes::<E>(
            &mut prepared.environment,
            &prepared.descriptors,
            readiness_enabled,
            deployment_policy.readiness_requirement,
        )
        .await?;

        // Source orchestration currently runs here after managed clients are prepared.
        deployed.node_clients = orchestrate_sources(&source_plan, deployed.node_clients)
            .await
            .map_err(|source| ComposeRunnerError::SourceOrchestration { source })?;

        let runner = self
            .build_runner::<Caps>(
                scenario,
                prepared,
                deployed,
                observability,
                readiness_enabled,
            )
            .await?;

        self.log_deploy_ready(
            scenario,
            deployment_policy,
            deployment.node_count(),
            &compose_runner_host(),
            readiness_enabled,
        );

        Ok(runner)
    }

    async fn build_runner<Caps>(
        &self,
        scenario: &Scenario<E, Caps>,
        mut prepared: PreparedDeployment<E>,
        deployed: DeployedNodes<E>,
        observability: ObservabilityInputs,
        readiness_enabled: bool,
    ) -> Result<Runner<E>, ComposeRunnerError>
    where
        Caps: RequiresNodeControl + ObservabilityCapabilityProvider + Send + Sync,
    {
        let telemetry = observability.telemetry_handle()?;
        let node_control = self.maybe_node_control::<Caps>(&prepared.environment);

        log_observability_endpoints(&observability);
        log_profiling_urls(&deployed.host, &deployed.host_ports);
        maybe_print_endpoints(&observability, &deployed.host, &deployed.host_ports);

        let input = RuntimeBuildInput {
            deployed: &deployed,
            descriptors: prepared.descriptors.clone(),
            duration: scenario.duration(),
            expectation_cooldown: scenario.expectation_cooldown(),
            telemetry,
            environment: &mut prepared.environment,
            node_control,
        };
        let runtime = build_compose_runtime::<E>(input).await?;
        let cleanup_guard =
            make_cleanup_guard(prepared.environment.into_cleanup()?, runtime.feed_task);

        info!(
            effective_readiness = readiness_enabled,
            host = deployed.host,
            "compose runtime prepared"
        );

        Ok(Runner::new(runtime.context, Some(cleanup_guard)))
    }

    fn maybe_node_control<Caps>(
        &self,
        environment: &StackEnvironment,
    ) -> Option<Arc<dyn NodeControlHandle<E>>>
    where
        Caps: RequiresNodeControl + Send + Sync,
    {
        Caps::REQUIRED.then(|| {
            Arc::new(ComposeNodeControl {
                compose_file: environment.compose_path().to_path_buf(),
                project_name: environment.project_name().to_owned(),
            }) as Arc<dyn NodeControlHandle<E>>
        })
    }

    fn log_deploy_start<Caps>(
        &self,
        scenario: &Scenario<E, Caps>,
        descriptors: &E::Deployment,
        deployment_policy: DeploymentPolicy,
        observability: &ObservabilityInputs,
    ) {
        let effective_readiness =
            self.deployer.readiness_checks && deployment_policy.readiness_enabled;

        info!(
            nodes = descriptors.node_count(),
            duration_secs = scenario.duration().as_secs(),
            readiness_checks = self.deployer.readiness_checks,
            readiness_enabled = deployment_policy.readiness_enabled,
            readiness_requirement = ?deployment_policy.readiness_requirement,
            effective_readiness,
            metrics_query_url = observability.metrics_query_url.as_ref().map(|u| u.as_str()),
            metrics_otlp_ingest_url = observability
                .metrics_otlp_ingest_url
                .as_ref()
                .map(|u| u.as_str()),
            grafana_url = observability.grafana_url.as_ref().map(|u| u.as_str()),
            "compose deployment starting"
        );
    }

    fn log_deploy_ready<Caps>(
        &self,
        scenario: &Scenario<E, Caps>,
        deployment_policy: DeploymentPolicy,
        node_count: usize,
        host: &str,
        readiness_enabled: bool,
    ) {
        info!(
            nodes = node_count,
            duration_secs = scenario.duration().as_secs(),
            readiness_checks = self.deployer.readiness_checks,
            readiness_enabled = deployment_policy.readiness_enabled,
            readiness_requirement = ?deployment_policy.readiness_requirement,
            effective_readiness = readiness_enabled,
            host,
            "compose deployment ready; handing control to scenario runner"
        );
    }
}

struct DeployedNodes<E: ComposeDeployEnv> {
    host_ports: HostPortMapping,
    host: String,
    node_clients: NodeClients<E>,
    client_builder: ClientBuilder<E>,
}

struct ComposeRuntime<E: ComposeDeployEnv> {
    context: RunContext<E>,
    feed_task: FeedHandle,
}

struct RuntimeBuildInput<'a, E: ComposeDeployEnv> {
    deployed: &'a DeployedNodes<E>,
    descriptors: E::Deployment,
    duration: Duration,
    expectation_cooldown: Duration,
    telemetry: Metrics,
    environment: &'a mut StackEnvironment,
    node_control: Option<Arc<dyn NodeControlHandle<E>>>,
}

async fn build_compose_runtime<E: ComposeDeployEnv>(
    input: RuntimeBuildInput<'_, E>,
) -> Result<ComposeRuntime<E>, ComposeRunnerError> {
    let node_clients = input.deployed.node_clients.clone();
    if node_clients.is_empty() {
        return Err(ComposeRunnerError::RuntimePreflight);
    }

    let (feed, feed_task) = input
        .deployed
        .client_builder
        .start_block_feed(&node_clients, input.environment)
        .await?;

    let context = build_run_context(
        input.descriptors,
        node_clients,
        input.duration,
        input.expectation_cooldown,
        input.telemetry,
        feed,
        input.node_control,
    );

    Ok(ComposeRuntime { context, feed_task })
}

async fn deploy_nodes<E: ComposeDeployEnv>(
    environment: &mut StackEnvironment,
    descriptors: &E::Deployment,
    readiness_enabled: bool,
    readiness_requirement: HttpReadinessRequirement,
) -> Result<DeployedNodes<E>, ComposeRunnerError> {
    let host_ports = PortManager::<E>::prepare(environment, descriptors).await?;
    wait_for_readiness_or_grace_period::<E>(
        readiness_enabled,
        descriptors,
        readiness_requirement,
        &host_ports,
        environment,
    )
    .await?;

    let host = compose_runner_host();
    let client_builder = ClientBuilder::<E>::new();
    let node_clients = client_builder
        .build_node_clients(descriptors, &host_ports, &host, environment)
        .await?;

    Ok(DeployedNodes {
        host_ports,
        host,
        node_clients,
        client_builder,
    })
}

fn build_run_context<E: ComposeDeployEnv>(
    descriptors: E::Deployment,
    node_clients: NodeClients<E>,
    run_duration: Duration,
    expectation_cooldown: Duration,
    telemetry: Metrics,
    feed: <E::FeedRuntime as FeedRuntime>::Feed,
    node_control: Option<Arc<dyn NodeControlHandle<E>>>,
) -> RunContext<E> {
    RunContext::new(
        descriptors,
        node_clients,
        run_duration,
        expectation_cooldown,
        telemetry,
        feed,
        node_control,
    )
}

fn resolve_observability_inputs<E, Caps>(
    scenario: &Scenario<E, Caps>,
) -> Result<ObservabilityInputs, ComposeRunnerError>
where
    Caps: ObservabilityCapabilityProvider,
    E: ComposeDeployEnv,
{
    let env_inputs = ObservabilityInputs::from_env()?;
    let cap_inputs = scenario
        .capabilities()
        .observability_capability()
        .map(ObservabilityInputs::from_capability)
        .unwrap_or_default();

    Ok(env_inputs.with_overrides(cap_inputs))
}

async fn wait_for_readiness_or_grace_period<E: ComposeDeployEnv>(
    readiness_checks: bool,
    descriptors: &E::Deployment,
    readiness_requirement: HttpReadinessRequirement,
    host_ports: &HostPortMapping,
    environment: &mut StackEnvironment,
) -> Result<(), ComposeRunnerError> {
    if readiness_checks {
        ReadinessChecker::<E>::wait_all(
            descriptors,
            host_ports,
            readiness_requirement,
            environment,
        )
        .await?;
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
    if !should_print_endpoints() {
        return;
    }

    let prometheus = endpoint_or_disabled(observability.metrics_query_url.as_ref());
    let grafana = endpoint_or_disabled(observability.grafana_url.as_ref());

    println!(
        "TESTNET_ENDPOINTS prometheus={} grafana={}",
        prometheus, grafana
    );

    print_profiling_urls(host, ports);
}

fn should_print_endpoints() -> bool {
    env::var(PRINT_ENDPOINTS_ENV).is_ok()
}

fn endpoint_or_disabled(endpoint: Option<&Url>) -> String {
    endpoint.map_or_else(|| "<disabled>".to_string(), |url| url.as_str().to_string())
}

fn log_profiling_urls(host: &str, ports: &HostPortMapping) {
    for (idx, node) in ports.nodes.iter().enumerate() {
        info!(
            node = idx,
            profiling_url = %profiling_url(host, node.api),
            "node profiling endpoint (profiling feature required)"
        );
    }
}

fn print_profiling_urls(host: &str, ports: &HostPortMapping) {
    for (idx, node) in ports.nodes.iter().enumerate() {
        println!(
            "TESTNET_PPROF node_{}={}",
            idx,
            profiling_url(host, node.api)
        );
    }
}

fn profiling_url(host: &str, api_port: u16) -> String {
    format!("http://{host}:{api_port}/debug/pprof/profile?seconds=15&format=proto")
}

struct PreparedDeployment<E: ComposeDeployEnv> {
    environment: StackEnvironment,
    descriptors: E::Deployment,
}

async fn prepare_deployment<E: ComposeDeployEnv>(
    setup: DeploymentSetup<'_, E>,
    observability: &ObservabilityInputs,
) -> Result<PreparedDeployment<E>, ComposeRunnerError> {
    let DeploymentContext {
        environment,
        descriptors,
    } = setup.prepare_workspace(observability).await?;

    Ok(PreparedDeployment {
        environment,
        descriptors: descriptors.clone(),
    })
}

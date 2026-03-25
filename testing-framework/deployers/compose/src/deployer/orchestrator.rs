use std::{env, sync::Arc, time::Duration};

use reqwest::Url;
use testing_framework_core::{
    scenario::{
        Application, ClusterControlProfile, ClusterMode, ClusterWaitHandle, DeploymentPolicy,
        DynError, ExistingCluster, FeedRuntime, HttpReadinessRequirement, Metrics, NodeClients,
        NodeControlHandle, ObservabilityCapabilityProvider, ObservabilityInputs,
        RequiresNodeControl, Runner, Scenario,
        internal::{
            ApplicationExternalProvider, CleanupGuard, FeedHandle, RuntimeAssembly,
            SourceOrchestrationPlan, SourceProviders, StaticManagedProvider,
            build_source_orchestration_plan, orchestrate_sources_with_providers,
        },
    },
    topology::DeploymentDescriptor,
};
use tracing::info;

use super::{
    ComposeDeployer, ComposeDeploymentMetadata,
    attach_provider::{ComposeAttachProvider, ComposeAttachedClusterWait},
    clients::ClientBuilder,
    make_cleanup_guard,
    ports::PortManager,
    readiness::ReadinessChecker,
    setup::{DeploymentContext, DeploymentSetup},
};
use crate::{
    docker::control::{ComposeAttachedNodeControl, ComposeNodeControl},
    env::ComposeDeployEnv,
    errors::ComposeRunnerError,
    infrastructure::{
        environment::StackEnvironment,
        ports::{HostPortMapping, compose_runner_host},
    },
    lifecycle::block_feed::spawn_block_feed_with_retry,
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
        self.deploy_with_metadata(scenario)
            .await
            .map(|(runner, _)| runner)
    }

    pub async fn deploy_with_metadata<Caps>(
        &self,
        scenario: &Scenario<E, Caps>,
    ) -> Result<(Runner<E>, ComposeDeploymentMetadata), ComposeRunnerError>
    where
        Caps: RequiresNodeControl + ObservabilityCapabilityProvider + Send + Sync,
    {
        validate_supported_cluster_mode(scenario).map_err(|source| {
            ComposeRunnerError::SourceOrchestration {
                source: source.into(),
            }
        })?;

        // Source planning is currently resolved here before deployer-specific setup.
        let source_plan = build_source_orchestration_plan(scenario).map_err(|source| {
            ComposeRunnerError::SourceOrchestration {
                source: source.into(),
            }
        })?;

        if matches!(scenario.cluster_mode(), ClusterMode::ExistingCluster) {
            return self
                .deploy_existing_cluster::<Caps>(scenario, source_plan)
                .await
                .map(|runner| (runner, existing_cluster_metadata(scenario)));
        }

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

        let source_providers = self.source_providers(deployed.node_clients.snapshot());

        deployed.node_clients = self
            .resolve_node_clients(&source_plan, source_providers)
            .await?;

        let project_name = prepared.environment.project_name().to_owned();

        let runner = self
            .build_runner::<Caps>(
                scenario,
                prepared,
                deployed,
                observability,
                readiness_enabled,
                project_name.clone(),
            )
            .await?;

        self.log_deploy_ready(
            scenario,
            deployment_policy,
            deployment.node_count(),
            &compose_runner_host(),
            readiness_enabled,
        );

        Ok((
            runner,
            ComposeDeploymentMetadata {
                project_name: Some(project_name),
            },
        ))
    }

    async fn deploy_existing_cluster<Caps>(
        &self,
        scenario: &Scenario<E, Caps>,
        source_plan: SourceOrchestrationPlan,
    ) -> Result<Runner<E>, ComposeRunnerError>
    where
        Caps: RequiresNodeControl + ObservabilityCapabilityProvider + Send + Sync,
    {
        let observability = resolve_observability_inputs(scenario)?;
        let source_providers = self.source_providers(Vec::new());

        let node_clients = self
            .resolve_node_clients(&source_plan, source_providers)
            .await?;

        self.ensure_non_empty_node_clients(&node_clients)?;

        let node_control = self.attached_node_control::<Caps>(scenario)?;
        let cluster_wait = self.attached_cluster_wait(scenario)?;
        let (feed, feed_task) = spawn_block_feed_with_retry::<E>(&node_clients).await?;
        let assembly = build_runtime_assembly(
            scenario.deployment().clone(),
            node_clients,
            scenario.duration(),
            scenario.expectation_cooldown(),
            scenario.cluster_control_profile(),
            observability.telemetry_handle()?,
            feed,
            node_control,
            cluster_wait,
        );

        let cleanup_guard: Box<dyn CleanupGuard> = Box::new(feed_task);
        Ok(assembly.build_runner(Some(cleanup_guard)))
    }

    fn source_providers(&self, managed_clients: Vec<E::NodeClient>) -> SourceProviders<E> {
        SourceProviders::default()
            .with_managed(Arc::new(StaticManagedProvider::new(managed_clients)))
            .with_attach(Arc::new(ComposeAttachProvider::<E>::new(
                compose_runner_host(),
            )))
            .with_external(Arc::new(ApplicationExternalProvider))
    }

    async fn resolve_node_clients(
        &self,
        source_plan: &SourceOrchestrationPlan,
        source_providers: SourceProviders<E>,
    ) -> Result<NodeClients<E>, ComposeRunnerError> {
        orchestrate_sources_with_providers(source_plan, source_providers)
            .await
            .map_err(|source| ComposeRunnerError::SourceOrchestration { source })
    }

    fn ensure_non_empty_node_clients(
        &self,
        node_clients: &NodeClients<E>,
    ) -> Result<(), ComposeRunnerError> {
        if node_clients.is_empty() {
            return Err(ComposeRunnerError::RuntimePreflight);
        }

        Ok(())
    }

    fn attached_node_control<Caps>(
        &self,
        scenario: &Scenario<E, Caps>,
    ) -> Result<Option<Arc<dyn NodeControlHandle<E>>>, ComposeRunnerError>
    where
        Caps: RequiresNodeControl + Send + Sync,
    {
        if !Caps::REQUIRED {
            return Ok(None);
        }

        let attach = scenario
            .existing_cluster()
            .ok_or(ComposeRunnerError::InternalInvariant {
                message: "existing-cluster node control requested outside existing-cluster mode",
            })?;
        let node_control = ComposeAttachedNodeControl::try_from_existing_cluster(attach)
            .map_err(|source| ComposeRunnerError::SourceOrchestration { source })?;

        Ok(Some(Arc::new(node_control) as Arc<dyn NodeControlHandle<E>>))
    }

    fn attached_cluster_wait<Caps>(
        &self,
        scenario: &Scenario<E, Caps>,
    ) -> Result<Arc<dyn ClusterWaitHandle<E>>, ComposeRunnerError>
    where
        Caps: Send + Sync,
    {
        let attach = scenario
            .existing_cluster()
            .ok_or(ComposeRunnerError::InternalInvariant {
                message: "compose cluster wait requested outside existing-cluster mode",
            })?;
        let cluster_wait = ComposeAttachedClusterWait::<E>::try_new(compose_runner_host(), attach)
            .map_err(|source| ComposeRunnerError::SourceOrchestration { source })?;

        Ok(Arc::new(cluster_wait))
    }

    async fn build_runner<Caps>(
        &self,
        scenario: &Scenario<E, Caps>,
        mut prepared: PreparedDeployment<E>,
        deployed: DeployedNodes<E>,
        observability: ObservabilityInputs,
        readiness_enabled: bool,
        project_name: String,
    ) -> Result<Runner<E>, ComposeRunnerError>
    where
        Caps: RequiresNodeControl + ObservabilityCapabilityProvider + Send + Sync,
    {
        let telemetry = observability.telemetry_handle()?;
        let node_control = self.maybe_node_control::<Caps>(&prepared.environment);
        let cluster_wait =
            self.managed_cluster_wait(ComposeDeploymentMetadata::for_project(project_name))?;

        log_observability_endpoints(&observability);
        log_profiling_urls(&deployed.host, &deployed.host_ports);
        maybe_print_endpoints(&observability, &deployed.host, &deployed.host_ports);

        let input = RuntimeBuildInput {
            deployed: &deployed,
            descriptors: prepared.descriptors.clone(),
            duration: scenario.duration(),
            expectation_cooldown: scenario.expectation_cooldown(),
            cluster_control_profile: scenario.cluster_control_profile(),
            telemetry,
            environment: &mut prepared.environment,
            node_control,
            cluster_wait,
        };
        let runtime = build_compose_runtime::<E>(input).await?;
        let cleanup_guard =
            make_cleanup_guard(prepared.environment.into_cleanup()?, runtime.feed_task);

        info!(
            effective_readiness = readiness_enabled,
            host = deployed.host,
            "compose runtime prepared"
        );

        Ok(runtime.assembly.build_runner(Some(cleanup_guard)))
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

    fn managed_cluster_wait(
        &self,
        metadata: ComposeDeploymentMetadata,
    ) -> Result<Arc<dyn ClusterWaitHandle<E>>, ComposeRunnerError> {
        let existing_cluster = metadata
            .existing_cluster()
            .map_err(|source| ComposeRunnerError::SourceOrchestration { source })?;
        let cluster_wait =
            ComposeAttachedClusterWait::<E>::try_new(compose_runner_host(), &existing_cluster)
                .map_err(|source| ComposeRunnerError::SourceOrchestration { source })?;

        Ok(Arc::new(cluster_wait))
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

fn validate_supported_cluster_mode<E: Application, Caps>(
    scenario: &Scenario<E, Caps>,
) -> Result<(), DynError> {
    if !matches!(scenario.cluster_mode(), ClusterMode::ExistingCluster) {
        return Ok(());
    }

    let cluster = scenario
        .existing_cluster()
        .ok_or_else(|| DynError::from("existing-cluster mode requires an existing cluster"))?;

    ensure_compose_existing_cluster(cluster)
}

fn ensure_compose_existing_cluster(cluster: &ExistingCluster) -> Result<(), DynError> {
    if cluster.compose_project().is_some() && cluster.compose_services().is_some() {
        return Ok(());
    }

    Err("compose deployer requires a compose existing-cluster descriptor".into())
}

#[cfg(test)]
mod tests {
    use testing_framework_core::scenario::ExistingCluster;

    use super::ensure_compose_existing_cluster;

    #[test]
    fn compose_cluster_validator_accepts_compose_descriptor() {
        ensure_compose_existing_cluster(&ExistingCluster::for_compose_project(
            "project".to_owned(),
        ))
        .expect("compose descriptor should be accepted");
    }

    #[test]
    fn compose_cluster_validator_rejects_k8s_descriptor() {
        let error = ensure_compose_existing_cluster(&ExistingCluster::for_k8s_selector(
            "app=node".to_owned(),
        ))
        .expect_err("k8s descriptor should be rejected");

        assert_eq!(
            error.to_string(),
            "compose deployer requires a compose existing-cluster descriptor"
        );
    }
}
fn existing_cluster_metadata<E, Caps>(scenario: &Scenario<E, Caps>) -> ComposeDeploymentMetadata
where
    E: ComposeDeployEnv,
    Caps: Send + Sync,
{
    ComposeDeploymentMetadata::from_existing_cluster(scenario.existing_cluster())
}

struct DeployedNodes<E: ComposeDeployEnv> {
    host_ports: HostPortMapping,
    host: String,
    node_clients: NodeClients<E>,
    client_builder: ClientBuilder<E>,
}

struct ComposeRuntime<E: ComposeDeployEnv> {
    assembly: RuntimeAssembly<E>,
    feed_task: FeedHandle,
}

struct RuntimeBuildInput<'a, E: ComposeDeployEnv> {
    deployed: &'a DeployedNodes<E>,
    descriptors: E::Deployment,
    duration: Duration,
    expectation_cooldown: Duration,
    cluster_control_profile: ClusterControlProfile,
    telemetry: Metrics,
    environment: &'a mut StackEnvironment,
    node_control: Option<Arc<dyn NodeControlHandle<E>>>,
    cluster_wait: Arc<dyn ClusterWaitHandle<E>>,
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

    let assembly = build_runtime_assembly(
        input.descriptors,
        node_clients,
        input.duration,
        input.expectation_cooldown,
        input.cluster_control_profile,
        input.telemetry,
        feed,
        input.node_control,
        input.cluster_wait,
    );

    Ok(ComposeRuntime {
        assembly,
        feed_task,
    })
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

fn build_runtime_assembly<E: ComposeDeployEnv>(
    descriptors: E::Deployment,
    node_clients: NodeClients<E>,
    run_duration: Duration,
    expectation_cooldown: Duration,
    cluster_control_profile: ClusterControlProfile,
    telemetry: Metrics,
    feed: <E::FeedRuntime as FeedRuntime>::Feed,
    node_control: Option<Arc<dyn NodeControlHandle<E>>>,
    cluster_wait: Arc<dyn ClusterWaitHandle<E>>,
) -> RuntimeAssembly<E> {
    let mut assembly = RuntimeAssembly::new(
        descriptors,
        node_clients,
        run_duration,
        expectation_cooldown,
        cluster_control_profile,
        telemetry,
        feed,
    )
    .with_cluster_wait(cluster_wait);

    if let Some(node_control) = node_control {
        assembly = assembly.with_node_control(node_control);
    }

    assembly
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

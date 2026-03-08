use std::{env, fmt::Debug, marker::PhantomData, sync::Arc, time::Duration};

use async_trait::async_trait;
use kube::Client;
use reqwest::Url;
use testing_framework_core::{
    scenario::{
        Application, ApplicationExternalProvider, CleanupGuard, ClusterWaitHandle, Deployer,
        DynError, FeedHandle, FeedRuntime, HttpReadinessRequirement, Metrics, MetricsError,
        NodeClients, ObservabilityCapabilityProvider, ObservabilityInputs, RequiresNodeControl,
        RunContext, Runner, Scenario, SourceOrchestrationPlan, SourceProviders,
        StaticManagedProvider, build_source_orchestration_plan, orchestrate_sources_with_providers,
    },
    topology::DeploymentDescriptor,
};
use tracing::{error, info};

use crate::{
    deployer::{
        K8sDeploymentMetadata,
        attach_provider::{K8sAttachProvider, K8sAttachedClusterWait},
    },
    env::K8sDeployEnv,
    infrastructure::cluster::{
        ClusterEnvironment, ClusterEnvironmentError, NodeClientError, PortSpecs,
        RemoteReadinessError, build_node_clients, collect_port_specs, ensure_cluster_readiness,
        kill_port_forwards, wait_for_ports_or_cleanup,
    },
    lifecycle::{block_feed::spawn_block_feed_with, cleanup::RunnerCleanup},
    wait::{ClusterReady, ClusterWaitError, PortForwardHandle},
};

const DISABLED_ENDPOINT: &str = "<disabled>";

/// Deploys a scenario into Kubernetes using an environment-specific stack.
#[derive(Clone, Copy)]
pub struct K8sDeployer<E: K8sDeployEnv> {
    readiness_checks: bool,
    _marker: PhantomData<E>,
}

impl<E: K8sDeployEnv> Default for K8sDeployer<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: K8sDeployEnv> K8sDeployer<E> {
    #[must_use]
    /// Create a k8s deployer with readiness checks enabled.
    pub const fn new() -> Self {
        Self {
            readiness_checks: true,
            _marker: PhantomData,
        }
    }

    #[must_use]
    /// Enable/disable readiness probes before handing control to workloads.
    pub const fn with_readiness(mut self, enabled: bool) -> Self {
        self.readiness_checks = enabled;
        self
    }

    /// Deploy and return k8s-specific metadata alongside the generic runner.
    pub async fn deploy_with_metadata<Caps>(
        &self,
        scenario: &Scenario<E, Caps>,
    ) -> Result<(Runner<E>, K8sDeploymentMetadata), K8sRunnerError>
    where
        Caps: RequiresNodeControl + ObservabilityCapabilityProvider + Send + Sync,
    {
        deploy_with_observability(self, scenario).await
    }
}

#[derive(Debug, thiserror::Error)]
/// High-level runner failures returned to the scenario harness.
pub enum K8sRunnerError {
    #[error("kubernetes runner requires at least one node (nodes={nodes})")]
    UnsupportedTopology { nodes: usize },
    #[error("failed to initialise kubernetes client: {source}")]
    ClientInit {
        #[source]
        source: kube::Error,
    },
    #[error("failed to prepare k8s assets: {source}")]
    Assets {
        #[source]
        source: testing_framework_core::scenario::DynError,
    },
    #[error("failed to install k8s stack: {source}")]
    InstallStack {
        #[source]
        source: testing_framework_core::scenario::DynError,
    },
    #[error(transparent)]
    ClusterEnvironment(#[from] ClusterEnvironmentError),
    #[error(transparent)]
    Cluster(#[from] Box<ClusterWaitError>),
    #[error(transparent)]
    Readiness(#[from] RemoteReadinessError),
    #[error(transparent)]
    NodeClients(#[from] NodeClientError),
    #[error(transparent)]
    Telemetry(#[from] MetricsError),
    #[error("internal invariant violated: {message}")]
    InternalInvariant { message: String },
    #[error("k8s runner requires at least one node client for feed data")]
    BlockFeedMissing,
    #[error("runtime preflight failed: no node clients available")]
    RuntimePreflight,
    #[error("source orchestration failed: {source}")]
    SourceOrchestration {
        #[source]
        source: DynError,
    },
    #[error("failed to initialize feed: {source}")]
    BlockFeed {
        #[source]
        source: DynError,
    },
}

#[async_trait]
impl<E, Caps> Deployer<E, Caps> for K8sDeployer<E>
where
    E: K8sDeployEnv,
    Caps: RequiresNodeControl + ObservabilityCapabilityProvider + Send + Sync,
{
    type Error = K8sRunnerError;

    async fn deploy(&self, scenario: &Scenario<E, Caps>) -> Result<Runner<E>, Self::Error> {
        self.deploy_with_metadata(scenario)
            .await
            .map(|(runner, _)| runner)
    }
}

async fn fail_cluster(cluster: &mut Option<ClusterEnvironment>, reason: &str) {
    if let Some(env) = cluster.as_mut() {
        env.fail(reason).await;
    }
}

impl From<ClusterWaitError> for K8sRunnerError {
    fn from(value: ClusterWaitError) -> Self {
        Self::Cluster(Box::new(value))
    }
}

type Feed<E> = <<E as Application>::FeedRuntime as FeedRuntime>::Feed;

fn ensure_supported_topology<E: K8sDeployEnv>(
    descriptors: &E::Deployment,
) -> Result<(), K8sRunnerError> {
    let nodes = descriptors.node_count();
    if nodes == 0 {
        return Err(K8sRunnerError::UnsupportedTopology { nodes });
    }

    Ok(())
}

async fn deploy_with_observability<E, Caps>(
    deployer: &K8sDeployer<E>,
    scenario: &Scenario<E, Caps>,
) -> Result<(Runner<E>, K8sDeploymentMetadata), K8sRunnerError>
where
    E: K8sDeployEnv,
    Caps: ObservabilityCapabilityProvider + Send + Sync,
{
    // Source planning is currently resolved here before deployer-specific setup.
    let source_plan = build_source_orchestration_plan(scenario).map_err(|source| {
        K8sRunnerError::SourceOrchestration {
            source: source.into(),
        }
    })?;

    let observability = resolve_observability_inputs(scenario.capabilities())?;

    if scenario.uses_existing_cluster() {
        let runner = deploy_attached_only::<E, Caps>(scenario, source_plan, observability).await?;
        return Ok((runner, attached_metadata(scenario)));
    }

    let deployment = build_k8s_deployment::<E, Caps>(deployer, scenario, &observability).await?;
    let metadata = K8sDeploymentMetadata {
        namespace: Some(deployment.cluster.namespace().to_owned()),
        label_selector: Some(E::attach_node_service_selector(
            deployment.cluster.release(),
        )),
    };
    let mut cluster = Some(deployment.cluster);

    let mut runtime = build_runtime_artifacts::<E>(&mut cluster, &observability).await?;
    let cluster_wait = managed_cluster_wait::<E>(&cluster, &metadata)?;

    let source_providers = source_providers::<E>(
        client_from_cluster(&cluster)?,
        runtime.node_clients.snapshot(),
    );

    runtime.node_clients = resolve_node_clients(&source_plan, source_providers).await?;
    ensure_non_empty_node_clients(&runtime.node_clients)?;

    let parts = build_runner_parts(scenario, deployment.node_count, runtime, cluster_wait);

    log_configured_observability(&observability);
    maybe_print_endpoints::<E>(&observability, &parts.node_clients);
    let runner = finalize_runner::<E>(&mut cluster, parts)?;
    Ok((runner, metadata))
}

async fn deploy_attached_only<E, Caps>(
    scenario: &Scenario<E, Caps>,
    source_plan: SourceOrchestrationPlan,
    observability: ObservabilityInputs,
) -> Result<Runner<E>, K8sRunnerError>
where
    E: K8sDeployEnv,
    Caps: ObservabilityCapabilityProvider + Send + Sync,
{
    let client = init_kube_client().await?;
    let source_providers = source_providers::<E>(client.clone(), Vec::new());
    let node_clients = resolve_node_clients(&source_plan, source_providers).await?;

    ensure_non_empty_node_clients(&node_clients)?;

    let telemetry = observability.telemetry_handle()?;
    let (feed, feed_task) = spawn_block_feed_with::<E>(&node_clients).await?;
    let cluster_wait = attached_cluster_wait::<E, Caps>(scenario, client)?;
    let context = RunContext::new(
        scenario.deployment().clone(),
        node_clients,
        scenario.duration(),
        scenario.expectation_cooldown(),
        telemetry,
        feed,
        None,
    )
    .with_cluster_wait(cluster_wait);

    Ok(Runner::new(context, Some(Box::new(feed_task))))
}

fn attached_metadata<E, Caps>(scenario: &Scenario<E, Caps>) -> K8sDeploymentMetadata
where
    E: K8sDeployEnv,
    Caps: Send + Sync,
{
    let namespace = scenario
        .existing_cluster()
        .and_then(|attach| attach.k8s_namespace())
        .map(ToOwned::to_owned);
    let label_selector = scenario
        .existing_cluster()
        .and_then(|attach| attach.k8s_label_selector())
        .map(ToOwned::to_owned);

    K8sDeploymentMetadata {
        namespace,
        label_selector,
    }
}

fn attached_cluster_wait<E, Caps>(
    scenario: &Scenario<E, Caps>,
    client: Client,
) -> Result<Arc<dyn ClusterWaitHandle<E>>, K8sRunnerError>
where
    E: K8sDeployEnv,
    Caps: Send + Sync,
{
    let attach = scenario
        .existing_cluster()
        .ok_or_else(|| K8sRunnerError::InternalInvariant {
            message: "k8s attached cluster wait requested outside attached source mode".to_owned(),
        })?;

    Ok(Arc::new(K8sAttachedClusterWait::<E>::new(
        client,
        attach.clone(),
    )))
}

fn managed_cluster_wait<E: K8sDeployEnv>(
    cluster: &Option<ClusterEnvironment>,
    metadata: &K8sDeploymentMetadata,
) -> Result<Arc<dyn ClusterWaitHandle<E>>, K8sRunnerError> {
    let client = client_from_cluster(cluster)?;
    let attach_source = metadata
        .attach_source()
        .map_err(|source| K8sRunnerError::SourceOrchestration { source })?;

    Ok(Arc::new(K8sAttachedClusterWait::<E>::new(
        client,
        attach_source,
    )))
}

fn client_from_cluster(cluster: &Option<ClusterEnvironment>) -> Result<Client, K8sRunnerError> {
    let client = cluster
        .as_ref()
        .ok_or_else(|| K8sRunnerError::InternalInvariant {
            message: "cluster must exist while resolving source providers".to_owned(),
        })?
        .client()
        .clone();

    Ok(client)
}

fn source_providers<E: K8sDeployEnv>(
    client: Client,
    managed_clients: Vec<E::NodeClient>,
) -> SourceProviders<E> {
    SourceProviders::default()
        .with_managed(Arc::new(StaticManagedProvider::new(managed_clients)))
        .with_attach(Arc::new(K8sAttachProvider::<E>::new(client)))
        .with_external(Arc::new(ApplicationExternalProvider))
}

async fn resolve_node_clients<E: K8sDeployEnv>(
    source_plan: &SourceOrchestrationPlan,
    source_providers: SourceProviders<E>,
) -> Result<NodeClients<E>, K8sRunnerError> {
    orchestrate_sources_with_providers(source_plan, source_providers)
        .await
        .map_err(|source| K8sRunnerError::SourceOrchestration { source })
}

fn ensure_non_empty_node_clients<E: K8sDeployEnv>(
    node_clients: &NodeClients<E>,
) -> Result<(), K8sRunnerError> {
    if node_clients.is_empty() {
        return Err(K8sRunnerError::RuntimePreflight);
    }

    Ok(())
}

struct BuiltK8sDeployment {
    cluster: ClusterEnvironment,
    node_count: usize,
}

async fn build_k8s_deployment<E, Caps>(
    deployer: &K8sDeployer<E>,
    scenario: &Scenario<E, Caps>,
    observability: &ObservabilityInputs,
) -> Result<BuiltK8sDeployment, K8sRunnerError>
where
    E: K8sDeployEnv,
    Caps: ObservabilityCapabilityProvider,
{
    let descriptors = scenario.deployment();
    let node_count = descriptors.node_count();
    let deployment_policy = scenario.deployment_policy();
    ensure_supported_topology::<E>(descriptors)?;
    let client = init_kube_client().await?;

    log_k8s_deploy_start(
        deployer,
        scenario.duration(),
        node_count,
        deployment_policy.readiness_enabled,
        deployment_policy.readiness_requirement,
        observability,
    );

    let port_specs = collect_port_specs::<E>(descriptors);
    let cluster = setup_cluster::<E>(
        &client,
        &port_specs,
        descriptors,
        deployer.readiness_checks && deployment_policy.readiness_enabled,
        deployment_policy.readiness_requirement,
        observability,
    )
    .await?;

    Ok(BuiltK8sDeployment {
        cluster,
        node_count,
    })
}

async fn setup_cluster<E: K8sDeployEnv>(
    client: &Client,
    specs: &PortSpecs,
    descriptors: &E::Deployment,
    readiness_checks: bool,
    readiness_requirement: HttpReadinessRequirement,
    observability: &ObservabilityInputs,
) -> Result<ClusterEnvironment, K8sRunnerError> {
    let (setup, cleanup) = prepare_cluster_setup::<E>(client, descriptors, observability).await?;
    let mut cleanup_guard = Some(cleanup);

    info!("waiting for helm-managed services to become ready");
    let cluster_ready = wait_for_ports_or_cleanup::<E>(
        client,
        &setup.namespace,
        &setup.release,
        specs,
        &mut cleanup_guard,
    )
    .await?;
    let environment = build_cluster_environment(client, setup, cleanup_guard, cluster_ready)?;

    if readiness_checks {
        info!("probing cluster readiness");
        ensure_cluster_readiness::<E>(descriptors, &environment, readiness_requirement).await?;
        info!("cluster readiness probes passed");
    }

    Ok(environment)
}

struct ClusterSetup {
    namespace: String,
    release: String,
}

async fn prepare_cluster_setup<E: K8sDeployEnv>(
    client: &Client,
    descriptors: &E::Deployment,
    observability: &ObservabilityInputs,
) -> Result<(ClusterSetup, RunnerCleanup), K8sRunnerError> {
    let assets = E::prepare_assets(descriptors, observability.metrics_otlp_ingest_url.as_ref())
        .map_err(|source| K8sRunnerError::Assets { source })?;
    let nodes = descriptors.node_count();
    let (namespace, release) = E::cluster_identifiers();
    info!(%namespace, %release, nodes, "preparing k8s assets and namespace");

    let cleanup = E::install_stack(client, &assets, &namespace, &release, nodes)
        .await
        .map_err(|source| K8sRunnerError::InstallStack { source })?;

    Ok((ClusterSetup { namespace, release }, cleanup))
}

fn build_cluster_environment(
    client: &Client,
    setup: ClusterSetup,
    mut cleanup_guard: Option<RunnerCleanup>,
    cluster_ready: ClusterReady,
) -> Result<ClusterEnvironment, K8sRunnerError> {
    let cleanup = cleanup_guard
        .take()
        .ok_or_else(|| K8sRunnerError::InternalInvariant {
            message: "cleanup guard must exist after successful cluster startup".to_owned(),
        })?;

    Ok(ClusterEnvironment::new(
        client.clone(),
        setup.namespace,
        setup.release,
        cleanup,
        &cluster_ready.ports,
        cluster_ready.port_forwards,
    ))
}

fn resolve_observability_inputs(
    observability: &impl ObservabilityCapabilityProvider,
) -> Result<ObservabilityInputs, K8sRunnerError> {
    let env_inputs = ObservabilityInputs::from_env()?;
    let cap_inputs = observability
        .observability_capability()
        .map(ObservabilityInputs::from_capability)
        .unwrap_or_default();
    Ok(env_inputs.with_overrides(cap_inputs))
}

async fn init_kube_client() -> Result<Client, K8sRunnerError> {
    Client::try_default()
        .await
        .map_err(|source| K8sRunnerError::ClientInit { source })
}

async fn build_node_clients_or_fail<E: K8sDeployEnv>(
    cluster: &mut Option<ClusterEnvironment>,
) -> Result<NodeClients<E>, K8sRunnerError> {
    let environment = cluster
        .as_ref()
        .ok_or_else(|| K8sRunnerError::InternalInvariant {
            message: "cluster must be available while building clients".to_owned(),
        })?;

    match build_node_clients::<E>(environment) {
        Ok(clients) => Ok(clients),
        Err(err) => {
            fail_cluster(cluster, "failed to construct node api clients").await;
            error!(error = ?err, "failed to build k8s node clients");
            Err(err.into())
        }
    }
}

struct RuntimeArtifacts<E: K8sDeployEnv> {
    node_clients: NodeClients<E>,
    telemetry: Metrics,
    feed: Feed<E>,
    feed_task: FeedHandle,
}

fn build_runner_parts<E: K8sDeployEnv, Caps>(
    scenario: &Scenario<E, Caps>,
    node_count: usize,
    runtime: RuntimeArtifacts<E>,
    cluster_wait: Arc<dyn ClusterWaitHandle<E>>,
) -> K8sRunnerParts<E> {
    K8sRunnerParts {
        descriptors: scenario.deployment().clone(),
        node_clients: runtime.node_clients,
        duration: scenario.duration(),
        expectation_cooldown: scenario.expectation_cooldown(),
        telemetry: runtime.telemetry,
        feed: runtime.feed,
        feed_task: runtime.feed_task,
        node_count,
        cluster_wait,
    }
}

async fn build_runtime_artifacts<E: K8sDeployEnv>(
    cluster: &mut Option<ClusterEnvironment>,
    observability: &ObservabilityInputs,
) -> Result<RuntimeArtifacts<E>, K8sRunnerError> {
    info!("building node clients");
    let node_clients = build_node_clients_or_fail::<E>(cluster).await?;
    if node_clients.is_empty() {
        fail_cluster(
            cluster,
            "runtime preflight failed: no node clients available",
        )
        .await;
        return Err(K8sRunnerError::RuntimePreflight);
    }

    let telemetry = build_telemetry_or_fail(cluster, observability).await?;
    let (feed, feed_task) = spawn_block_feed_or_fail::<E>(cluster, &node_clients).await?;

    Ok(RuntimeArtifacts {
        node_clients,
        telemetry,
        feed,
        feed_task,
    })
}

async fn build_telemetry_or_fail(
    cluster: &mut Option<ClusterEnvironment>,
    observability: &ObservabilityInputs,
) -> Result<Metrics, K8sRunnerError> {
    match observability.telemetry_handle() {
        Ok(handle) => Ok(handle),
        Err(err) => {
            fail_cluster_with_log(
                cluster,
                "failed to configure metrics telemetry handle",
                &err,
            )
            .await;
            Err(err.into())
        }
    }
}

async fn spawn_block_feed_or_fail<E: K8sDeployEnv>(
    cluster: &mut Option<ClusterEnvironment>,
    node_clients: &NodeClients<E>,
) -> Result<(Feed<E>, FeedHandle), K8sRunnerError> {
    match spawn_block_feed_with::<E>(node_clients).await {
        Ok(pair) => Ok(pair),
        Err(err) => {
            fail_cluster_with_log(cluster, "failed to initialize block feed", &err).await;
            Err(err)
        }
    }
}

async fn fail_cluster_with_log<ErrorValue: Debug>(
    cluster: &mut Option<ClusterEnvironment>,
    reason: &str,
    error_value: &ErrorValue,
) {
    fail_cluster(cluster, reason).await;
    error!(error = ?error_value, "{reason}");
}

fn maybe_print_endpoints<E: K8sDeployEnv>(
    observability: &ObservabilityInputs,
    node_clients: &NodeClients<E>,
) {
    if env::var("TESTNET_PRINT_ENDPOINTS").is_err() {
        return;
    }

    println!(
        "TESTNET_ENDPOINTS prometheus={} grafana={}",
        endpoint_or_disabled(observability.metrics_query_url.as_ref()),
        endpoint_or_disabled(observability.grafana_url.as_ref())
    );

    print_node_pprof_endpoints::<E>(node_clients);
}

struct K8sRunnerParts<E: K8sDeployEnv> {
    descriptors: E::Deployment,
    node_clients: NodeClients<E>,
    duration: Duration,
    expectation_cooldown: Duration,
    telemetry: Metrics,
    feed: Feed<E>,
    feed_task: FeedHandle,
    node_count: usize,
    cluster_wait: Arc<dyn ClusterWaitHandle<E>>,
}

fn finalize_runner<E: K8sDeployEnv>(
    cluster: &mut Option<ClusterEnvironment>,
    parts: K8sRunnerParts<E>,
) -> Result<Runner<E>, K8sRunnerError> {
    let environment = take_ready_cluster(cluster)?;
    let (cleanup, port_forwards) = environment.into_cleanup()?;

    let K8sRunnerParts {
        descriptors,
        node_clients,
        duration,
        expectation_cooldown,
        telemetry,
        feed,
        feed_task,
        node_count,
        cluster_wait,
    } = parts;
    let duration_secs = duration.as_secs();

    let cleanup_guard: Box<dyn CleanupGuard> =
        Box::new(K8sCleanupGuard::new(cleanup, feed_task, port_forwards));
    let context = build_k8s_run_context(
        descriptors,
        node_clients,
        duration,
        expectation_cooldown,
        telemetry,
        feed,
        cluster_wait,
    );

    info!(
        nodes = node_count,
        duration_secs, "k8s deployment ready; handing control to scenario runner"
    );

    Ok(Runner::new(context, Some(cleanup_guard)))
}

fn take_ready_cluster(
    cluster: &mut Option<ClusterEnvironment>,
) -> Result<ClusterEnvironment, K8sRunnerError> {
    cluster
        .take()
        .ok_or_else(|| K8sRunnerError::InternalInvariant {
            message: "cluster should still be available".to_owned(),
        })
}

fn build_k8s_run_context<E: K8sDeployEnv>(
    descriptors: E::Deployment,
    node_clients: NodeClients<E>,
    duration: Duration,
    expectation_cooldown: Duration,
    telemetry: Metrics,
    feed: Feed<E>,
    cluster_wait: Arc<dyn ClusterWaitHandle<E>>,
) -> RunContext<E> {
    RunContext::new(
        descriptors,
        node_clients,
        duration,
        expectation_cooldown,
        telemetry,
        feed,
        None,
    )
    .with_cluster_wait(cluster_wait)
}

fn endpoint_or_disabled(endpoint: Option<&Url>) -> String {
    endpoint.map_or_else(
        || String::from(DISABLED_ENDPOINT),
        |url| String::from(url.as_str()),
    )
}

fn log_configured_observability(observability: &ObservabilityInputs) {
    if let Some(url) = observability.metrics_query_url.as_ref() {
        info!(metrics_query_url = %url.as_str(), "metrics query endpoint configured");
    }

    if let Some(url) = observability.grafana_url.as_ref() {
        info!(grafana_url = %url.as_str(), "grafana url configured");
    }
}

fn print_node_pprof_endpoints<E: K8sDeployEnv>(node_clients: &NodeClients<E>) {
    let nodes = node_clients.snapshot();

    for (idx, client) in nodes.iter().enumerate() {
        if let Some(base_url) = E::node_base_url(client) {
            println!(
                "TESTNET_PPROF node_{}={}/debug/pprof/profile?seconds=15&format=proto",
                idx, base_url
            );
        }
    }
}

fn log_k8s_deploy_start<E>(
    deployer: &K8sDeployer<E>,
    duration: Duration,
    node_count: usize,
    readiness_enabled: bool,
    readiness_requirement: HttpReadinessRequirement,
    observability: &ObservabilityInputs,
) where
    E: K8sDeployEnv,
{
    info!(
        nodes = node_count,
        duration_secs = duration.as_secs(),
        readiness_checks = deployer.readiness_checks,
        readiness_enabled,
        readiness_requirement = ?readiness_requirement,
        effective_readiness = deployer.readiness_checks && readiness_enabled,
        metrics_query_url = observability.metrics_query_url.as_ref().map(|u| u.as_str()),
        metrics_otlp_ingest_url = observability
            .metrics_otlp_ingest_url
            .as_ref()
            .map(|u| u.as_str()),
        grafana_url = observability.grafana_url.as_ref().map(|u| u.as_str()),
        "starting k8s deployment"
    );
}

struct K8sCleanupGuard {
    cleanup: RunnerCleanup,
    feed_task: Option<FeedHandle>,
    port_forwards: Vec<PortForwardHandle>,
}

impl K8sCleanupGuard {
    const fn new(
        cleanup: RunnerCleanup,
        feed_task: FeedHandle,
        port_forwards: Vec<PortForwardHandle>,
    ) -> Self {
        Self {
            cleanup,
            feed_task: Some(feed_task),
            port_forwards,
        }
    }
}

impl CleanupGuard for K8sCleanupGuard {
    fn cleanup(mut self: Box<Self>) {
        if let Some(feed_task) = self.feed_task.take() {
            CleanupGuard::cleanup(Box::new(feed_task));
        }
        kill_port_forwards(&mut self.port_forwards);
        CleanupGuard::cleanup(Box::new(self.cleanup));
    }
}

use std::{
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use testing_framework_core::{
    scenario::{
        Application, CleanupGuard, Deployer, DeploymentPolicy, DynError, FeedHandle, FeedRuntime,
        HttpReadinessRequirement, Metrics, NodeClients, NodeControlCapability, NodeControlHandle,
        RetryPolicy, RunContext, Runner, Scenario, ScenarioError, SourceOrchestrationPlan,
        build_source_orchestration_plan, spawn_feed,
    },
    topology::DeploymentDescriptor,
};
use thiserror::Error;
use tokio_retry::{
    RetryIf,
    strategy::{ExponentialBackoff, jitter},
};
use tracing::{debug, info, warn};

use crate::{
    env::{LocalDeployerEnv, Node, wait_local_http_readiness},
    external::build_external_client,
    keep_tempdir_from_env,
    manual::ManualCluster,
    node_control::{NodeManager, NodeManagerSeed},
};

const READINESS_ATTEMPTS: usize = 3;
const READINESS_BACKOFF_BASE_MS: u64 = 250;
const READINESS_BACKOFF_MAX_SECS: u64 = 2;

struct LocalProcessGuard<E: LocalDeployerEnv> {
    nodes: Vec<Node<E>>,
    feed_task: Option<FeedHandle>,
}

impl<E: LocalDeployerEnv> LocalProcessGuard<E> {
    fn new(nodes: Vec<Node<E>>, feed_task: FeedHandle) -> Self {
        Self {
            nodes,
            feed_task: Some(feed_task),
        }
    }
}

impl<E: LocalDeployerEnv> CleanupGuard for LocalProcessGuard<E> {
    fn cleanup(mut self: Box<Self>) {
        if let Some(feed_task) = self.feed_task.take() {
            CleanupGuard::cleanup(Box::new(feed_task));
        }
        // Nodes own local processes; dropping them stops the processes.
        drop(self.nodes);
    }
}
/// Spawns nodes as local processes.
#[derive(Clone)]
pub struct ProcessDeployer<E: LocalDeployerEnv> {
    membership_check: bool,
    _env: PhantomData<E>,
}

/// Errors returned by the local deployer.
#[derive(Debug, Error)]
pub enum ProcessDeployerError {
    #[error("failed to spawn local topology: {source}")]
    Spawn {
        #[source]
        source: DynError,
    },
    #[error("readiness probe failed: {source}")]
    ReadinessFailed {
        #[source]
        source: DynError,
    },
    #[error("scenario topology is not supported by the local deployer")]
    UnsupportedTopology,
    #[error("workload failed: {source}")]
    WorkloadFailed {
        #[source]
        source: DynError,
    },
    #[error("runtime preflight failed: no node clients available")]
    RuntimePreflight,
    #[error("source orchestration failed: {source}")]
    SourceOrchestration {
        #[source]
        source: DynError,
    },
    #[error("expectations failed: {source}")]
    ExpectationsFailed {
        #[source]
        source: DynError,
    },
}

#[derive(Debug, Error)]
enum RetryAttemptError {
    #[error("failed to spawn local topology: {source}")]
    Spawn {
        #[source]
        source: DynError,
    },
    #[error("readiness probe failed: {source}")]
    Readiness {
        #[source]
        source: DynError,
    },
}

impl From<RetryAttemptError> for ProcessDeployerError {
    fn from(value: RetryAttemptError) -> Self {
        match value {
            RetryAttemptError::Spawn { source } => Self::Spawn { source },
            RetryAttemptError::Readiness { source } => Self::ReadinessFailed { source },
        }
    }
}

#[derive(Clone, Copy)]
struct RetryExecutionConfig {
    max_attempts: usize,
    keep_tempdir: bool,
    readiness_enabled: bool,
    readiness_requirement: HttpReadinessRequirement,
}

impl From<ScenarioError> for ProcessDeployerError {
    fn from(value: ScenarioError) -> Self {
        match value {
            ScenarioError::Workload(source) => Self::WorkloadFailed { source },
            ScenarioError::ExpectationCapture(source)
            | ScenarioError::ExpectationFailedDuringCapture(source)
            | ScenarioError::Expectations(source) => Self::ExpectationsFailed { source },
        }
    }
}

#[async_trait]
impl<E: LocalDeployerEnv> Deployer<E, ()> for ProcessDeployer<E> {
    type Error = ProcessDeployerError;

    async fn deploy(&self, scenario: &Scenario<E, ()>) -> Result<Runner<E>, Self::Error> {
        self.deploy_without_node_control(scenario).await
    }
}

#[async_trait]
impl<E: LocalDeployerEnv> Deployer<E, NodeControlCapability> for ProcessDeployer<E> {
    type Error = ProcessDeployerError;

    async fn deploy(
        &self,
        scenario: &Scenario<E, NodeControlCapability>,
    ) -> Result<Runner<E>, Self::Error> {
        self.deploy_with_node_control(scenario).await
    }
}

impl<E: LocalDeployerEnv> ProcessDeployer<E> {
    /// Construct a local deployer.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Enable or disable membership readiness checks.
    #[must_use]
    pub fn with_membership_check(mut self, enabled: bool) -> Self {
        self.membership_check = enabled;
        self
    }

    /// Build a manual cluster from a prepared topology descriptor.
    #[must_use]
    pub fn manual_cluster_from_descriptors(&self, descriptors: E::Deployment) -> ManualCluster<E> {
        ManualCluster::from_topology(descriptors)
    }

    async fn deploy_without_node_control(
        &self,
        scenario: &Scenario<E, ()>,
    ) -> Result<Runner<E>, ProcessDeployerError> {
        // Source planning is currently resolved here before node spawn/runtime setup.
        let source_plan = build_source_orchestration_plan(scenario).map_err(|source| {
            ProcessDeployerError::SourceOrchestration {
                source: source.into(),
            }
        })?;

        log_local_deploy_start(
            scenario.deployment().node_count(),
            scenario.deployment_policy(),
            false,
        );

        let nodes = Self::spawn_nodes_for_scenario(scenario, self.membership_check).await?;
        let node_clients = NodeClients::<E>::new(nodes.iter().map(|node| node.client()).collect());

        let node_clients = merge_source_clients_for_local::<E>(&source_plan, node_clients)
            .map_err(|source| ProcessDeployerError::SourceOrchestration { source })?;

        let runtime = run_context_for(
            scenario.deployment().clone(),
            node_clients,
            scenario.duration(),
            scenario.expectation_cooldown(),
            None,
        )
        .await?;

        let cleanup_guard: Box<dyn CleanupGuard> =
            Box::new(LocalProcessGuard::<E>::new(nodes, runtime.feed_task));

        Ok(Runner::new(runtime.context, Some(cleanup_guard)))
    }

    async fn deploy_with_node_control(
        &self,
        scenario: &Scenario<E, NodeControlCapability>,
    ) -> Result<Runner<E>, ProcessDeployerError> {
        // Source planning is currently resolved here before node spawn/runtime setup.
        let source_plan = build_source_orchestration_plan(scenario).map_err(|source| {
            ProcessDeployerError::SourceOrchestration {
                source: source.into(),
            }
        })?;

        log_local_deploy_start(
            scenario.deployment().node_count(),
            scenario.deployment_policy(),
            true,
        );

        let nodes = Self::spawn_nodes_for_scenario(scenario, self.membership_check).await?;
        let node_control = self.node_control_from(scenario, nodes);
        let node_clients =
            merge_source_clients_for_local::<E>(&source_plan, node_control.node_clients())
                .map_err(|source| ProcessDeployerError::SourceOrchestration { source })?;
        let runtime = run_context_for(
            scenario.deployment().clone(),
            node_clients,
            scenario.duration(),
            scenario.expectation_cooldown(),
            Some(node_control),
        )
        .await?;

        Ok(Runner::new(
            runtime.context,
            Some(Box::new(runtime.feed_task)),
        ))
    }

    fn node_control_from(
        &self,
        scenario: &Scenario<E, NodeControlCapability>,
        nodes: Vec<Node<E>>,
    ) -> Arc<NodeManager<E>> {
        let node_control = Arc::new(NodeManager::new_with_seed(
            scenario.deployment().clone(),
            NodeClients::default(),
            keep_tempdir(scenario.deployment_policy()),
            NodeManagerSeed::default(),
        ));
        node_control.initialize_with_nodes(nodes);
        node_control
    }

    async fn spawn_nodes_for_scenario<Caps>(
        scenario: &Scenario<E, Caps>,
        membership_check: bool,
    ) -> Result<Vec<Node<E>>, ProcessDeployerError> {
        info!(
            nodes = scenario.deployment().node_count(),
            "spawning local nodes"
        );
        Self::spawn_with_readiness_retry(
            scenario.deployment(),
            membership_check,
            scenario.deployment_policy(),
        )
        .await
    }

    async fn spawn_with_readiness_retry(
        descriptors: &E::Deployment,
        membership_check: bool,
        deployment_policy: DeploymentPolicy,
    ) -> Result<Vec<Node<E>>, ProcessDeployerError> {
        let (retry_policy, execution) =
            build_retry_execution_config(deployment_policy, membership_check);
        let attempts = Arc::new(AtomicUsize::new(0));
        let strategy = retry_backoff_strategy(retry_policy, execution.max_attempts);
        let operation = {
            let attempts = Arc::clone(&attempts);
            move || {
                let attempts = Arc::clone(&attempts);
                async move { run_retry_attempt::<E>(descriptors, execution, attempts).await }
            }
        };
        let should_retry = retry_decision(Arc::clone(&attempts), execution.max_attempts);

        let nodes = RetryIf::spawn(strategy, operation, should_retry).await?;
        Ok(nodes)
    }
}

fn merge_source_clients_for_local<E: LocalDeployerEnv>(
    source_plan: &SourceOrchestrationPlan,
    node_clients: NodeClients<E>,
) -> Result<NodeClients<E>, DynError> {
    for source in source_plan.external_sources() {
        let client =
            E::external_node_client(source).or_else(|_| build_external_client::<E>(source))?;
        node_clients.add_node(client);
    }
    Ok(node_clients)
}

fn build_retry_execution_config(
    deployment_policy: DeploymentPolicy,
    membership_check: bool,
) -> (RetryPolicy, RetryExecutionConfig) {
    let retry_policy = retry_policy_from(deployment_policy);
    let execution = RetryExecutionConfig {
        max_attempts: retry_policy.max_attempts.max(1),
        keep_tempdir: keep_tempdir(deployment_policy),
        readiness_enabled: deployment_policy.readiness_enabled && membership_check,
        readiness_requirement: deployment_policy.readiness_requirement,
    };

    (retry_policy, execution)
}

async fn run_retry_attempt<E: LocalDeployerEnv>(
    descriptors: &E::Deployment,
    execution: RetryExecutionConfig,
    attempts: Arc<AtomicUsize>,
) -> Result<Vec<Node<E>>, RetryAttemptError> {
    let attempt = attempts.fetch_add(1, Ordering::Relaxed) + 1;
    let nodes = spawn_nodes_for_attempt::<E>(descriptors, execution.keep_tempdir).await?;
    run_readiness_for_attempt::<E>(attempt, nodes, execution).await
}

fn retry_policy_from(deployment_policy: DeploymentPolicy) -> RetryPolicy {
    deployment_policy
        .retry_policy
        .unwrap_or_else(default_local_retry_policy)
}

fn retry_backoff_strategy(
    retry_policy: RetryPolicy,
    max_attempts: usize,
) -> impl Iterator<Item = Duration> {
    ExponentialBackoff::from_millis(retry_policy.base_delay.as_millis() as u64)
        .max_delay(retry_policy.max_delay)
        .map(jitter)
        .take(max_attempts.saturating_sub(1))
}

async fn spawn_nodes_for_attempt<E: LocalDeployerEnv>(
    descriptors: &E::Deployment,
    keep_tempdir: bool,
) -> Result<Vec<Node<E>>, RetryAttemptError> {
    NodeManager::<E>::spawn_initial_nodes(descriptors, keep_tempdir)
        .await
        .map_err(|source| RetryAttemptError::Spawn {
            source: source.into(),
        })
}

async fn run_readiness_for_attempt<E: LocalDeployerEnv>(
    attempt: usize,
    nodes: Vec<Node<E>>,
    execution: RetryExecutionConfig,
) -> Result<Vec<Node<E>>, RetryAttemptError> {
    if !execution.readiness_enabled {
        info!("skipping local readiness checks");
        return Ok(nodes);
    }

    match wait_local_http_readiness::<E>(&nodes, execution.readiness_requirement).await {
        Ok(()) => {
            info!(attempt, "local nodes are ready");
            Ok(nodes)
        }
        Err(source) => {
            let error: DynError = source.into();
            debug!(attempt, error = ?error, "local readiness failed");
            drop(nodes);
            Err(RetryAttemptError::Readiness { source: error })
        }
    }
}

fn retry_decision(
    attempts: Arc<AtomicUsize>,
    max_attempts: usize,
) -> impl FnMut(&RetryAttemptError) -> bool {
    move |error: &RetryAttemptError| {
        let attempt = attempts.load(Ordering::Relaxed);
        if attempt < max_attempts {
            warn!(
                attempt,
                max_attempts,
                error = %error,
                "local spawn/readiness failed; retrying with backoff"
            );
            true
        } else {
            false
        }
    }
}

impl<E: LocalDeployerEnv> Default for ProcessDeployer<E> {
    fn default() -> Self {
        Self {
            membership_check: true,
            _env: PhantomData,
        }
    }
}

const fn default_local_retry_policy() -> RetryPolicy {
    RetryPolicy::new(
        READINESS_ATTEMPTS,
        Duration::from_millis(READINESS_BACKOFF_BASE_MS),
        Duration::from_secs(READINESS_BACKOFF_MAX_SECS),
    )
}

fn keep_tempdir(policy: DeploymentPolicy) -> bool {
    policy.cleanup_policy.preserve_artifacts || keep_tempdir_from_env()
}

async fn spawn_feed_with<E: Application>(
    node_clients: &NodeClients<E>,
) -> Result<(<E::FeedRuntime as FeedRuntime>::Feed, FeedHandle), ProcessDeployerError> {
    debug!(
        nodes = node_clients.len(),
        "selecting node client for local feed"
    );

    let Some(block_source_client) = node_clients.random_client() else {
        return Err(ProcessDeployerError::WorkloadFailed {
            source: "feed requires at least one node".into(),
        });
    };

    info!("starting feed");

    spawn_feed::<E>(block_source_client)
        .await
        .map_err(workload_error)
}

fn workload_error(source: DynError) -> ProcessDeployerError {
    ProcessDeployerError::WorkloadFailed { source }
}

fn log_local_deploy_start(node_count: usize, policy: DeploymentPolicy, has_node_control: bool) {
    info!(
        nodes = node_count,
        node_control = has_node_control,
        readiness_enabled = policy.readiness_enabled,
        readiness_requirement = ?policy.readiness_requirement,
        "starting local deployment"
    );
}

struct RuntimeContext<E: Application> {
    context: RunContext<E>,
    feed_task: FeedHandle,
}

async fn run_context_for<E: Application>(
    descriptors: E::Deployment,
    node_clients: NodeClients<E>,
    duration: Duration,
    expectation_cooldown: Duration,
    node_control: Option<Arc<dyn NodeControlHandle<E>>>,
) -> Result<RuntimeContext<E>, ProcessDeployerError> {
    if node_clients.is_empty() {
        return Err(ProcessDeployerError::RuntimePreflight);
    }

    let (feed, feed_task) = spawn_feed_with::<E>(&node_clients).await?;
    let context = RunContext::new(
        descriptors,
        node_clients,
        duration,
        expectation_cooldown,
        Metrics::empty(),
        feed,
        node_control,
    );

    Ok(RuntimeContext { context, feed_task })
}

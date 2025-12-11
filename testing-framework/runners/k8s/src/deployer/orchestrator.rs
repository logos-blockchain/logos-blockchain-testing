use anyhow::Error;
use async_trait::async_trait;
use kube::Client;
use testing_framework_core::{
    scenario::{BlockFeedTask, CleanupGuard, Deployer, MetricsError, RunContext, Runner, Scenario},
    topology::generation::GeneratedTopology,
};
use tracing::{error, info};

use crate::{
    infrastructure::{
        assets::{AssetsError, prepare_assets},
        cluster::{
            ClusterEnvironment, NodeClientError, PortSpecs, RemoteReadinessError,
            build_node_clients, cluster_identifiers, collect_port_specs, ensure_cluster_readiness,
            install_stack, kill_port_forwards, metrics_handle_from_port, wait_for_ports_or_cleanup,
        },
        helm::HelmError,
    },
    lifecycle::{block_feed::spawn_block_feed_with, cleanup::RunnerCleanup},
    wait::ClusterWaitError,
};

/// Deploys a scenario into Kubernetes using Helm charts and port-forwards.
#[derive(Clone, Copy)]
pub struct K8sDeployer {
    readiness_checks: bool,
}

impl Default for K8sDeployer {
    fn default() -> Self {
        Self::new()
    }
}

impl K8sDeployer {
    #[must_use]
    /// Create a k8s deployer with readiness checks enabled.
    pub const fn new() -> Self {
        Self {
            readiness_checks: true,
        }
    }

    #[must_use]
    /// Enable/disable readiness probes before handing control to workloads.
    pub const fn with_readiness(mut self, enabled: bool) -> Self {
        self.readiness_checks = enabled;
        self
    }
}

#[derive(Debug, thiserror::Error)]
/// High-level runner failures returned to the scenario harness.
pub enum K8sRunnerError {
    #[error(
        "kubernetes runner requires at least one validator and one executor (validators={validators}, executors={executors})"
    )]
    UnsupportedTopology { validators: usize, executors: usize },
    #[error("failed to initialise kubernetes client: {source}")]
    ClientInit {
        #[source]
        source: kube::Error,
    },
    #[error(transparent)]
    Assets(#[from] AssetsError),
    #[error(transparent)]
    Helm(#[from] HelmError),
    #[error(transparent)]
    Cluster(#[from] Box<ClusterWaitError>),
    #[error(transparent)]
    Readiness(#[from] RemoteReadinessError),
    #[error(transparent)]
    NodeClients(#[from] NodeClientError),
    #[error(transparent)]
    Telemetry(#[from] MetricsError),
    #[error("k8s runner requires at least one node client to follow blocks")]
    BlockFeedMissing,
    #[error("failed to initialize block feed: {source}")]
    BlockFeed {
        #[source]
        source: Error,
    },
}

#[async_trait]
impl Deployer for K8sDeployer {
    type Error = K8sRunnerError;

    async fn deploy(&self, scenario: &Scenario) -> Result<Runner, Self::Error> {
        let descriptors = scenario.topology().clone();
        let validator_count = descriptors.validators().len();
        let executor_count = descriptors.executors().len();
        ensure_supported_topology(&descriptors)?;

        let client = Client::try_default()
            .await
            .map_err(|source| K8sRunnerError::ClientInit { source })?;
        info!(
            validators = validator_count,
            executors = executor_count,
            duration_secs = scenario.duration().as_secs(),
            readiness_checks = self.readiness_checks,
            "starting k8s deployment"
        );

        let port_specs = collect_port_specs(&descriptors);
        let mut cluster =
            Some(setup_cluster(&client, &port_specs, &descriptors, self.readiness_checks).await?);

        info!("building node clients");
        let node_clients = match build_node_clients(
            cluster
                .as_ref()
                .expect("cluster must be available while building clients"),
        ) {
            Ok(clients) => clients,
            Err(err) => {
                if let Some(env) = cluster.as_mut() {
                    env.fail("failed to construct node api clients").await;
                }
                error!(error = ?err, "failed to build k8s node clients");
                return Err(err.into());
            }
        };

        let telemetry = match metrics_handle_from_port(
            cluster
                .as_ref()
                .expect("cluster must be available for telemetry")
                .prometheus_port(),
        ) {
            Ok(handle) => handle,
            Err(err) => {
                if let Some(env) = cluster.as_mut() {
                    env.fail("failed to configure prometheus metrics handle")
                        .await;
                }
                error!(error = ?err, "failed to configure prometheus metrics handle");
                return Err(err.into());
            }
        };
        let (block_feed, block_feed_guard) = match spawn_block_feed_with(&node_clients).await {
            Ok(pair) => pair,
            Err(err) => {
                if let Some(env) = cluster.as_mut() {
                    env.fail("failed to initialize block feed").await;
                }
                error!(error = ?err, "failed to initialize block feed");
                return Err(err);
            }
        };
        let (cleanup, port_forwards) = cluster
            .take()
            .expect("cluster should still be available")
            .into_cleanup();
        let cleanup_guard: Box<dyn CleanupGuard> = Box::new(K8sCleanupGuard::new(
            cleanup,
            block_feed_guard,
            port_forwards,
        ));
        let context = RunContext::new(
            descriptors,
            None,
            node_clients,
            scenario.duration(),
            telemetry,
            block_feed,
            None,
        );
        info!(
            validators = validator_count,
            executors = executor_count,
            duration_secs = scenario.duration().as_secs(),
            "k8s deployment ready; handing control to scenario runner"
        );
        Ok(Runner::new(context, Some(cleanup_guard)))
    }
}

impl From<ClusterWaitError> for K8sRunnerError {
    fn from(value: ClusterWaitError) -> Self {
        Self::Cluster(Box::new(value))
    }
}

fn ensure_supported_topology(descriptors: &GeneratedTopology) -> Result<(), K8sRunnerError> {
    let validators = descriptors.validators().len();
    let executors = descriptors.executors().len();
    if validators == 0 || executors == 0 {
        return Err(K8sRunnerError::UnsupportedTopology {
            validators,
            executors,
        });
    }
    Ok(())
}

async fn setup_cluster(
    client: &Client,
    specs: &PortSpecs,
    descriptors: &GeneratedTopology,
    readiness_checks: bool,
) -> Result<ClusterEnvironment, K8sRunnerError> {
    let assets = prepare_assets(descriptors)?;
    let validators = descriptors.validators().len();
    let executors = descriptors.executors().len();

    let (namespace, release) = cluster_identifiers();
    info!(%namespace, %release, validators, executors, "preparing k8s assets and namespace");

    let mut cleanup_guard =
        Some(install_stack(client, &assets, &namespace, &release, validators, executors).await?);

    info!("waiting for helm-managed services to become ready");
    let cluster_ready =
        wait_for_ports_or_cleanup(client, &namespace, &release, specs, &mut cleanup_guard).await?;

    info!(
        prometheus_port = cluster_ready.ports.prometheus,
        "discovered prometheus endpoint"
    );

    let environment = ClusterEnvironment::new(
        client.clone(),
        namespace,
        release,
        cleanup_guard
            .take()
            .expect("cleanup guard must exist after successful cluster startup"),
        &cluster_ready.ports,
        cluster_ready.port_forwards,
    );

    if readiness_checks {
        info!("probing cluster readiness");
        ensure_cluster_readiness(descriptors, &environment).await?;
        info!("cluster readiness probes passed");
    }

    Ok(environment)
}

struct K8sCleanupGuard {
    cleanup: RunnerCleanup,
    block_feed: Option<BlockFeedTask>,
    port_forwards: Vec<std::process::Child>,
}

impl K8sCleanupGuard {
    const fn new(
        cleanup: RunnerCleanup,
        block_feed: BlockFeedTask,
        port_forwards: Vec<std::process::Child>,
    ) -> Self {
        Self {
            cleanup,
            block_feed: Some(block_feed),
            port_forwards,
        }
    }
}

impl CleanupGuard for K8sCleanupGuard {
    fn cleanup(mut self: Box<Self>) {
        if let Some(block_feed) = self.block_feed.take() {
            CleanupGuard::cleanup(Box::new(block_feed));
        }
        kill_port_forwards(&mut self.port_forwards);
        CleanupGuard::cleanup(Box::new(self.cleanup));
    }
}

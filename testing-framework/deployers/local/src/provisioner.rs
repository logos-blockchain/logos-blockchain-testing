use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use testing_framework_core::scenario::{
    ClusterControlProfile, ClusterControlRequest, ClusterProvisioner, ClusterRequest,
    ClusterSource, ClusterStartMode, ClusterUnit, DeploymentPolicy, DynError,
    HttpReadinessRequirement, NodeClients, ProvisionedCluster, RetryPolicy,
};
use tokio_retry::{
    RetryIf,
    strategy::{ExponentialBackoff, jitter},
};
use tracing::{debug, info, warn};

use crate::{
    LocalCluster, LocalDeployerEnv, NodeManager,
    env::{Node, wait_local_readiness},
    external::build_external_client,
    keep_tempdir_from_env,
};

const READINESS_ATTEMPTS: usize = 3;
const READINESS_BACKOFF_BASE_MS: u64 = 250;
const READINESS_BACKOFF_MAX_SECS: u64 = 2;

#[derive(Clone, Copy)]
struct RetryExecutionConfig {
    max_attempts: usize,
    keep_tempdir: bool,
    readiness_enabled: bool,
    readiness_requirement: HttpReadinessRequirement,
}

#[derive(Debug, thiserror::Error)]
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

#[derive(Debug, thiserror::Error)]
pub enum LocalClusterProvisionerError {
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
    #[error("source orchestration failed: {source}")]
    Source {
        #[source]
        source: DynError,
    },
    #[error("local cluster provisioner does not support attached clusters")]
    AttachedUnsupported,
}

impl From<RetryAttemptError> for LocalClusterProvisionerError {
    fn from(value: RetryAttemptError) -> Self {
        match value {
            RetryAttemptError::Spawn { source } => Self::Spawn { source },
            RetryAttemptError::Readiness { source } => Self::Readiness { source },
        }
    }
}

/// Local implementation of the shared cluster provisioning operation.
#[derive(Clone, Copy, Debug, Default)]
pub struct LocalClusterProvisioner;

/// Concrete local handle plus the backend-independent runtime surfaces.
pub type ProvisionedLocalCluster<E> = ProvisionedCluster<E, LocalCluster<E>>;

impl LocalClusterProvisioner {
    #[must_use]
    pub fn manual_cluster<E: LocalDeployerEnv>(
        &self,
        deployment: E::Deployment,
    ) -> LocalCluster<E> {
        LocalCluster::empty(deployment, keep_tempdir_from_env())
    }

    pub async fn provision<E: LocalDeployerEnv>(
        &self,
        request: ClusterRequest<E>,
        membership_check: bool,
    ) -> Result<ProvisionedLocalCluster<E>, LocalClusterProvisionerError> {
        match request.source().clone() {
            ClusterSource::Managed {
                deployment,
                external,
            } => {
                let keep_tempdir =
                    request.policy().cleanup_policy.preserve_artifacts || keep_tempdir_from_env();
                let cluster = LocalCluster::empty(deployment.clone(), keep_tempdir);

                if request.start_mode() == ClusterStartMode::Eager {
                    let nodes = spawn_with_readiness_retry::<E>(
                        &deployment,
                        membership_check,
                        request.policy(),
                    )
                    .await?;
                    cluster.initialize_with_nodes(nodes);
                }

                cluster
                    .add_external_sources(external)
                    .map_err(|source| LocalClusterProvisionerError::Source { source })?;

                let profile = if request.start_mode() == ClusterStartMode::OnDemand {
                    ClusterControlProfile::ManualControlled
                } else {
                    ClusterControlProfile::FrameworkManaged
                };
                let mut unit = ClusterUnit::new(Some(deployment), cluster.node_clients(), profile)
                    .with_cluster_wait(Arc::new(cluster.clone()))
                    .with_cleanup(cluster.cleanup_guard());

                if request.control() == ClusterControlRequest::Full {
                    unit = unit.with_node_control(Arc::new(cluster.clone()));
                }

                Ok(ProvisionedCluster::new(Some(cluster), unit))
            }
            ClusterSource::External { nodes } => {
                let clients = NodeClients::default();
                for source in nodes {
                    let client = E::external_node_client(&source)
                        .or_else(|_| build_external_client::<E>(&source))
                        .map_err(|source| LocalClusterProvisionerError::Source { source })?;
                    clients.add_node(client);
                }
                Ok(ProvisionedCluster::new(
                    None,
                    ClusterUnit::new(None, clients, ClusterControlProfile::ExternalUncontrolled),
                ))
            }
            ClusterSource::Attached { .. } => {
                Err(LocalClusterProvisionerError::AttachedUnsupported)
            }
        }
    }
}

#[async_trait::async_trait]
impl<E: LocalDeployerEnv> ClusterProvisioner<E> for LocalClusterProvisioner {
    async fn provision_cluster(
        &self,
        request: ClusterRequest<E>,
    ) -> Result<ClusterUnit<E>, DynError> {
        let (_, unit) = self
            .provision(request, true)
            .await
            .map_err(DynError::from)?
            .into_parts();
        Ok(unit)
    }
}

async fn spawn_with_readiness_retry<E: LocalDeployerEnv>(
    deployment: &E::Deployment,
    membership_check: bool,
    policy: DeploymentPolicy,
) -> Result<Vec<Node<E>>, LocalClusterProvisionerError> {
    let (retry_policy, execution) = build_retry_execution_config(policy, membership_check);
    let attempts = Arc::new(AtomicUsize::new(0));
    let strategy = ExponentialBackoff::from_millis(retry_policy.base_delay.as_millis() as u64)
        .max_delay(retry_policy.max_delay)
        .map(jitter)
        .take(execution.max_attempts.saturating_sub(1));
    let operation = {
        let attempts = Arc::clone(&attempts);
        move || {
            let attempts = Arc::clone(&attempts);
            async move { run_retry_attempt::<E>(deployment, execution, attempts).await }
        }
    };
    let should_retry = {
        let attempts = Arc::clone(&attempts);
        move |error: &RetryAttemptError| {
            let attempt = attempts.load(Ordering::Relaxed);
            if attempt < execution.max_attempts {
                warn!(
                    attempt,
                    max_attempts = execution.max_attempts,
                    error = %error,
                    "local spawn/readiness failed; retrying with backoff"
                );
                true
            } else {
                false
            }
        }
    };

    RetryIf::spawn(strategy, operation, should_retry)
        .await
        .map_err(Into::into)
}

fn build_retry_execution_config(
    policy: DeploymentPolicy,
    membership_check: bool,
) -> (RetryPolicy, RetryExecutionConfig) {
    let retry_policy = policy.retry_policy.unwrap_or_else(|| {
        RetryPolicy::new(
            READINESS_ATTEMPTS,
            Duration::from_millis(READINESS_BACKOFF_BASE_MS),
            Duration::from_secs(READINESS_BACKOFF_MAX_SECS),
        )
    });
    let execution = RetryExecutionConfig {
        max_attempts: retry_policy.max_attempts.max(1),
        keep_tempdir: policy.cleanup_policy.preserve_artifacts || keep_tempdir_from_env(),
        readiness_enabled: policy.readiness_enabled && membership_check,
        readiness_requirement: policy.readiness_requirement,
    };
    (retry_policy, execution)
}

async fn run_retry_attempt<E: LocalDeployerEnv>(
    deployment: &E::Deployment,
    execution: RetryExecutionConfig,
    attempts: Arc<AtomicUsize>,
) -> Result<Vec<Node<E>>, RetryAttemptError> {
    let attempt = attempts.fetch_add(1, Ordering::Relaxed) + 1;
    let nodes = NodeManager::<E>::spawn_initial_nodes(deployment, execution.keep_tempdir)
        .await
        .map_err(|source| RetryAttemptError::Spawn {
            source: source.into(),
        })?;

    if !execution.readiness_enabled {
        info!("skipping local readiness checks");
        return Ok(nodes);
    }

    match wait_local_readiness::<E>(&nodes, execution.readiness_requirement).await {
        Ok(()) => {
            info!(attempt, "local nodes are ready");
            Ok(nodes)
        }
        Err(source) => {
            let source: DynError = source.into();
            debug!(attempt, error = ?source, "local readiness failed");
            drop(nodes);
            Err(RetryAttemptError::Readiness { source })
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        path::Path,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };

    use testing_framework_core::{
        scenario::{
            Application, ClusterControlProfile, ClusterControlRequest, ClusterRequest,
            ClusterStartMode, DynError, ExistingCluster, ExternalNodeSource, StartNodeOptions,
        },
        topology::DeploymentDescriptor,
    };

    use super::{LocalClusterProvisioner, LocalClusterProvisionerError};
    use crate::{
        BuiltNodeConfig, LaunchSpec, LocalDeployerEnv, NodeConfigEntry, NodeEndpoints,
        ProcessSpawnError,
    };

    #[derive(Default)]
    struct LifecycleCalls {
        prepare: AtomicUsize,
        cleanup: AtomicUsize,
    }

    #[derive(Clone, Default)]
    struct EmptyDeployment {
        lifecycle: Arc<LifecycleCalls>,
    }

    impl DeploymentDescriptor for EmptyDeployment {
        fn node_count(&self) -> usize {
            0
        }
    }

    #[derive(Clone)]
    struct EmptyConfig;

    struct TestEnv;

    #[async_trait::async_trait]
    impl Application for TestEnv {
        type Deployment = EmptyDeployment;
        type NodeClient = String;
        type NodeConfig = EmptyConfig;

        fn external_node_client(source: &ExternalNodeSource) -> Result<Self::NodeClient, DynError> {
            if source.endpoint().is_empty() {
                return Err("empty test endpoint".into());
            }
            Ok(source.endpoint().to_owned())
        }
    }

    #[async_trait::async_trait]
    impl LocalDeployerEnv for TestEnv {
        fn prepare_local_cluster(deployment: &Self::Deployment) {
            deployment.lifecycle.prepare.fetch_add(1, Ordering::Relaxed);
        }

        fn cleanup_local_cluster(deployment: &Self::Deployment) {
            deployment.lifecycle.cleanup.fetch_add(1, Ordering::Relaxed);
        }

        fn build_node_config(
            _topology: &Self::Deployment,
            _index: usize,
            _peer_ports_by_name: &HashMap<String, u16>,
            _options: &StartNodeOptions<Self>,
            _peer_ports: &[u16],
        ) -> Result<BuiltNodeConfig<EmptyConfig>, DynError> {
            unreachable!("empty deployment never builds node configs")
        }

        fn build_initial_node_configs(
            _topology: &Self::Deployment,
        ) -> Result<Vec<NodeConfigEntry<EmptyConfig>>, ProcessSpawnError> {
            Ok(Vec::new())
        }

        async fn build_launch_spec(
            _config: &EmptyConfig,
            _dir: &Path,
            _label: &str,
        ) -> Result<LaunchSpec, DynError> {
            unreachable!("empty deployment never launches nodes")
        }

        fn node_endpoints(_config: &EmptyConfig) -> Result<NodeEndpoints, DynError> {
            unreachable!("empty deployment has no endpoints")
        }

        fn node_client(_endpoints: &NodeEndpoints) -> Result<Self::NodeClient, DynError> {
            unreachable!("empty deployment has no clients")
        }
    }

    #[tokio::test]
    async fn on_demand_managed_unit_exposes_shared_control_and_cleanup() {
        let deployment = EmptyDeployment::default();
        let lifecycle = Arc::clone(&deployment.lifecycle);
        let request = ClusterRequest::managed(deployment)
            .with_start_mode(ClusterStartMode::OnDemand)
            .with_control(ClusterControlRequest::Full);
        let provisioned = LocalClusterProvisioner
            .provision::<TestEnv>(request, true)
            .await
            .expect("manual unit should provision");
        let (cluster, mut unit) = provisioned.into_parts();
        let cluster = cluster.expect("managed unit should expose its concrete local handle");

        assert_eq!(lifecycle.prepare.load(Ordering::Relaxed), 1);
        assert_eq!(
            unit.control_profile(),
            ClusterControlProfile::ManualControlled
        );
        assert!(unit.node_control().is_some());
        assert!(unit.cluster_wait().is_some());

        unit.take_cleanup()
            .expect("managed unit should own cleanup")
            .cleanup();
        assert_eq!(lifecycle.cleanup.load(Ordering::Relaxed), 1);
        assert!(cluster.stop_all().is_err(), "cleanup must lock out clones");
        drop(cluster);
        assert_eq!(lifecycle.cleanup.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn provisioning_failure_cleans_up_prepared_cluster_once() {
        let deployment = EmptyDeployment::default();
        let lifecycle = Arc::clone(&deployment.lifecycle);
        let invalid_source = ExternalNodeSource::new("invalid".into(), String::new());
        let request = ClusterRequest::managed(deployment)
            .with_start_mode(ClusterStartMode::OnDemand)
            .with_external_nodes(vec![invalid_source]);

        let error = LocalClusterProvisioner
            .provision::<TestEnv>(request, true)
            .await
            .err()
            .expect("invalid external source should fail provisioning");

        assert!(matches!(error, LocalClusterProvisionerError::Source { .. }));
        assert_eq!(lifecycle.prepare.load(Ordering::Relaxed), 1);
        assert_eq!(lifecycle.cleanup.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn external_unit_is_borrowed_and_uncontrolled() {
        let source = ExternalNodeSource::new("external-0".into(), "http://node-0".into());
        let provisioned = LocalClusterProvisioner
            .provision::<TestEnv>(ClusterRequest::external(vec![source]), true)
            .await
            .expect("external unit should resolve");
        let (cluster, mut unit) = provisioned.into_parts();

        assert!(cluster.is_none());
        assert_eq!(unit.node_clients().snapshot(), vec!["http://node-0"]);
        assert_eq!(
            unit.control_profile(),
            ClusterControlProfile::ExternalUncontrolled
        );
        assert!(unit.node_control().is_none());
        assert!(unit.cluster_wait().is_none());
        assert!(unit.take_cleanup().is_none());
    }

    #[tokio::test]
    async fn attached_unit_reports_backend_support_gap() {
        let request = ClusterRequest::<TestEnv>::attached(ExistingCluster::for_compose_project(
            "existing".into(),
        ));
        let error = LocalClusterProvisioner
            .provision(request, true)
            .await
            .err()
            .expect("local attached provisioning should be rejected");

        assert_eq!(
            error.to_string(),
            "local cluster provisioner does not support attached clusters"
        );
    }
}

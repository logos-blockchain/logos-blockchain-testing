use std::{marker::PhantomData, sync::Arc, time::Duration};

use async_trait::async_trait;
use testing_framework_core::{
    scenario::{
        Application, ClusterControlProfile, ClusterControlRequest, ClusterMode, ClusterRequest,
        ClusterWaitHandle, Deployer, DeploymentPolicy, DynError, Metrics, NodeClients,
        NodeControlCapability, NodeControlHandle, Runner, RuntimeExtensions, Scenario,
        ScenarioError,
        internal::{CleanupGuard, RuntimeAssembly},
    },
    topology::DeploymentDescriptor,
};
use thiserror::Error;
use tracing::info;

use crate::{
    LocalClusterProvisioner, LocalClusterProvisionerError, env::LocalDeployerEnv,
    manual::ManualCluster,
};

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
    #[error("runtime extension setup failed: {source}")]
    RuntimeExtensions {
        #[source]
        source: DynError,
    },
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

impl From<LocalClusterProvisionerError> for ProcessDeployerError {
    fn from(value: LocalClusterProvisionerError) -> Self {
        match value {
            LocalClusterProvisionerError::Spawn { source } => Self::Spawn { source },
            LocalClusterProvisionerError::Readiness { source } => Self::ReadinessFailed { source },
            LocalClusterProvisionerError::Source { source } => Self::SourceOrchestration { source },
            LocalClusterProvisionerError::AttachedUnsupported => Self::SourceOrchestration {
                source: LocalClusterProvisionerError::AttachedUnsupported.into(),
            },
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
        validate_supported_cluster_mode(scenario)?;

        log_local_deploy_start(
            scenario.deployment().node_count(),
            scenario.deployment_policy(),
            false,
        );

        let provisioned = LocalClusterProvisioner
            .provision(
                cluster_request(scenario, ClusterControlRequest::None),
                self.membership_check,
            )
            .await?;
        let (_cluster, mut unit) = provisioned.into_parts();
        let node_clients = unit.node_clients().clone();

        let (runtime_extensions, runtime_cleanup) = scenario
            .prepare_runtime_extensions(node_clients.clone())
            .await
            .map_err(|source| ProcessDeployerError::RuntimeExtensions { source })?;

        let runtime = run_context_for(
            scenario.deployment().clone(),
            node_clients,
            scenario.duration(),
            scenario.expectation_cooldown(),
            unit.control_profile(),
            runtime_extensions,
            runtime_cleanup,
            unit.node_control(),
            unit.cluster_wait(),
        )
        .await?;
        Ok(runtime.assembly.build_runner(unit.take_cleanup()))
    }

    async fn deploy_with_node_control(
        &self,
        scenario: &Scenario<E, NodeControlCapability>,
    ) -> Result<Runner<E>, ProcessDeployerError> {
        validate_supported_cluster_mode(scenario)?;

        log_local_deploy_start(
            scenario.deployment().node_count(),
            scenario.deployment_policy(),
            true,
        );

        let provisioned = LocalClusterProvisioner
            .provision(
                cluster_request(scenario, ClusterControlRequest::Full),
                self.membership_check,
            )
            .await?;
        let (_cluster, mut unit) = provisioned.into_parts();
        let node_clients = unit.node_clients().clone();
        let (runtime_extensions, runtime_cleanup) = scenario
            .prepare_runtime_extensions(node_clients.clone())
            .await
            .map_err(|source| ProcessDeployerError::RuntimeExtensions { source })?;
        let runtime = run_context_for(
            scenario.deployment().clone(),
            node_clients,
            scenario.duration(),
            scenario.expectation_cooldown(),
            unit.control_profile(),
            runtime_extensions,
            runtime_cleanup,
            unit.node_control(),
            unit.cluster_wait(),
        )
        .await?;
        Ok(runtime.assembly.build_runner(unit.take_cleanup()))
    }
}

fn validate_supported_cluster_mode<E: Application, Caps>(
    scenario: &Scenario<E, Caps>,
) -> Result<(), ProcessDeployerError> {
    ensure_local_cluster_mode(scenario.cluster_mode())
}

fn ensure_local_cluster_mode(mode: ClusterMode) -> Result<(), ProcessDeployerError> {
    if matches!(mode, ClusterMode::ExistingCluster) {
        return Err(ProcessDeployerError::SourceOrchestration {
            source: DynError::from("local deployer does not support existing-cluster mode"),
        });
    }

    Ok(())
}

fn cluster_request<E: Application, Caps>(
    scenario: &Scenario<E, Caps>,
    control: ClusterControlRequest,
) -> ClusterRequest<E> {
    let request = match scenario.cluster_mode() {
        ClusterMode::Managed => ClusterRequest::managed(scenario.deployment().clone())
            .with_external_nodes(scenario.external_nodes().to_vec()),
        ClusterMode::ExistingCluster => ClusterRequest::attached(
            scenario
                .existing_cluster()
                .expect("existing-cluster mode must contain an attached source")
                .clone(),
        )
        .with_external_nodes(scenario.external_nodes().to_vec()),
        ClusterMode::ExternalOnly => ClusterRequest::external(scenario.external_nodes().to_vec()),
    };

    request
        .with_policy(scenario.deployment_policy())
        .with_control(control)
}

#[cfg(test)]
mod tests {
    use testing_framework_core::scenario::ClusterMode;

    use super::ensure_local_cluster_mode;

    #[test]
    fn local_cluster_validator_accepts_managed_mode() {
        ensure_local_cluster_mode(ClusterMode::Managed).expect("managed mode should be accepted");
    }

    #[test]
    fn local_cluster_validator_rejects_existing_cluster_mode() {
        let error = ensure_local_cluster_mode(ClusterMode::ExistingCluster)
            .expect_err("existing-cluster mode should be rejected");

        assert_eq!(
            error.to_string(),
            "source orchestration failed: local deployer does not support existing-cluster mode"
        );
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
    assembly: RuntimeAssembly<E>,
}

async fn run_context_for<E: Application>(
    descriptors: E::Deployment,
    node_clients: NodeClients<E>,
    duration: Duration,
    expectation_cooldown: Duration,
    cluster_control_profile: ClusterControlProfile,
    runtime_extensions: RuntimeExtensions,
    runtime_cleanup: Option<Box<dyn CleanupGuard>>,
    node_control: Option<Arc<dyn NodeControlHandle<E>>>,
    cluster_wait: Option<Arc<dyn ClusterWaitHandle<E>>>,
) -> Result<RuntimeContext<E>, ProcessDeployerError> {
    if node_clients.is_empty() && runtime_extensions.is_empty() {
        return Err(ProcessDeployerError::RuntimePreflight);
    }

    let mut assembly = RuntimeAssembly::new(
        descriptors,
        node_clients,
        duration,
        expectation_cooldown,
        cluster_control_profile,
        Metrics::empty(),
    )
    .with_runtime_extensions(runtime_extensions)
    .with_cleanup_guard(runtime_cleanup);
    if let Some(node_control) = node_control {
        assembly = assembly.with_node_control(node_control);
    }
    if let Some(cluster_wait) = cluster_wait {
        assembly = assembly.with_cluster_wait(cluster_wait);
    }

    Ok(RuntimeContext { assembly })
}

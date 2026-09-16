use std::{sync::Arc, time::Duration};

use thiserror::Error;

use super::builder::Builder;
use crate::{
    scenario::{
        Application, ClusterControlSummary, DeploymentPolicy, DynError, HttpReadinessRequirement,
        NodeClients,
        expectation::Expectation,
        runtime::{
            CleanupGuard, RuntimeExtensionFactory, RuntimeExtensions, prepare_runtime_extensions,
        },
        workload::Workload,
    },
    topology::DynTopologyError,
};

#[derive(Debug, Error)]
pub enum ScenarioBuildError {
    #[error("topology build failed: {0}")]
    Topology(#[source] DynTopologyError),
    #[error("workload '{name}' failed to initialize")]
    WorkloadInit { name: String, source: DynError },
    #[error("expectation '{name}' failed to initialize")]
    ExpectationInit { name: String, source: DynError },
}

/// Immutable scenario definition used by the runner, workloads, and
/// expectations.
pub struct Scenario<E: Application> {
    deployment: E::Deployment,
    workloads: Vec<Arc<dyn Workload<E>>>,
    expectations: Vec<Box<dyn Expectation<E>>>,
    runtime_extensions: Vec<Box<dyn RuntimeExtensionFactory<E>>>,
    duration: Duration,
    expectation_cooldown: Duration,
    deployment_policy: DeploymentPolicy,
}

impl<E: Application> Scenario<E> {
    pub(super) fn new(
        deployment: E::Deployment,
        workloads: Vec<Arc<dyn Workload<E>>>,
        expectations: Vec<Box<dyn Expectation<E>>>,
        runtime_extensions: Vec<Box<dyn RuntimeExtensionFactory<E>>>,
        duration: Duration,
        expectation_cooldown: Duration,
        deployment_policy: DeploymentPolicy,
    ) -> Self {
        Self {
            deployment,
            workloads,
            expectations,
            runtime_extensions,
            duration,
            expectation_cooldown,
            deployment_policy,
        }
    }

    #[must_use]
    pub fn deployment(&self) -> &E::Deployment {
        &self.deployment
    }

    #[must_use]
    pub fn workloads(&self) -> &[Arc<dyn Workload<E>>] {
        &self.workloads
    }

    #[must_use]
    pub fn expectations(&self) -> &[Box<dyn Expectation<E>>] {
        &self.expectations
    }

    #[must_use]
    pub fn expectations_mut(&mut self) -> &mut [Box<dyn Expectation<E>>] {
        &mut self.expectations
    }

    #[must_use]
    pub const fn duration(&self) -> Duration {
        self.duration
    }

    #[must_use]
    pub const fn expectation_cooldown(&self) -> Duration {
        self.expectation_cooldown
    }

    #[must_use]
    pub const fn http_readiness_requirement(&self) -> HttpReadinessRequirement {
        self.deployment_policy.readiness_requirement
    }

    #[must_use]
    pub const fn deployment_policy(&self) -> DeploymentPolicy {
        self.deployment_policy
    }

    #[doc(hidden)]
    pub async fn prepare_runtime_extensions(
        &self,
        node_clients: NodeClients<E>,
    ) -> Result<
        (
            RuntimeExtensions,
            Option<Box<dyn CleanupGuard>>,
            ClusterControlSummary,
        ),
        DynError,
    > {
        Ok(
            prepare_runtime_extensions(&self.runtime_extensions, &self.deployment, node_clients)
                .await?
                .into_parts(),
        )
    }
}

impl<E: Application> super::builder::ScenarioBuilder<E> {
    pub fn build(self) -> Result<Scenario<E>, ScenarioBuildError> {
        self.inner.build()
    }
}

impl<E: Application> Builder<E> {
    #[must_use]
    pub const fn run_duration(&self) -> Duration {
        self.duration
    }

    #[must_use]
    pub const fn expectation_cooldown_override(&self) -> Option<Duration> {
        self.expectation_cooldown
    }

    #[must_use]
    pub const fn http_readiness_requirement(&self) -> HttpReadinessRequirement {
        self.deployment_policy.readiness_requirement
    }

    #[must_use]
    pub const fn deployment_policy(&self) -> DeploymentPolicy {
        self.deployment_policy
    }
}

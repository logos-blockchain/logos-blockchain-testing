use std::{sync::Arc, time::Duration};

use thiserror::Error;

use super::builder::Builder;
use crate::{
    scenario::{
        Application, ClusterControlProfile, ClusterMode, DeploymentPolicy, DynError,
        ExistingCluster, ExternalNodeSource, HttpReadinessRequirement, expectation::Expectation,
        runtime::orchestration::SourceOrchestrationPlan, sources::ScenarioSources,
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
    #[error("invalid scenario source configuration: {message}")]
    SourceConfiguration { message: String },
    #[error("scenario source mode '{mode}' is not wired into deployers yet")]
    SourceModeNotWiredYet { mode: &'static str },
}

/// Immutable scenario definition used by the runner, workloads, and
/// expectations.
pub struct Scenario<E: Application, Caps = ()> {
    deployment: E::Deployment,
    workloads: Vec<Arc<dyn Workload<E>>>,
    expectations: Vec<Box<dyn Expectation<E>>>,
    duration: Duration,
    expectation_cooldown: Duration,
    deployment_policy: DeploymentPolicy,
    sources: ScenarioSources,
    source_orchestration_plan: SourceOrchestrationPlan,
    capabilities: Caps,
}

impl<E: Application, Caps> Scenario<E, Caps> {
    pub(super) fn new(
        deployment: E::Deployment,
        workloads: Vec<Arc<dyn Workload<E>>>,
        expectations: Vec<Box<dyn Expectation<E>>>,
        duration: Duration,
        expectation_cooldown: Duration,
        deployment_policy: DeploymentPolicy,
        sources: ScenarioSources,
        source_orchestration_plan: SourceOrchestrationPlan,
        capabilities: Caps,
    ) -> Self {
        Self {
            deployment,
            workloads,
            expectations,
            duration,
            expectation_cooldown,
            deployment_policy,
            sources,
            source_orchestration_plan,
            capabilities,
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

    #[must_use]
    pub fn existing_cluster(&self) -> Option<&ExistingCluster> {
        self.sources.existing_cluster()
    }

    #[must_use]
    pub const fn cluster_mode(&self) -> ClusterMode {
        self.sources.cluster_mode()
    }

    #[must_use]
    pub const fn cluster_control_profile(&self) -> ClusterControlProfile {
        self.sources.control_profile()
    }

    #[must_use]
    #[doc(hidden)]
    pub fn attached_source(&self) -> Option<&ExistingCluster> {
        self.existing_cluster()
    }

    #[must_use]
    pub fn external_nodes(&self) -> &[ExternalNodeSource] {
        self.sources.external_nodes()
    }

    #[must_use]
    pub fn has_external_nodes(&self) -> bool {
        !self.sources.external_nodes().is_empty()
    }

    #[must_use]
    pub const fn source_orchestration_plan(&self) -> &SourceOrchestrationPlan {
        &self.source_orchestration_plan
    }

    #[must_use]
    pub const fn capabilities(&self) -> &Caps {
        &self.capabilities
    }
}

impl<E: Application> super::builder::ScenarioBuilder<E> {
    pub fn build(self) -> Result<Scenario<E>, ScenarioBuildError> {
        self.inner.build()
    }
}

impl<E: Application> super::builder::NodeControlScenarioBuilder<E> {
    pub fn build(
        self,
    ) -> Result<Scenario<E, crate::scenario::NodeControlCapability>, ScenarioBuildError> {
        self.inner.build()
    }
}

impl<E: Application> super::builder::ObservabilityScenarioBuilder<E> {
    pub fn build(
        self,
    ) -> Result<Scenario<E, crate::scenario::ObservabilityCapability>, ScenarioBuildError> {
        self.inner.build()
    }
}

impl<E: Application, Caps> Builder<E, Caps> {
    #[must_use]
    pub const fn capabilities(&self) -> &Caps {
        &self.capabilities
    }

    #[must_use]
    pub const fn capabilities_mut(&mut self) -> &mut Caps {
        &mut self.capabilities
    }

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

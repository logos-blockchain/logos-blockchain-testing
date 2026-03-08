use std::{sync::Arc, time::Duration};

use thiserror::Error;
use tracing::{debug, info};

use super::{
    Application, ClusterMode, DeploymentPolicy, DynError, ExistingCluster, ExternalNodeSource,
    HttpReadinessRequirement, NodeControlCapability, ObservabilityCapability,
    builder_ops::CoreBuilderAccess,
    expectation::Expectation,
    runtime::{
        context::RunMetrics,
        orchestration::{SourceOrchestrationPlan, SourceOrchestrationPlanError},
    },
    sources::ScenarioSources,
    workload::Workload,
};
use crate::topology::{DeploymentDescriptor, DeploymentProvider, DeploymentSeed, DynTopologyError};

const MIN_EXPECTATION_FALLBACK_SECS: u64 = 10;
const MIN_RUN_DURATION_SECS: u64 = 10;

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
    fn new(
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
    #[doc(hidden)]
    pub fn attached_source(&self) -> Option<&ExistingCluster> {
        self.existing_cluster()
    }

    #[must_use]
    pub fn external_nodes(&self) -> &[ExternalNodeSource] {
        self.sources.external_nodes()
    }

    #[must_use]
    pub const fn uses_existing_cluster(&self) -> bool {
        matches!(self.cluster_mode(), ClusterMode::ExistingCluster)
    }

    #[must_use]
    pub const fn is_managed(&self) -> bool {
        matches!(self.cluster_mode(), ClusterMode::Managed)
    }

    #[must_use]
    pub const fn is_external_only(&self) -> bool {
        matches!(self.cluster_mode(), ClusterMode::ExternalOnly)
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

/// Scenario builder entry point.
pub struct Builder<E: Application, Caps = ()> {
    deployment_provider: Box<dyn DeploymentProvider<E::Deployment>>,
    topology_seed: Option<DeploymentSeed>,
    workloads: Vec<Box<dyn Workload<E>>>,
    expectations: Vec<Box<dyn Expectation<E>>>,
    duration: Duration,
    expectation_cooldown: Option<Duration>,
    deployment_policy: DeploymentPolicy,
    sources: ScenarioSources,
    capabilities: Caps,
}

pub struct ScenarioBuilder<E: Application> {
    inner: Builder<E, ()>,
}

pub struct NodeControlScenarioBuilder<E: Application> {
    inner: Builder<E, NodeControlCapability>,
}

pub struct ObservabilityScenarioBuilder<E: Application> {
    inner: Builder<E, ObservabilityCapability>,
}

macro_rules! impl_common_builder_methods {
    ($builder:ident) => {
        impl<E: Application> $builder<E> {
            #[must_use]
            pub fn map_deployment_provider(
                self,
                f: impl FnOnce(
                    Box<dyn DeploymentProvider<E::Deployment>>,
                ) -> Box<dyn DeploymentProvider<E::Deployment>>,
            ) -> Self {
                self.map_core_builder(|builder| builder.map_deployment_provider(f))
            }

            #[must_use]
            pub fn with_deployment_provider(
                self,
                deployment_provider: Box<dyn DeploymentProvider<E::Deployment>>,
            ) -> Self {
                self.map_core_builder(|builder| {
                    builder.with_deployment_provider(deployment_provider)
                })
            }

            #[must_use]
            pub fn with_deployment_seed(self, seed: DeploymentSeed) -> Self {
                self.map_core_builder(|builder| builder.with_deployment_seed(seed))
            }

            #[must_use]
            pub fn with_workload<W>(self, workload: W) -> Self
            where
                W: Workload<E> + 'static,
            {
                self.map_core_builder(|builder| builder.with_workload(workload))
            }

            #[must_use]
            pub fn with_workload_boxed(self, workload: Box<dyn Workload<E>>) -> Self {
                self.map_core_builder(|builder| builder.with_workload_boxed(workload))
            }

            #[must_use]
            pub fn with_expectation<Exp>(self, expectation: Exp) -> Self
            where
                Exp: Expectation<E> + 'static,
            {
                self.map_core_builder(|builder| builder.with_expectation(expectation))
            }

            #[must_use]
            pub fn with_expectation_boxed(self, expectation: Box<dyn Expectation<E>>) -> Self {
                self.map_core_builder(|builder| builder.with_expectation_boxed(expectation))
            }

            #[must_use]
            pub fn with_run_duration(self, duration: Duration) -> Self {
                self.map_core_builder(|builder| builder.with_run_duration(duration))
            }

            #[must_use]
            pub fn with_expectation_cooldown(self, cooldown: Duration) -> Self {
                self.map_core_builder(|builder| builder.with_expectation_cooldown(cooldown))
            }

            #[must_use]
            pub fn with_http_readiness_requirement(
                self,
                requirement: HttpReadinessRequirement,
            ) -> Self {
                self.map_core_builder(|builder| {
                    builder.with_http_readiness_requirement(requirement)
                })
            }

            #[must_use]
            pub fn with_deployment_policy(self, policy: DeploymentPolicy) -> Self {
                self.map_core_builder(|builder| builder.with_deployment_policy(policy))
            }

            #[must_use]
            pub fn with_existing_cluster(self, cluster: ExistingCluster) -> Self {
                self.map_core_builder(|builder| builder.with_existing_cluster(cluster))
            }

            #[must_use]
            #[doc(hidden)]
            pub fn with_attach_source(self, attach: ExistingCluster) -> Self {
                self.with_existing_cluster(attach)
            }

            #[must_use]
            pub fn with_external_node(self, node: ExternalNodeSource) -> Self {
                self.map_core_builder(|builder| builder.with_external_node(node))
            }

            #[must_use]
            pub fn with_external_only_sources(self) -> Self {
                self.map_core_builder(|builder| builder.with_external_only_sources())
            }

            #[must_use]
            pub fn run_duration(&self) -> Duration {
                self.core_builder_ref().run_duration()
            }
        }
    };
}

impl<E: Application> CoreBuilderAccess for ScenarioBuilder<E> {
    type Env = E;
    type Caps = ();

    fn map_core_builder(
        mut self,
        f: impl FnOnce(Builder<Self::Env, Self::Caps>) -> Builder<Self::Env, Self::Caps>,
    ) -> Self {
        self.inner = f(self.inner);
        self
    }

    fn core_builder_ref(&self) -> &Builder<Self::Env, Self::Caps> {
        &self.inner
    }

    fn core_builder_mut(&mut self) -> &mut Builder<Self::Env, Self::Caps> {
        &mut self.inner
    }
}

impl<E: Application> CoreBuilderAccess for NodeControlScenarioBuilder<E> {
    type Env = E;
    type Caps = NodeControlCapability;

    fn map_core_builder(
        mut self,
        f: impl FnOnce(Builder<Self::Env, Self::Caps>) -> Builder<Self::Env, Self::Caps>,
    ) -> Self {
        self.inner = f(self.inner);
        self
    }

    fn core_builder_ref(&self) -> &Builder<Self::Env, Self::Caps> {
        &self.inner
    }

    fn core_builder_mut(&mut self) -> &mut Builder<Self::Env, Self::Caps> {
        &mut self.inner
    }
}

impl<E: Application> CoreBuilderAccess for ObservabilityScenarioBuilder<E> {
    type Env = E;
    type Caps = ObservabilityCapability;

    fn map_core_builder(
        mut self,
        f: impl FnOnce(Builder<Self::Env, Self::Caps>) -> Builder<Self::Env, Self::Caps>,
    ) -> Self {
        self.inner = f(self.inner);
        self
    }

    fn core_builder_ref(&self) -> &Builder<Self::Env, Self::Caps> {
        &self.inner
    }

    fn core_builder_mut(&mut self) -> &mut Builder<Self::Env, Self::Caps> {
        &mut self.inner
    }
}

impl<E: Application, Caps: Default> Builder<E, Caps> {
    #[must_use]
    /// Start a builder from a topology provider.
    pub fn new(deployment_provider: Box<dyn DeploymentProvider<E::Deployment>>) -> Self {
        Self {
            deployment_provider,
            topology_seed: None,
            workloads: Vec::new(),
            expectations: Vec::new(),
            duration: Duration::ZERO,
            expectation_cooldown: None,
            deployment_policy: DeploymentPolicy::default(),
            sources: ScenarioSources::default(),
            capabilities: Caps::default(),
        }
    }
}

impl<E: Application> ScenarioBuilder<E> {
    #[must_use]
    pub fn new(deployment_provider: Box<dyn DeploymentProvider<E::Deployment>>) -> Self {
        Self {
            inner: Builder::new(deployment_provider),
        }
    }

    #[must_use]
    pub fn enable_node_control(self) -> NodeControlScenarioBuilder<E> {
        NodeControlScenarioBuilder {
            inner: self.inner.with_capabilities(NodeControlCapability),
        }
    }

    #[must_use]
    pub fn enable_observability(self) -> ObservabilityScenarioBuilder<E> {
        ObservabilityScenarioBuilder {
            inner: self
                .inner
                .with_capabilities(ObservabilityCapability::default()),
        }
    }

    pub fn build(self) -> Result<Scenario<E>, ScenarioBuildError> {
        self.inner.build()
    }

    pub(crate) fn with_observability(
        self,
        observability: ObservabilityCapability,
    ) -> ObservabilityScenarioBuilder<E> {
        ObservabilityScenarioBuilder {
            inner: self.inner.with_capabilities(observability),
        }
    }
}

impl_common_builder_methods!(ScenarioBuilder);

impl<E: Application> NodeControlScenarioBuilder<E> {
    pub fn build(self) -> Result<Scenario<E, NodeControlCapability>, ScenarioBuildError> {
        self.inner.build()
    }
}

impl_common_builder_methods!(NodeControlScenarioBuilder);

impl<E: Application> ObservabilityScenarioBuilder<E> {
    pub fn build(self) -> Result<Scenario<E, ObservabilityCapability>, ScenarioBuildError> {
        self.inner.build()
    }

    pub(crate) fn capabilities_mut(&mut self) -> &mut ObservabilityCapability {
        self.inner.capabilities_mut()
    }
}

impl_common_builder_methods!(ObservabilityScenarioBuilder);

impl<E: Application, Caps> Builder<E, Caps> {
    #[must_use]
    /// Transform the existing deployment provider while preserving all
    /// accumulated builder state.
    pub fn map_deployment_provider(
        mut self,
        f: impl FnOnce(
            Box<dyn DeploymentProvider<E::Deployment>>,
        ) -> Box<dyn DeploymentProvider<E::Deployment>>,
    ) -> Self {
        self.deployment_provider = f(self.deployment_provider);
        self
    }

    #[must_use]
    /// Replace the topology provider while preserving all accumulated builder
    /// state.
    pub fn with_deployment_provider(
        mut self,
        deployment_provider: Box<dyn DeploymentProvider<E::Deployment>>,
    ) -> Self {
        self.deployment_provider = deployment_provider;
        self
    }

    #[must_use]
    /// Internal capability transition helper.
    pub(crate) fn with_capabilities<NewCaps>(self, capabilities: NewCaps) -> Builder<E, NewCaps> {
        let Self {
            deployment_provider,
            topology_seed,
            workloads,
            expectations,
            duration,
            expectation_cooldown,
            deployment_policy,
            sources,
            ..
        } = self;

        Builder {
            deployment_provider,
            topology_seed,
            workloads,
            expectations,
            duration,
            expectation_cooldown,
            deployment_policy,
            sources,
            capabilities,
        }
    }

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

    #[must_use]
    pub fn with_deployment_seed(mut self, seed: DeploymentSeed) -> Self {
        self.topology_seed = Some(seed);
        self
    }

    #[must_use]
    pub fn with_workload<W>(mut self, workload: W) -> Self
    where
        W: Workload<E> + 'static,
    {
        self.add_workload(Box::new(workload));
        self
    }

    #[must_use]
    pub fn with_workload_boxed(mut self, workload: Box<dyn Workload<E>>) -> Self {
        self.add_workload(workload);
        self
    }

    #[must_use]
    /// Add a standalone expectation not tied to a workload.
    pub fn with_expectation<Exp>(mut self, expectation: Exp) -> Self
    where
        Exp: Expectation<E> + 'static,
    {
        self.add_expectation(Box::new(expectation));
        self
    }

    #[must_use]
    pub fn with_expectation_boxed(mut self, expectation: Box<dyn Expectation<E>>) -> Self {
        self.add_expectation(expectation);
        self
    }

    #[must_use]
    /// Configure the intended run duration.
    pub const fn with_run_duration(mut self, duration: Duration) -> Self {
        self.duration = duration;
        self
    }

    #[must_use]
    /// Override the expectation cooldown used by the runner.
    pub const fn with_expectation_cooldown(mut self, cooldown: Duration) -> Self {
        self.expectation_cooldown = Some(cooldown);
        self
    }

    #[must_use]
    pub const fn with_http_readiness_requirement(
        mut self,
        requirement: HttpReadinessRequirement,
    ) -> Self {
        self.deployment_policy.readiness_requirement = requirement;
        self
    }

    #[must_use]
    pub const fn with_deployment_policy(mut self, policy: DeploymentPolicy) -> Self {
        self.deployment_policy = policy;
        self
    }

    #[must_use]
    pub fn with_existing_cluster(mut self, cluster: ExistingCluster) -> Self {
        self.sources = self.sources.with_attach(cluster);
        self
    }

    #[must_use]
    #[doc(hidden)]
    pub fn with_attach_source(self, attach: ExistingCluster) -> Self {
        self.with_existing_cluster(attach)
    }

    #[must_use]
    pub fn with_external_node(mut self, node: ExternalNodeSource) -> Self {
        self.sources = self.sources.with_external_node(node);
        self
    }

    #[must_use]
    pub fn with_external_only_sources(mut self) -> Self {
        self.sources = self.sources.into_external_only();
        self
    }

    fn add_workload(&mut self, workload: Box<dyn Workload<E>>) {
        self.expectations.extend(workload.expectations());
        self.workloads.push(workload);
    }

    fn add_expectation(&mut self, expectation: Box<dyn Expectation<E>>) {
        self.expectations.push(expectation);
    }

    #[must_use]
    /// Finalize the scenario, computing run metrics and initializing
    /// components.
    pub fn build(self) -> Result<Scenario<E, Caps>, ScenarioBuildError> {
        let mut parts = BuilderParts::from_builder(self);
        let descriptors = parts.resolve_deployment()?;
        let run_plan = parts.run_plan();
        let run_metrics = RunMetrics::new(run_plan.duration);
        let source_orchestration_plan = build_source_orchestration_plan(parts.sources())?;

        initialize_components(
            &descriptors,
            &run_metrics,
            &mut parts.workloads,
            &mut parts.expectations,
        )?;
        let workloads: Vec<Arc<dyn Workload<E>>> =
            parts.workloads.into_iter().map(Arc::from).collect();

        info!(
            nodes = descriptors.node_count(),
            duration_secs = run_plan.duration.as_secs(),
            workloads = workloads.len(),
            expectations = parts.expectations.len(),
            "scenario built"
        );

        Ok(Scenario::new(
            descriptors,
            workloads,
            parts.expectations,
            run_plan.duration,
            run_plan.expectation_cooldown,
            parts.deployment_policy,
            parts.sources,
            source_orchestration_plan,
            parts.capabilities,
        ))
    }
}

struct RunPlan {
    duration: Duration,
    expectation_cooldown: Duration,
}

struct BuilderParts<E: Application, Caps> {
    deployment_provider: Box<dyn DeploymentProvider<E::Deployment>>,
    topology_seed: Option<DeploymentSeed>,
    workloads: Vec<Box<dyn Workload<E>>>,
    expectations: Vec<Box<dyn Expectation<E>>>,
    duration: Duration,
    expectation_cooldown: Option<Duration>,
    deployment_policy: DeploymentPolicy,
    sources: ScenarioSources,
    capabilities: Caps,
}

impl<E: Application, Caps> BuilderParts<E, Caps> {
    fn from_builder(builder: Builder<E, Caps>) -> Self {
        let Builder {
            deployment_provider,
            topology_seed,
            workloads,
            expectations,
            duration,
            expectation_cooldown,
            deployment_policy,
            sources,
            capabilities,
            ..
        } = builder;

        Self {
            deployment_provider,
            topology_seed,
            workloads,
            expectations,
            duration,
            expectation_cooldown,
            deployment_policy,
            sources,
            capabilities,
        }
    }

    fn resolve_deployment(&self) -> Result<E::Deployment, ScenarioBuildError> {
        self.deployment_provider
            .build(self.topology_seed.as_ref())
            .map_err(ScenarioBuildError::Topology)
    }

    fn run_plan(&self) -> RunPlan {
        RunPlan {
            duration: enforce_min_duration(self.duration),
            expectation_cooldown: expectation_cooldown_for(self.expectation_cooldown),
        }
    }

    fn sources(&self) -> &ScenarioSources {
        &self.sources
    }
}

fn build_source_orchestration_plan(
    sources: &ScenarioSources,
) -> Result<SourceOrchestrationPlan, ScenarioBuildError> {
    SourceOrchestrationPlan::try_from_sources(sources).map_err(source_plan_error_to_build_error)
}

fn source_plan_error_to_build_error(error: SourceOrchestrationPlanError) -> ScenarioBuildError {
    match error {
        SourceOrchestrationPlanError::SourceModeNotWiredYet { mode } => {
            ScenarioBuildError::SourceModeNotWiredYet { mode }
        }
    }
}

impl<E: Application> Builder<E, ()> {
    #[must_use]
    pub fn enable_node_control(self) -> Builder<E, NodeControlCapability> {
        self.with_capabilities(NodeControlCapability)
    }

    #[must_use]
    pub fn enable_observability(self) -> Builder<E, ObservabilityCapability> {
        self.with_capabilities(ObservabilityCapability::default())
    }
}

fn initialize_components<E: Application>(
    descriptors: &E::Deployment,
    run_metrics: &RunMetrics,
    workloads: &mut [Box<dyn Workload<E>>],
    expectations: &mut [Box<dyn Expectation<E>>],
) -> Result<(), ScenarioBuildError> {
    initialize_workloads(descriptors, run_metrics, workloads)?;

    initialize_expectations(descriptors, run_metrics, expectations)?;

    Ok(())
}

fn initialize_workloads<E: Application>(
    descriptors: &E::Deployment,
    run_metrics: &RunMetrics,
    workloads: &mut [Box<dyn Workload<E>>],
) -> Result<(), ScenarioBuildError> {
    for workload in workloads {
        debug!(workload = workload.name(), "initializing workload");
        let name = workload.name().to_owned();

        workload
            .init(descriptors, run_metrics)
            .map_err(|source| workload_init_error(name, source))?;
    }

    Ok(())
}

fn initialize_expectations<E: Application>(
    descriptors: &E::Deployment,
    run_metrics: &RunMetrics,
    expectations: &mut [Box<dyn Expectation<E>>],
) -> Result<(), ScenarioBuildError> {
    for expectation in expectations {
        debug!(expectation = expectation.name(), "initializing expectation");
        let name = expectation.name().to_owned();

        expectation
            .init(descriptors, run_metrics)
            .map_err(|source| expectation_init_error(name, source))?;
    }

    Ok(())
}

fn workload_init_error(name: String, source: DynError) -> ScenarioBuildError {
    ScenarioBuildError::WorkloadInit { name, source }
}

fn expectation_init_error(name: String, source: DynError) -> ScenarioBuildError {
    ScenarioBuildError::ExpectationInit { name, source }
}

fn enforce_min_duration(requested: Duration) -> Duration {
    let min_duration = min_run_duration();

    requested.max(min_duration)
}

fn default_expectation_cooldown() -> Duration {
    Duration::from_secs(MIN_EXPECTATION_FALLBACK_SECS)
}

fn expectation_cooldown_for(override_value: Option<Duration>) -> Duration {
    override_value.unwrap_or_else(default_expectation_cooldown)
}

fn min_run_duration() -> Duration {
    Duration::from_secs(MIN_RUN_DURATION_SECS)
}

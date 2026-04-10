use std::{sync::Arc, time::Duration};

use tracing::info;

use super::{
    model::{Scenario, ScenarioBuildError},
    validation::{
        build_source_orchestration_plan, enforce_min_duration, expectation_cooldown_for,
        initialize_components, validate_source_contract,
    },
};
use crate::{
    scenario::{
        Application, DeploymentPolicy, DynError, ExistingCluster, ExternalNodeSource,
        HttpReadinessRequirement, IntoExistingCluster, NodeControlCapability,
        ObservabilityCapability, RequiresNodeControl, builder_ops::CoreBuilderAccess,
        expectation::Expectation, runtime::context::RunMetrics, sources::ScenarioSources,
        workload::Workload,
    },
    topology::{DeploymentDescriptor, DeploymentProvider, DeploymentSeed, FixedDeploymentProvider},
};

/// Scenario builder entry point.
pub struct Builder<E: Application, Caps = ()> {
    pub(super) deployment_provider: Box<dyn DeploymentProvider<E::Deployment>>,
    pub(super) topology_seed: Option<DeploymentSeed>,
    pub(super) workloads: Vec<Box<dyn Workload<E>>>,
    pub(super) expectations: Vec<Box<dyn Expectation<E>>>,
    pub(super) duration: Duration,
    pub(super) expectation_cooldown: Option<Duration>,
    pub(super) deployment_policy: DeploymentPolicy,
    pub(super) sources: ScenarioSources,
    pub(super) capabilities: Caps,
}

pub struct ScenarioBuilder<E: Application> {
    pub(super) inner: Builder<E, ()>,
}

pub struct NodeControlScenarioBuilder<E: Application> {
    pub(super) inner: Builder<E, NodeControlCapability>,
}

pub struct ObservabilityScenarioBuilder<E: Application> {
    pub(super) inner: Builder<E, ObservabilityCapability>,
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
            pub fn with_existing_cluster_from(
                self,
                cluster: impl IntoExistingCluster,
            ) -> Result<Self, DynError> {
                let cluster = cluster.into_existing_cluster()?;

                Ok(self.with_existing_cluster(cluster))
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
            pub fn with_external_nodes(
                self,
                nodes: impl IntoIterator<Item = ExternalNodeSource>,
            ) -> Self {
                self.map_core_builder(|builder| builder.with_external_nodes(nodes))
            }

            #[must_use]
            pub fn with_external_only(self) -> Self {
                self.map_core_builder(|builder| builder.with_external_only())
            }

            #[must_use]
            pub fn with_external_only_nodes(
                self,
                nodes: impl IntoIterator<Item = ExternalNodeSource>,
            ) -> Self {
                self.map_core_builder(|builder| builder.with_external_only_nodes(nodes))
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
    pub fn with_deployment(deployment: E::Deployment) -> Self
    where
        E::Deployment: Clone + Send + Sync + 'static,
    {
        Self::new(Box::new(FixedDeploymentProvider::new(deployment)))
    }

    #[must_use]
    pub fn with_node_control(self) -> NodeControlScenarioBuilder<E> {
        NodeControlScenarioBuilder {
            inner: self.inner.with_capabilities(NodeControlCapability),
        }
    }

    #[must_use]
    #[doc(hidden)]
    pub fn enable_node_control(self) -> NodeControlScenarioBuilder<E> {
        self.with_node_control()
    }

    #[must_use]
    pub fn with_observability(self) -> ObservabilityScenarioBuilder<E> {
        ObservabilityScenarioBuilder {
            inner: self
                .inner
                .with_capabilities(ObservabilityCapability::default()),
        }
    }

    #[must_use]
    #[doc(hidden)]
    pub fn enable_observability(self) -> ObservabilityScenarioBuilder<E> {
        self.with_observability()
    }

    pub(crate) fn with_observability_capability(
        self,
        observability: ObservabilityCapability,
    ) -> ObservabilityScenarioBuilder<E> {
        ObservabilityScenarioBuilder {
            inner: self.inner.with_capabilities(observability),
        }
    }
}

impl_common_builder_methods!(ScenarioBuilder);
impl_common_builder_methods!(NodeControlScenarioBuilder);
impl_common_builder_methods!(ObservabilityScenarioBuilder);

impl<E: Application> ObservabilityScenarioBuilder<E> {
    pub(crate) fn capabilities_mut(&mut self) -> &mut ObservabilityCapability {
        self.inner.capabilities_mut()
    }
}

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
    pub fn with_existing_cluster_from(
        self,
        cluster: impl IntoExistingCluster,
    ) -> Result<Self, DynError> {
        let cluster = cluster.into_existing_cluster()?;

        Ok(self.with_existing_cluster(cluster))
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
    pub fn with_external_nodes(
        mut self,
        nodes: impl IntoIterator<Item = ExternalNodeSource>,
    ) -> Self {
        for node in nodes {
            self.sources = self.sources.with_external_node(node);
        }

        self
    }

    #[must_use]
    pub fn with_external_only(mut self) -> Self {
        self.sources = self.sources.into_external_only();
        self
    }

    #[must_use]
    pub fn with_external_only_nodes(
        self,
        nodes: impl IntoIterator<Item = ExternalNodeSource>,
    ) -> Self {
        self.with_external_only().with_external_nodes(nodes)
    }

    #[must_use]
    #[doc(hidden)]
    pub fn with_external_only_sources(self) -> Self {
        self.with_external_only()
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
    pub fn build(self) -> Result<Scenario<E, Caps>, ScenarioBuildError>
    where
        Caps: RequiresNodeControl,
    {
        let mut parts = BuilderParts::from_builder(self);
        let descriptors = parts.resolve_deployment()?;
        let run_plan = parts.run_plan();
        let run_metrics = RunMetrics::new(run_plan.duration);

        validate_source_contract::<Caps>(parts.sources())?;

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

impl<E: Application> Builder<E, ()> {
    #[must_use]
    pub fn with_node_control(self) -> Builder<E, NodeControlCapability> {
        self.with_capabilities(NodeControlCapability)
    }

    #[must_use]
    #[doc(hidden)]
    pub fn enable_node_control(self) -> Builder<E, NodeControlCapability> {
        self.with_node_control()
    }

    #[must_use]
    pub fn with_observability(self) -> Builder<E, ObservabilityCapability> {
        self.with_capabilities(ObservabilityCapability::default())
    }

    #[must_use]
    #[doc(hidden)]
    pub fn enable_observability(self) -> Builder<E, ObservabilityCapability> {
        self.with_observability()
    }
}

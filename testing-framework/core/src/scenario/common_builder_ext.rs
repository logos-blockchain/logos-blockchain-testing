use std::time::Duration;

use super::{
    Application, CleanupPolicy, DeploymentPolicy, Expectation, HttpReadinessRequirement,
    RetryPolicy, RuntimeExtensionFactory, Workload, internal::CoreBuilderAccess,
};
use crate::topology::{DeploymentProvider, DeploymentSeed};

type DeploymentProviderHandle<E> = Box<dyn DeploymentProvider<<E as Application>::Deployment>>;

/// Common fluent builder methods shared by all wrappers around `CoreBuilder`.
///
/// This lets app integrations reuse generic scenario behavior without
/// re-implementing forwarding methods in each app-specific builder.
pub trait CoreBuilderExt: CoreBuilderAccess + Sized {
    #[must_use]
    fn with_deployment_provider(
        self,
        deployment_provider: DeploymentProviderHandle<Self::Env>,
    ) -> Self {
        self.map_core_builder(|builder| builder.with_deployment_provider(deployment_provider))
    }

    #[must_use]
    fn with_deployment_seed(self, seed: DeploymentSeed) -> Self {
        self.map_core_builder(|builder| builder.with_deployment_seed(seed))
    }

    #[must_use]
    fn with_workload<W>(self, workload: W) -> Self
    where
        W: Workload<Self::Env> + 'static,
    {
        self.map_core_builder(|builder| builder.with_workload(workload))
    }

    #[must_use]
    fn with_workload_boxed(self, workload: Box<dyn Workload<Self::Env>>) -> Self {
        self.map_core_builder(|builder| builder.with_workload_boxed(workload))
    }

    #[must_use]
    fn with_expectation<Exp>(self, expectation: Exp) -> Self
    where
        Exp: Expectation<Self::Env> + 'static,
    {
        self.map_core_builder(|builder| builder.with_expectation(expectation))
    }

    #[must_use]
    fn with_expectation_boxed(self, expectation: Box<dyn Expectation<Self::Env>>) -> Self {
        self.map_core_builder(|builder| builder.with_expectation_boxed(expectation))
    }

    #[must_use]
    fn with_runtime_extension_factory(
        self,
        extension: Box<dyn RuntimeExtensionFactory<Self::Env>>,
    ) -> Self {
        self.map_core_builder(|builder| builder.with_runtime_extension_factory(extension))
    }

    #[must_use]
    fn with_run_duration(self, duration: Duration) -> Self {
        self.map_core_builder(|builder| builder.with_run_duration(duration))
    }

    #[must_use]
    fn with_expectation_cooldown(self, cooldown: Duration) -> Self {
        self.map_core_builder(|builder| builder.with_expectation_cooldown(cooldown))
    }

    #[must_use]
    fn with_http_readiness_requirement(self, requirement: HttpReadinessRequirement) -> Self {
        self.map_core_builder(|builder| builder.with_http_readiness_requirement(requirement))
    }

    #[must_use]
    fn with_deployment_policy(self, policy: DeploymentPolicy) -> Self {
        self.map_core_builder(|builder| builder.with_deployment_policy(policy))
    }

    #[must_use]
    fn with_readiness_enabled(self, enabled: bool) -> Self {
        self.map_core_builder(|builder| {
            let mut policy = builder.deployment_policy();
            policy.readiness_enabled = enabled;
            builder.with_deployment_policy(policy)
        })
    }

    #[must_use]
    fn with_http_readiness_all(self) -> Self {
        self.with_http_readiness_requirement(HttpReadinessRequirement::AllNodesReady)
    }

    #[must_use]
    fn with_http_readiness_any(self) -> Self {
        self.with_http_readiness_requirement(HttpReadinessRequirement::AnyNodeReady)
    }

    #[must_use]
    fn with_http_readiness_at_least(self, min_ready_nodes: usize) -> Self {
        self.with_http_readiness_requirement(HttpReadinessRequirement::AtLeast(min_ready_nodes))
    }

    #[must_use]
    fn with_retry_policy(self, retry: RetryPolicy) -> Self {
        self.map_core_builder(|builder| {
            let mut policy = builder.deployment_policy();
            policy.retry_policy = Some(retry);
            builder.with_deployment_policy(policy)
        })
    }

    #[must_use]
    fn without_retry_policy(self) -> Self {
        self.map_core_builder(|builder| {
            let mut policy = builder.deployment_policy();
            policy.retry_policy = None;
            builder.with_deployment_policy(policy)
        })
    }

    #[must_use]
    fn with_preserve_artifacts(self, preserve: bool) -> Self {
        self.map_core_builder(|builder| {
            let mut policy = builder.deployment_policy();
            policy.cleanup_policy = CleanupPolicy::new(preserve);
            builder.with_deployment_policy(policy)
        })
    }

    #[must_use]
    fn run_duration(&self) -> Duration {
        self.core_builder_ref().run_duration()
    }
}

impl<T: CoreBuilderAccess> CoreBuilderExt for T {}

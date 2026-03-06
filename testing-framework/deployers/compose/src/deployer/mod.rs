mod attach_provider;
pub mod clients;
pub mod orchestrator;
pub mod ports;
pub mod readiness;
pub mod setup;

use std::marker::PhantomData;

use async_trait::async_trait;
use testing_framework_core::scenario::{
    CleanupGuard, Deployer, FeedHandle, ObservabilityCapabilityProvider, RequiresNodeControl,
    Runner, Scenario,
};

use crate::{env::ComposeDeployEnv, errors::ComposeRunnerError, lifecycle::cleanup::RunnerCleanup};

/// Docker Compose-based deployer for test scenarios.
#[derive(Clone, Copy)]
pub struct ComposeDeployer<E: ComposeDeployEnv> {
    readiness_checks: bool,
    _env: PhantomData<E>,
}

/// Compose deployment metadata returned by compose-specific deployment APIs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ComposeDeploymentMetadata {
    /// Docker Compose project name used for this deployment when available.
    pub project_name: Option<String>,
}

impl<E: ComposeDeployEnv> Default for ComposeDeployer<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: ComposeDeployEnv> ComposeDeployer<E> {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            readiness_checks: true,
            _env: PhantomData,
        }
    }

    #[must_use]
    pub const fn with_readiness(mut self, enabled: bool) -> Self {
        self.readiness_checks = enabled;
        self
    }

    /// Deploy and return compose-specific metadata alongside the generic
    /// runner.
    pub async fn deploy_with_metadata<Caps>(
        &self,
        scenario: &Scenario<E, Caps>,
    ) -> Result<(Runner<E>, ComposeDeploymentMetadata), ComposeRunnerError>
    where
        Caps: RequiresNodeControl + ObservabilityCapabilityProvider + Send + Sync,
    {
        let deployer = Self {
            readiness_checks: self.readiness_checks,
            _env: PhantomData,
        };

        orchestrator::DeploymentOrchestrator::new(deployer)
            .deploy_with_metadata(scenario)
            .await
    }
}

#[async_trait]
impl<E, Caps> Deployer<E, Caps> for ComposeDeployer<E>
where
    Caps: RequiresNodeControl + ObservabilityCapabilityProvider + Send + Sync,
    E: ComposeDeployEnv,
{
    type Error = ComposeRunnerError;

    async fn deploy(&self, scenario: &Scenario<E, Caps>) -> Result<Runner<E>, Self::Error> {
        let deployer = Self {
            readiness_checks: self.readiness_checks,
            _env: PhantomData,
        };
        orchestrator::DeploymentOrchestrator::new(deployer)
            .deploy(scenario)
            .await
    }
}

pub(super) struct ComposeCleanupGuard {
    environment: RunnerCleanup,
    block_feed: Option<FeedHandle>,
}

impl ComposeCleanupGuard {
    const fn new(environment: RunnerCleanup, block_feed: FeedHandle) -> Self {
        Self {
            environment,
            block_feed: Some(block_feed),
        }
    }
}

impl CleanupGuard for ComposeCleanupGuard {
    fn cleanup(mut self: Box<Self>) {
        if let Some(block_feed) = self.block_feed.take() {
            CleanupGuard::cleanup(Box::new(block_feed));
        }
        CleanupGuard::cleanup(Box::new(self.environment));
    }
}

pub(super) fn make_cleanup_guard(
    environment: RunnerCleanup,
    block_feed: FeedHandle,
) -> Box<dyn CleanupGuard> {
    Box::new(ComposeCleanupGuard::new(environment, block_feed))
}

#[cfg(test)]
mod tests {}

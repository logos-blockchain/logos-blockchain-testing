mod attach_provider;
pub mod clients;
pub mod orchestrator;
pub mod ports;
pub mod readiness;
pub mod setup;

use std::{marker::PhantomData, time::Duration};

use async_trait::async_trait;
use testing_framework_core::scenario::{
    AttachSource, CleanupGuard, Deployer, DynError, FeedHandle, ObservabilityCapabilityProvider,
    RequiresNodeControl, Runner, Scenario,
};
use tokio::time::sleep;

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

impl ComposeDeploymentMetadata {
    /// Returns project name when deployment is bound to a specific compose
    /// project.
    #[must_use]
    pub fn project_name(&self) -> Option<&str> {
        self.project_name.as_deref()
    }

    /// Builds an attach source for the same compose project.
    pub fn attach_source_for_services(
        &self,
        services: Vec<String>,
    ) -> Result<AttachSource, DynError> {
        let Some(project_name) = self.project_name() else {
            return Err("compose metadata has no project name".into());
        };

        Ok(AttachSource::compose(services).with_project(project_name.to_owned()))
    }

    /// Discovers compose node services and builds an attach source for them.
    pub async fn attach_source_for_discovered_services(&self) -> Result<AttachSource, DynError> {
        let services = self.discover_services().await?;
        self.attach_source_for_services(services)
    }

    /// Discovers node services for this compose deployment.
    pub async fn discover_services(&self) -> Result<Vec<String>, DynError> {
        let Some(project_name) = self.project_name() else {
            return Err("compose metadata has no project name".into());
        };

        crate::docker::attached::discover_attachable_services(project_name).await
    }

    /// Returns the current StartedAt timestamp for a compose service container.
    pub async fn service_started_at(&self, service: &str) -> Result<String, DynError> {
        let Some(project_name) = self.project_name() else {
            return Err("compose metadata has no project name".into());
        };

        let container_id =
            crate::docker::attached::discover_service_container_id(project_name, service).await?;
        let started_at = crate::docker::attached::run_docker_capture([
            "inspect",
            "--format",
            "{{.State.StartedAt}}",
            &container_id,
        ])
        .await?;
        let started_at = started_at.trim();

        if started_at.is_empty() {
            return Err(format!(
                "docker inspect returned empty StartedAt for compose service '{service}'"
            )
            .into());
        }

        Ok(started_at.to_owned())
    }

    /// Waits until a service container reports a different StartedAt timestamp.
    pub async fn wait_until_service_restarted(
        &self,
        service: &str,
        previous_started_at: &str,
        timeout: Duration,
    ) -> Result<(), DynError> {
        let deadline = std::time::Instant::now() + timeout;

        loop {
            let started_at = self.service_started_at(service).await?;
            if started_at != previous_started_at {
                return Ok(());
            }

            if std::time::Instant::now() >= deadline {
                return Err(
                    format!("timed out waiting for restarted compose service '{service}'").into(),
                );
            }

            sleep(Duration::from_millis(500)).await;
        }
    }
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

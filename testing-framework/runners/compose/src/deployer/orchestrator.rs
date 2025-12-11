use std::sync::Arc;

use testing_framework_core::scenario::{
    NodeControlHandle, RequiresNodeControl, RunContext, Runner, Scenario,
};
use tracing::info;

use super::{
    ComposeDeployer,
    clients::ClientBuilder,
    make_cleanup_guard,
    ports::PortManager,
    readiness::ReadinessChecker,
    setup::{DeploymentContext, DeploymentSetup},
};
use crate::{
    docker::control::ComposeNodeControl,
    errors::ComposeRunnerError,
    infrastructure::{environment::StackEnvironment, ports::compose_runner_host},
    lifecycle::readiness::metrics_handle_from_port,
};

pub struct DeploymentOrchestrator {
    deployer: ComposeDeployer,
}

impl DeploymentOrchestrator {
    pub const fn new(deployer: ComposeDeployer) -> Self {
        Self { deployer }
    }

    pub async fn deploy<Caps>(
        &self,
        scenario: &Scenario<Caps>,
    ) -> Result<Runner, ComposeRunnerError>
    where
        Caps: RequiresNodeControl + Send + Sync,
    {
        let setup = DeploymentSetup::new(scenario.topology());
        setup.validate_environment().await?;

        let DeploymentContext {
            mut environment,
            descriptors,
        } = setup.prepare_workspace().await?;

        tracing::info!(
            validators = descriptors.validators().len(),
            executors = descriptors.executors().len(),
            duration_secs = scenario.duration().as_secs(),
            readiness_checks = self.deployer.readiness_checks,
            "compose deployment starting"
        );

        let validator_count = descriptors.validators().len();
        let executor_count = descriptors.executors().len();
        let host_ports = PortManager::prepare(&mut environment, &descriptors).await?;

        if self.deployer.readiness_checks {
            ReadinessChecker::wait_all(&descriptors, &host_ports, &mut environment).await?;
        } else {
            info!("readiness checks disabled; giving the stack a short grace period");
            crate::lifecycle::readiness::maybe_sleep_for_disabled_readiness(false).await;
        }

        let host = compose_runner_host();
        let client_builder = ClientBuilder::new();
        let node_clients = client_builder
            .build_node_clients(&descriptors, &host_ports, &host, &mut environment)
            .await?;
        let telemetry = metrics_handle_from_port(environment.prometheus_port(), &host)?;
        let node_control = self.maybe_node_control::<Caps>(&environment);

        let (block_feed, block_feed_guard) = client_builder
            .start_block_feed(&node_clients, &mut environment)
            .await?;
        let cleanup_guard = make_cleanup_guard(environment.into_cleanup(), block_feed_guard);

        let context = RunContext::new(
            descriptors,
            None,
            node_clients,
            scenario.duration(),
            telemetry,
            block_feed,
            node_control,
        );

        info!(
            validators = validator_count,
            executors = executor_count,
            duration_secs = scenario.duration().as_secs(),
            readiness_checks = self.deployer.readiness_checks,
            host,
            "compose deployment ready; handing control to scenario runner"
        );

        Ok(Runner::new(context, Some(cleanup_guard)))
    }

    fn maybe_node_control<Caps>(
        &self,
        environment: &StackEnvironment,
    ) -> Option<Arc<dyn NodeControlHandle>>
    where
        Caps: RequiresNodeControl + Send + Sync,
    {
        Caps::REQUIRED.then(|| {
            Arc::new(ComposeNodeControl {
                compose_file: environment.compose_path().to_path_buf(),
                project_name: environment.project_name().to_owned(),
            }) as Arc<dyn NodeControlHandle>
        })
    }
}

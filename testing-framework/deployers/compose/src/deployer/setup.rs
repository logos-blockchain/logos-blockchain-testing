use testing_framework_core::{scenario::ObservabilityInputs, topology::DeploymentDescriptor};
use tracing::info;

use crate::{
    docker::ensure_docker_available,
    env::ComposeDeployEnv,
    errors::ComposeRunnerError,
    infrastructure::environment::{
        StackEnvironment, ensure_supported_topology, prepare_environment,
    },
};

pub struct DeploymentSetup<'a, E: ComposeDeployEnv> {
    descriptors: &'a E::Deployment,
}

pub struct DeploymentContext<'a, E: ComposeDeployEnv> {
    pub descriptors: &'a E::Deployment,
    pub environment: StackEnvironment,
}

impl<'a, E: ComposeDeployEnv> DeploymentSetup<'a, E> {
    pub fn new(descriptors: &'a E::Deployment) -> Self {
        Self { descriptors }
    }

    pub async fn validate_environment(&self) -> Result<(), ComposeRunnerError> {
        ensure_docker_available().await?;
        ensure_supported_topology::<E>(self.descriptors)?;

        log_deployment_start(self.descriptors.node_count());

        Ok(())
    }

    pub async fn prepare_workspace(
        self,
        observability: &ObservabilityInputs,
    ) -> Result<DeploymentContext<'a, E>, ComposeRunnerError> {
        let metrics_otlp_ingest_url = observability.metrics_otlp_ingest_url.as_ref();
        let environment =
            prepare_environment::<E>(self.descriptors, metrics_otlp_ingest_url).await?;

        log_workspace_prepared(&environment);

        Ok(DeploymentContext {
            descriptors: self.descriptors,
            environment,
        })
    }
}

fn log_deployment_start(nodes: usize) {
    info!(nodes, "starting compose deployment");
}

fn log_workspace_prepared(environment: &StackEnvironment) {
    info!(
        compose_file = %environment.compose_path().display(),
        project = environment.project_name(),
        root = %environment.root().display(),
        "compose workspace prepared"
    );
}

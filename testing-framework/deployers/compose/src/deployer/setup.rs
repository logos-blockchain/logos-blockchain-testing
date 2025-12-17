use testing_framework_core::{
    scenario::ObservabilityInputs, topology::generation::GeneratedTopology,
};
use tracing::info;

use crate::{
    docker::ensure_docker_available,
    errors::ComposeRunnerError,
    infrastructure::environment::{
        StackEnvironment, ensure_supported_topology, prepare_environment,
    },
};

pub struct DeploymentSetup {
    descriptors: GeneratedTopology,
}

pub struct DeploymentContext {
    pub descriptors: GeneratedTopology,
    pub environment: StackEnvironment,
}

impl DeploymentSetup {
    pub fn new(descriptors: &GeneratedTopology) -> Self {
        Self {
            descriptors: descriptors.clone(),
        }
    }

    pub async fn validate_environment(&self) -> Result<(), ComposeRunnerError> {
        ensure_docker_available().await?;
        ensure_supported_topology(&self.descriptors)?;

        info!(
            validators = self.descriptors.validators().len(),
            executors = self.descriptors.executors().len(),
            "starting compose deployment"
        );

        Ok(())
    }

    pub async fn prepare_workspace(
        self,
        observability: &ObservabilityInputs,
    ) -> Result<DeploymentContext, ComposeRunnerError> {
        let environment = prepare_environment(
            &self.descriptors,
            observability.metrics_otlp_ingest_url.as_ref(),
        )
        .await?;

        info!(
            compose_file = %environment.compose_path().display(),
            project = environment.project_name(),
            root = %environment.root().display(),
            "compose workspace prepared"
        );

        Ok(DeploymentContext {
            descriptors: self.descriptors,
            environment,
        })
    }
}

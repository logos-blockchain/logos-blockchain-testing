use std::{
    env,
    net::{Ipv4Addr, TcpListener as StdTcpListener},
};

use testing_framework_core::topology::generation::GeneratedTopology;
use tracing::{debug, info};

use crate::{
    docker::ensure_docker_available,
    errors::ComposeRunnerError,
    infrastructure::environment::{
        PortReservation, StackEnvironment, ensure_supported_topology, prepare_environment,
    },
};

pub const PROMETHEUS_PORT_ENV: &str = "TEST_FRAMEWORK_PROMETHEUS_PORT";
pub const DEFAULT_PROMETHEUS_PORT: u16 = 9090;

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

    pub async fn prepare_workspace(self) -> Result<DeploymentContext, ComposeRunnerError> {
        let prometheus_env = env::var(PROMETHEUS_PORT_ENV)
            .ok()
            .and_then(|raw| raw.parse::<u16>().ok());
        if prometheus_env.is_some() {
            info!(port = prometheus_env, "using prometheus port from env");
        }
        let prometheus_port = prometheus_env
            .and_then(|port| reserve_port(port))
            .or_else(|| allocate_prometheus_port())
            .unwrap_or_else(|| PortReservation::new(DEFAULT_PROMETHEUS_PORT, None));
        debug!(
            prometheus_port = prometheus_port.port(),
            "selected prometheus port"
        );
        let environment =
            prepare_environment(&self.descriptors, prometheus_port, prometheus_env.is_some())
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

fn allocate_prometheus_port() -> Option<PortReservation> {
    reserve_port(DEFAULT_PROMETHEUS_PORT).or_else(|| reserve_port(0))
}

fn reserve_port(port: u16) -> Option<PortReservation> {
    let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, port)).ok()?;
    let actual_port = listener.local_addr().ok()?.port();
    Some(PortReservation::new(actual_port, Some(listener)))
}

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use testing_framework_core::{
    adjust_timeout,
    scenario::{Application, DynError, ExistingCluster, NodeControlHandle},
};
use tokio::{process::Command, time::timeout};
use tracing::info;

use crate::{
    docker::{
        attached::discover_service_container_id,
        commands::{ComposeCommandError, run_docker_command},
    },
    errors::ComposeRunnerError,
};

const COMPOSE_RESTART_TIMEOUT: Duration = Duration::from_secs(120);
const COMPOSE_RESTART_DESCRIPTION: &str = "docker compose restart";
const DOCKER_CONTAINER_RESTART_DESCRIPTION: &str = "docker container restart";
const DOCKER_CONTAINER_STOP_DESCRIPTION: &str = "docker container stop";

pub async fn restart_compose_service(
    compose_file: &Path,
    project_name: &str,
    service: &str,
) -> Result<(), ComposeRunnerError> {
    let command = compose_restart_command(compose_file, project_name, service);

    info!(
        service,
        project = project_name,
        compose_file = %compose_file.display(),
        "restarting compose service"
    );

    run_docker_command(
        command,
        adjust_timeout(COMPOSE_RESTART_TIMEOUT),
        COMPOSE_RESTART_DESCRIPTION,
    )
    .await
    .map_err(ComposeRunnerError::Compose)
}

pub async fn restart_attached_compose_service(
    project_name: &str,
    service: &str,
) -> Result<(), DynError> {
    let container_id = discover_service_container_id(project_name, service).await?;
    let command = docker_container_command("restart", &container_id);

    info!(
        service,
        project = project_name,
        container = container_id,
        "restarting attached compose service"
    );

    run_docker_action(
        command,
        DOCKER_CONTAINER_RESTART_DESCRIPTION,
        adjust_timeout(COMPOSE_RESTART_TIMEOUT),
    )
    .await
}

pub async fn stop_attached_compose_service(
    project_name: &str,
    service: &str,
) -> Result<(), DynError> {
    let container_id = discover_service_container_id(project_name, service).await?;
    let command = docker_container_command("stop", &container_id);

    info!(
        service,
        project = project_name,
        container = container_id,
        "stopping attached compose service"
    );

    run_docker_action(
        command,
        DOCKER_CONTAINER_STOP_DESCRIPTION,
        adjust_timeout(COMPOSE_RESTART_TIMEOUT),
    )
    .await
}

fn compose_restart_command(compose_file: &Path, project_name: &str, service: &str) -> Command {
    let mut command = Command::new("docker");
    command
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("-p")
        .arg(project_name)
        .arg("restart")
        .arg(service);
    command
}

fn docker_container_command(action: &str, container_id: &str) -> Command {
    let mut command = Command::new("docker");
    command.arg(action).arg(container_id);
    command
}

async fn run_docker_action(
    mut command: Command,
    description: &str,
    timeout_duration: Duration,
) -> Result<(), DynError> {
    match timeout(timeout_duration, command.output()).await {
        Ok(Ok(output)) => {
            if output.status.success() {
                return Ok(());
            }

            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();

            Err(format!(
                "{description} failed with status {}: {stderr}",
                output.status
            )
            .into())
        }
        Ok(Err(source)) => Err(format!("{description} failed to spawn: {source}").into()),
        Err(_) => {
            let compose_timeout = ComposeCommandError::Timeout {
                command: description.to_owned(),
                timeout: timeout_duration,
            };

            Err(compose_timeout.into())
        }
    }
}

/// Compose-specific node control handle for restarting nodes.
pub struct ComposeNodeControl {
    pub(crate) compose_file: PathBuf,
    pub(crate) project_name: String,
}

#[async_trait::async_trait]
impl<E: Application> NodeControlHandle<E> for ComposeNodeControl {
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        restart_compose_service(&self.compose_file, &self.project_name, name)
            .await
            .map_err(|err| format!("node restart failed: {err}").into())
    }
}

/// Node control handle for compose existing-cluster mode.
pub struct ComposeAttachedNodeControl {
    pub(crate) project_name: String,
}

impl ComposeAttachedNodeControl {
    pub fn try_from_existing_cluster(source: &ExistingCluster) -> Result<Self, DynError> {
        let Some(project_name) = source
            .compose_project()
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Err("attached compose node control requires explicit project name".into());
        };

        Ok(Self {
            project_name: project_name.to_owned(),
        })
    }
}

#[async_trait::async_trait]
impl<E: Application> NodeControlHandle<E> for ComposeAttachedNodeControl {
    async fn restart_node(&self, name: &str) -> Result<(), DynError> {
        restart_attached_compose_service(&self.project_name, name)
            .await
            .map_err(|source| format!("node restart failed for service '{name}': {source}").into())
    }

    async fn stop_node(&self, name: &str) -> Result<(), DynError> {
        stop_attached_compose_service(&self.project_name, name)
            .await
            .map_err(|source| format!("node stop failed for service '{name}': {source}").into())
    }
}

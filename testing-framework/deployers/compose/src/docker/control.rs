use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use testing_framework_core::{
    adjust_timeout,
    scenario::{Application, DynError, NodeControlHandle},
};
use tokio::process::Command;
use tracing::info;

use crate::{docker::commands::run_docker_command, errors::ComposeRunnerError};

const COMPOSE_RESTART_TIMEOUT: Duration = Duration::from_secs(120);
const COMPOSE_RESTART_DESCRIPTION: &str = "docker compose restart";

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

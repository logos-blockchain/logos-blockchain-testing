use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use testing_framework_core::scenario::{DynError, NodeControlHandle};
use tokio::process::Command;
use tracing::info;

use crate::{docker::commands::run_docker_command, errors::ComposeRunnerError};

const COMPOSE_RESTART_TIMEOUT: Duration = Duration::from_secs(120);

pub async fn restart_compose_service(
    compose_file: &Path,
    project_name: &str,
    service: &str,
) -> Result<(), ComposeRunnerError> {
    let mut command = Command::new("docker");
    command
        .arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("-p")
        .arg(project_name)
        .arg("restart")
        .arg(service);

    let description = "docker compose restart";
    info!(service, project = project_name, compose_file = %compose_file.display(), "restarting compose service");
    run_docker_command(
        command,
        testing_framework_core::adjust_timeout(COMPOSE_RESTART_TIMEOUT),
        description,
    )
    .await
    .map_err(ComposeRunnerError::Compose)
}

/// Compose-specific node control handle for restarting nodes.
pub struct ComposeNodeControl {
    pub(crate) compose_file: PathBuf,
    pub(crate) project_name: String,
}

#[async_trait::async_trait]
impl NodeControlHandle for ComposeNodeControl {
    async fn restart_node(&self, index: usize) -> Result<(), DynError> {
        restart_compose_service(
            &self.compose_file,
            &self.project_name,
            &format!("node-{index}"),
        )
        .await
        .map_err(|err| format!("node restart failed: {err}").into())
    }
}

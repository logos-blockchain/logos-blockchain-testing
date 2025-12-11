use std::{io, path::Path, process, time::Duration};

use testing_framework_core::adjust_timeout;
use tokio::{process::Command, time::timeout};
use tracing::{debug, info, warn};

const COMPOSE_UP_TIMEOUT: Duration = Duration::from_secs(120);

/// Errors running docker compose commands.
#[derive(Debug, thiserror::Error)]
pub enum ComposeCommandError {
    #[error("{command} exited with status {status}")]
    Failed {
        command: String,
        status: process::ExitStatus,
    },
    #[error("failed to spawn {command}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("{command} timed out after {timeout:?}")]
    Timeout { command: String, timeout: Duration },
}

/// Run an arbitrary docker command with a timeout.
pub async fn run_docker_command(
    mut command: Command,
    timeout_duration: Duration,
    description: &str,
) -> Result<(), ComposeCommandError> {
    debug!(description, ?command, "running docker command");
    let result = timeout(timeout_duration, command.status()).await;
    match result {
        Ok(status) => handle_compose_status(status, description),
        Err(_) => Err(ComposeCommandError::Timeout {
            command: description.to_owned(),
            timeout: timeout_duration,
        }),
    }
}

/// Runs `docker compose up -d` for the generated stack.
pub async fn compose_up(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
) -> Result<(), ComposeCommandError> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_path)
        .arg("-p")
        .arg(project_name)
        .arg("up")
        .arg("-d")
        .current_dir(root);

    info!(
        compose_file = %compose_path.display(),
        project = project_name,
        root = %root.display(),
        "running docker compose up"
    );

    run_compose_command(cmd, adjust_timeout(COMPOSE_UP_TIMEOUT), "docker compose up").await
}

/// Runs `docker compose down --volumes` for the generated stack.
pub async fn compose_down(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
) -> Result<(), ComposeCommandError> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_path)
        .arg("-p")
        .arg(project_name)
        .arg("down")
        .arg("--volumes")
        .current_dir(root);

    info!(
        compose_file = %compose_path.display(),
        project = project_name,
        root = %root.display(),
        "running docker compose down"
    );

    run_compose_command(
        cmd,
        adjust_timeout(COMPOSE_UP_TIMEOUT),
        "docker compose down",
    )
    .await
}

/// Dump docker compose logs to stderr for debugging failures.
pub async fn dump_compose_logs(compose_file: &Path, project: &str, root: &Path) {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("-p")
        .arg(project)
        .arg("logs")
        .arg("--no-color")
        .current_dir(root);

    match cmd.output().await {
        Ok(output) => print_logs(&output.stdout, &output.stderr),
        Err(err) => warn!(error = ?err, "failed to collect docker compose logs"),
    }
}

fn print_logs(stdout: &[u8], stderr: &[u8]) {
    if !stdout.is_empty() {
        warn!(
            logs = %String::from_utf8_lossy(stdout),
            "docker compose stdout"
        );
    }
    if !stderr.is_empty() {
        warn!(
            logs = %String::from_utf8_lossy(stderr),
            "docker compose stderr"
        );
    }
}

async fn run_compose_command(
    mut command: Command,
    timeout_duration: Duration,
    description: &str,
) -> Result<(), ComposeCommandError> {
    let result = timeout(timeout_duration, command.status()).await;
    match result {
        Ok(status) => handle_compose_status(status, description),
        Err(_) => Err(ComposeCommandError::Timeout {
            command: description.to_owned(),
            timeout: timeout_duration,
        }),
    }
}

fn handle_compose_status(
    status: std::io::Result<std::process::ExitStatus>,
    description: &str,
) -> Result<(), ComposeCommandError> {
    match status {
        Ok(code) if code.success() => {
            debug!(description, "docker command succeeded");
            Ok(())
        }
        Ok(code) => {
            warn!(description, status = ?code, "docker command failed");
            Err(ComposeCommandError::Failed {
                command: description.to_owned(),
                status: code,
            })
        }
        Err(err) => {
            warn!(description, error = ?err, "failed to spawn docker command");
            Err(ComposeCommandError::Spawn {
                command: description.to_owned(),
                source: err,
            })
        }
    }
}

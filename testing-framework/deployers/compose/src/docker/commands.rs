use std::{
    io,
    path::Path,
    process::{self, ExitStatus},
    time::Duration,
};

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
    run_command_status(&mut command, timeout_duration, description).await
}

/// Runs `docker compose up -d` for the generated stack.
pub async fn compose_up(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
) -> Result<(), ComposeCommandError> {
    run_compose_action(
        compose_path,
        project_name,
        root,
        ["up", "-d"],
        adjust_timeout(COMPOSE_UP_TIMEOUT),
        "docker compose up",
    )
    .await
}

/// Runs `docker compose up --no-start` for the generated stack.
pub async fn compose_create(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
) -> Result<(), ComposeCommandError> {
    run_compose_action(
        compose_path,
        project_name,
        root,
        ["up", "--no-start"],
        adjust_timeout(COMPOSE_UP_TIMEOUT),
        "docker compose create",
    )
    .await
}

/// Runs `docker compose up -d --no-deps <service>` for a single service.
pub async fn compose_up_service(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
    service: &str,
) -> Result<(), ComposeCommandError> {
    run_compose_action_with_service(
        compose_path,
        project_name,
        root,
        ["up", "-d", "--no-deps"],
        service,
        adjust_timeout(COMPOSE_UP_TIMEOUT),
        "docker compose up service",
    )
    .await
}

/// Runs `docker compose down --volumes` for the generated stack.
pub async fn compose_down(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
) -> Result<(), ComposeCommandError> {
    run_compose_action(
        compose_path,
        project_name,
        root,
        ["down", "--volumes"],
        adjust_timeout(COMPOSE_UP_TIMEOUT),
        "docker compose down",
    )
    .await
}

/// Dump docker compose logs to stderr for debugging failures.
pub async fn dump_compose_logs(compose_file: &Path, project: &str, root: &Path) {
    let mut cmd = compose_command(compose_file, project, root);
    cmd.arg("logs").arg("--no-color");

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

fn compose_command(compose_path: &Path, project_name: &str, root: &Path) -> Command {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_path)
        .arg("-p")
        .arg(project_name)
        .current_dir(root);
    cmd
}

async fn run_compose_action<const N: usize>(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
    args: [&str; N],
    timeout_duration: Duration,
    description: &str,
) -> Result<(), ComposeCommandError> {
    let mut cmd = compose_command(compose_path, project_name, root);
    cmd.args(args);

    info!(
        compose_file = %compose_path.display(),
        project = project_name,
        root = %root.display(),
        description,
        "running docker compose command"
    );

    run_command_status(&mut cmd, timeout_duration, description).await
}

async fn run_compose_action_with_service<const N: usize>(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
    args: [&str; N],
    service: &str,
    timeout_duration: Duration,
    description: &str,
) -> Result<(), ComposeCommandError> {
    let mut cmd = compose_command(compose_path, project_name, root);
    cmd.args(args).arg(service);

    info!(
        compose_file = %compose_path.display(),
        project = project_name,
        root = %root.display(),
        service,
        description,
        "running docker compose command"
    );

    run_command_status(&mut cmd, timeout_duration, description).await
}

fn handle_compose_status(
    status: io::Result<ExitStatus>,
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

async fn run_command_status(
    command: &mut Command,
    timeout_duration: Duration,
    description: &str,
) -> Result<(), ComposeCommandError> {
    match timeout(timeout_duration, command.status()).await {
        Ok(status) => handle_compose_status(status, description),
        Err(_) => Err(ComposeCommandError::Timeout {
            command: description.to_owned(),
            timeout: timeout_duration,
        }),
    }
}

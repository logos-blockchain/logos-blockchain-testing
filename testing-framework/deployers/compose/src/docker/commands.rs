use std::{
    io::{self, Write as _},
    path::Path,
    process::{self, ExitStatus, Stdio},
    sync::{Arc, Mutex},
    time::Duration,
};

use testing_framework_core::adjust_timeout;
use tokio::{
    io::{AsyncRead, AsyncReadExt as _},
    process::Command,
    time::timeout,
};
use tracing::{debug, info, warn};

const COMPOSE_UP_TIMEOUT: Duration = Duration::from_secs(120);
const OUTPUT_DRAIN_TIMEOUT: Duration = Duration::from_millis(100);
type CapturedOutput = Arc<Mutex<Vec<u8>>>;

/// Errors running docker compose commands.
#[derive(Debug, thiserror::Error)]
pub enum ComposeCommandError {
    #[error("{command} exited with status {status}\nstdout:\n{stdout}\nstderr:\n{stderr}")]
    Failed {
        command: String,
        status: process::ExitStatus,
        stdout: String,
        stderr: String,
    },
    #[error("failed to spawn {command}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error(
        "{command} timed out after {timeout:?}\nstdout before timeout:\n{stdout}\nstderr before \
         timeout:\n{stderr}"
    )]
    Timeout {
        command: String,
        timeout: Duration,
        stdout: String,
        stderr: String,
    },
    #[error("compose cleanup failed; primary: {primary}; fallback: {fallback}")]
    CleanupFallback {
        primary: Box<ComposeCommandError>,
        fallback: Box<ComposeCommandError>,
    },
}

impl ComposeCommandError {
    pub(crate) fn is_port_conflict(&self) -> bool {
        let Self::Failed { stdout, stderr, .. } = self else {
            return false;
        };
        is_port_conflict_message(stdout) || is_port_conflict_message(stderr)
    }
}

fn is_port_conflict_message(stderr: &str) -> bool {
    stderr.contains("port is already allocated") || stderr.contains("address already in use")
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

/// Runs `docker compose start` for an existing service.
pub async fn compose_start_service(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
    service: &str,
) -> Result<(), ComposeCommandError> {
    run_compose_action_with_service(
        compose_path,
        project_name,
        root,
        ["start"],
        service,
        adjust_timeout(COMPOSE_UP_TIMEOUT),
        "docker compose start service",
    )
    .await
}

/// Runs `docker compose stop` for a single service.
pub async fn compose_stop_service(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
    service: &str,
) -> Result<(), ComposeCommandError> {
    run_compose_action_with_service(
        compose_path,
        project_name,
        root,
        ["stop"],
        service,
        adjust_timeout(COMPOSE_UP_TIMEOUT),
        "docker compose stop service",
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
        ["down", "--volumes", "--remove-orphans"],
        adjust_timeout(COMPOSE_UP_TIMEOUT),
        "docker compose down",
    )
    .await
}

/// Removes selected services, including running or partially created
/// containers.
pub async fn compose_rm_services(
    compose_path: &Path,
    project_name: &str,
    root: &Path,
    services: &[String],
) -> Result<(), ComposeCommandError> {
    let mut command = compose_command(compose_path, project_name, root);
    command.args(["rm", "--stop", "--force", "--volumes"]);
    command.args(services);
    run_command_status(
        &mut command,
        adjust_timeout(COMPOSE_UP_TIMEOUT),
        "docker compose rm services",
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

fn handle_compose_output(
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    description: &str,
) -> Result<(), ComposeCommandError> {
    if status.success() {
        debug!(description, "docker command succeeded");
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&stdout).trim().to_owned();
    let stderr = String::from_utf8_lossy(&stderr).trim().to_owned();
    warn!(description, ?status, %stdout, %stderr, "docker command failed");
    Err(ComposeCommandError::Failed {
        command: description.to_owned(),
        status,
        stdout,
        stderr,
    })
}

async fn run_command_status(
    command: &mut Command,
    timeout_duration: Duration,
    description: &str,
) -> Result<(), ComposeCommandError> {
    command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|source| ComposeCommandError::Spawn {
            command: description.to_owned(),
            source,
        })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| missing_pipe_error(description, "stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| missing_pipe_error(description, "stderr"))?;
    let stdout_output = CapturedOutput::default();
    let stderr_output = CapturedOutput::default();
    let stdout_task = tokio::spawn(capture_and_forward(
        stdout,
        false,
        Arc::clone(&stdout_output),
    ));
    let stderr_task = tokio::spawn(capture_and_forward(
        stderr,
        true,
        Arc::clone(&stderr_output),
    ));

    let status = timeout(timeout_duration, child.wait()).await;
    match status {
        Ok(status) => {
            collect_output(stdout_task, description).await?;
            collect_output(stderr_task, description).await?;
            let stdout = captured_bytes(&stdout_output);
            let stderr = captured_bytes(&stderr_output);
            match status {
                Ok(status) => handle_compose_output(status, stdout, stderr, description),
                Err(source) => Err(ComposeCommandError::Spawn {
                    command: description.to_owned(),
                    source,
                }),
            }
        }
        Err(_) => {
            let _ = child.start_kill();
            let _ = child.wait().await;
            drain_or_abort_output(stdout_task).await;
            drain_or_abort_output(stderr_task).await;
            Err(ComposeCommandError::Timeout {
                command: description.to_owned(),
                timeout: timeout_duration,
                stdout: captured_text(&stdout_output),
                stderr: captured_text(&stderr_output),
            })
        }
    }
}

fn missing_pipe_error(description: &str, stream: &str) -> ComposeCommandError {
    ComposeCommandError::Spawn {
        command: description.to_owned(),
        source: io::Error::other(format!("{stream} pipe was not available")),
    }
}

async fn capture_and_forward<R>(
    mut reader: R,
    stderr: bool,
    captured: CapturedOutput,
) -> io::Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0_u8; 4096];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            return Ok(());
        }
        let chunk = &buffer[..read];
        captured
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(chunk);
        if stderr {
            let mut target = io::stderr().lock();
            target.write_all(chunk)?;
            target.flush()?;
        } else {
            let mut target = io::stdout().lock();
            target.write_all(chunk)?;
            target.flush()?;
        }
    }
}

async fn collect_output(
    task: tokio::task::JoinHandle<io::Result<()>>,
    description: &str,
) -> Result<(), ComposeCommandError> {
    task.await
        .map_err(|source| ComposeCommandError::Spawn {
            command: description.to_owned(),
            source: io::Error::other(source),
        })?
        .map_err(|source| ComposeCommandError::Spawn {
            command: description.to_owned(),
            source,
        })
}

async fn drain_or_abort_output(mut task: tokio::task::JoinHandle<io::Result<()>>) {
    if timeout(OUTPUT_DRAIN_TIMEOUT, &mut task).await.is_err() {
        task.abort();
        let _ = task.await;
    }
}

fn captured_bytes(output: &CapturedOutput) -> Vec<u8> {
    output
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

fn captured_text(output: &CapturedOutput) -> String {
    String::from_utf8_lossy(&captured_bytes(output))
        .trim()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use tokio::process::Command;

    use super::{ComposeCommandError, is_port_conflict_message, run_command_status};

    #[test]
    fn identifies_only_known_docker_port_conflicts() {
        assert!(is_port_conflict_message(
            "Bind for 127.0.0.1:49152 failed: port is already allocated"
        ));
        assert!(is_port_conflict_message(
            "listen tcp4 127.0.0.1:49152: bind: address already in use"
        ));
        assert!(!is_port_conflict_message(
            "pull access denied for example, repository does not exist"
        ));
    }

    #[tokio::test]
    async fn timeout_retains_output_emitted_before_the_deadline() {
        let started = std::time::Instant::now();
        let mut command = Command::new("sh");
        command.args([
            "-c",
            "printf 'pulling layer\\n'; printf 'still waiting\\n' >&2; sleep 2",
        ]);

        let error = run_command_status(&mut command, Duration::from_millis(50), "diagnostic probe")
            .await
            .unwrap_err();

        let ComposeCommandError::Timeout { stdout, stderr, .. } = error else {
            panic!("expected timeout error");
        };
        assert!(stdout.contains("pulling layer"));
        assert!(stderr.contains("still waiting"));
        assert!(started.elapsed() < Duration::from_millis(500));
    }
}

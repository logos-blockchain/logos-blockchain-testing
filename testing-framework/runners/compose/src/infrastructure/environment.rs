use std::{
    env,
    net::{Ipv4Addr, TcpListener as StdTcpListener},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context as _, anyhow};
use testing_framework_core::{
    adjust_timeout, scenario::CleanupGuard, topology::generation::GeneratedTopology,
};
use tokio::{process::Command, time::timeout};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::{
    deployer::setup::DEFAULT_PROMETHEUS_PORT,
    descriptor::ComposeDescriptor,
    docker::{
        commands::{compose_up, dump_compose_logs, run_docker_command},
        ensure_compose_image,
        platform::resolve_image,
        workspace::ComposeWorkspace,
    },
    errors::{ComposeRunnerError, ConfigError, WorkspaceError},
    infrastructure::{
        cfgsync::{CfgsyncServerHandle, update_cfgsync_config},
        template::write_compose_file,
    },
    lifecycle::cleanup::RunnerCleanup,
};

const CFGSYNC_START_TIMEOUT: Duration = Duration::from_secs(180);
const COMPOSE_PORT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(30);
const STACK_BRINGUP_MAX_ATTEMPTS: usize = 3;

/// Paths and flags describing the prepared compose workspace.
pub struct WorkspaceState {
    pub workspace: ComposeWorkspace,
    pub root: PathBuf,
    pub cfgsync_path: PathBuf,
    pub use_kzg: bool,
}

/// Holds paths and handles for a running docker-compose stack.
pub struct StackEnvironment {
    compose_path: PathBuf,
    project_name: String,
    root: PathBuf,
    workspace: Option<ComposeWorkspace>,
    cfgsync_handle: Option<CfgsyncServerHandle>,
    prometheus_port: u16,
    grafana_port: u16,
}

impl StackEnvironment {
    /// Builds an environment from the prepared workspace and compose artifacts.
    pub fn from_workspace(
        state: WorkspaceState,
        compose_path: PathBuf,
        project_name: String,
        cfgsync_handle: Option<CfgsyncServerHandle>,
        prometheus_port: u16,
        grafana_port: u16,
    ) -> Self {
        let WorkspaceState {
            workspace, root, ..
        } = state;

        Self {
            compose_path,
            project_name,
            root,
            workspace: Some(workspace),
            cfgsync_handle,
            prometheus_port,
            grafana_port,
        }
    }

    pub fn compose_path(&self) -> &Path {
        &self.compose_path
    }

    /// Host port exposed by Prometheus.
    pub const fn prometheus_port(&self) -> u16 {
        self.prometheus_port
    }

    /// Host port exposed by Grafana.
    pub const fn grafana_port(&self) -> u16 {
        self.grafana_port
    }

    /// Docker compose project name.
    pub fn project_name(&self) -> &str {
        &self.project_name
    }

    /// Root directory that contains generated assets.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Convert into a cleanup guard while keeping the environment borrowed.
    pub fn take_cleanup(&mut self) -> RunnerCleanup {
        RunnerCleanup::new(
            self.compose_path.clone(),
            self.project_name.clone(),
            self.root.clone(),
            self.workspace
                .take()
                .expect("workspace must be available while cleaning up"),
            self.cfgsync_handle.take(),
        )
    }

    /// Convert into a cleanup guard, consuming the environment.
    pub fn into_cleanup(self) -> RunnerCleanup {
        RunnerCleanup::new(
            self.compose_path,
            self.project_name,
            self.root,
            self.workspace
                .expect("workspace must be available while cleaning up"),
            self.cfgsync_handle,
        )
    }

    /// Dump compose logs and trigger cleanup after a failure.
    pub async fn fail(&mut self, reason: &str) {
        error!(
            reason = reason,
            "compose stack failure; dumping docker logs"
        );
        dump_compose_logs(self.compose_path(), self.project_name(), self.root()).await;
        Box::new(self.take_cleanup()).cleanup();
    }
}

/// Represents a claimed port, optionally guarded by an open socket.
pub struct PortReservation {
    port: u16,
    _guard: Option<StdTcpListener>,
}

impl PortReservation {
    /// Holds a port and an optional socket guard to keep it reserved.
    pub const fn new(port: u16, guard: Option<StdTcpListener>) -> Self {
        Self {
            port,
            _guard: guard,
        }
    }

    /// The reserved port number.
    pub const fn port(&self) -> u16 {
        self.port
    }
}

/// Verifies the topology has at least one validator so compose can start.
pub fn ensure_supported_topology(
    descriptors: &GeneratedTopology,
) -> Result<(), ComposeRunnerError> {
    let validators = descriptors.validators().len();
    if validators == 0 {
        return Err(ComposeRunnerError::MissingValidator {
            validators,
            executors: descriptors.executors().len(),
        });
    }
    Ok(())
}

/// Create a temporary workspace with copied testnet assets and derived paths.
pub fn prepare_workspace_state() -> Result<WorkspaceState, WorkspaceError> {
    let workspace = ComposeWorkspace::create().map_err(WorkspaceError::new)?;
    let root = workspace.root_path().to_path_buf();
    let cfgsync_path = workspace.stack_dir().join("cfgsync.yaml");
    let use_kzg = workspace.root_path().join("kzgrs_test_params").exists();

    let state = WorkspaceState {
        workspace,
        root,
        cfgsync_path,
        use_kzg,
    };

    debug!(
        root = %state.root.display(),
        cfgsync = %state.cfgsync_path.display(),
        use_kzg = state.use_kzg,
        "prepared compose workspace state"
    );

    Ok(state)
}

/// Log wrapper for `prepare_workspace_state`.
pub fn prepare_workspace_logged() -> Result<WorkspaceState, ComposeRunnerError> {
    info!("preparing compose workspace");

    prepare_workspace_state().map_err(Into::into)
}

/// Render cfgsync config based on the topology and chosen port, logging
/// progress.
pub fn update_cfgsync_logged(
    workspace: &WorkspaceState,
    descriptors: &GeneratedTopology,
    cfgsync_port: u16,
) -> Result<(), ComposeRunnerError> {
    info!(cfgsync_port, "updating cfgsync configuration");

    configure_cfgsync(workspace, descriptors, cfgsync_port).map_err(Into::into)
}

/// Start the cfgsync server container using the generated config.
pub async fn start_cfgsync_stage(
    workspace: &WorkspaceState,
    cfgsync_port: u16,
) -> Result<CfgsyncServerHandle, ComposeRunnerError> {
    info!(cfgsync_port = cfgsync_port, "launching cfgsync server");
    let handle = launch_cfgsync(&workspace.cfgsync_path, cfgsync_port).await?;
    debug!(container = ?handle, "cfgsync server launched");
    Ok(handle)
}

/// Update cfgsync YAML on disk with topology-derived values.
pub fn configure_cfgsync(
    workspace: &WorkspaceState,
    descriptors: &GeneratedTopology,
    cfgsync_port: u16,
) -> Result<(), ConfigError> {
    update_cfgsync_config(
        &workspace.cfgsync_path,
        descriptors,
        workspace.use_kzg,
        cfgsync_port,
    )
    .map_err(|source| ConfigError::Cfgsync {
        path: workspace.cfgsync_path.clone(),
        source,
    })
}

/// Bind an ephemeral port for cfgsync, returning the chosen value.
pub fn allocate_cfgsync_port() -> Result<u16, ConfigError> {
    let listener =
        StdTcpListener::bind((Ipv4Addr::UNSPECIFIED, 0)).map_err(|source| ConfigError::Port {
            source: source.into(),
        })?;

    let port = listener
        .local_addr()
        .map_err(|source| ConfigError::Port {
            source: source.into(),
        })?
        .port();
    debug!(port, "allocated cfgsync port");
    Ok(port)
}

/// Launch cfgsync in a detached docker container on the provided port.
pub async fn launch_cfgsync(
    cfgsync_path: &Path,
    port: u16,
) -> Result<CfgsyncServerHandle, ConfigError> {
    let testnet_dir = cfgsync_path
        .parent()
        .ok_or_else(|| ConfigError::CfgsyncStart {
            port,
            source: anyhow!("cfgsync path {cfgsync_path:?} has no parent directory"),
        })?;
    let (image, _) = resolve_image();
    let container_name = format!("nomos-cfgsync-{}", Uuid::new_v4());
    debug!(
        container = %container_name,
        image,
        cfgsync = %cfgsync_path.display(),
        port,
        "starting cfgsync container"
    );

    let mut command = Command::new("docker");
    command
        .arg("run")
        .arg("-d")
        .arg("--name")
        .arg(&container_name)
        .arg("--entrypoint")
        .arg("cfgsync-server")
        .arg("-p")
        .arg(format!("{port}:{port}"))
        .arg("-v")
        .arg(format!(
            "{}:/etc/nomos:ro",
            testnet_dir
                .canonicalize()
                .unwrap_or_else(|_| testnet_dir.to_path_buf())
                .display()
        ))
        .arg(&image)
        .arg("/etc/nomos/cfgsync.yaml");

    run_docker_command(
        command,
        adjust_timeout(CFGSYNC_START_TIMEOUT),
        "docker run cfgsync server",
    )
    .await
    .map_err(|source| ConfigError::CfgsyncStart {
        port,
        source: anyhow!(source),
    })?;

    info!(container = %container_name, port, "cfgsync container started");

    Ok(CfgsyncServerHandle::Container {
        name: container_name,
        stopped: false,
    })
}

/// Render compose file and associated assets for the current topology.
pub fn write_compose_artifacts(
    workspace: &WorkspaceState,
    descriptors: &GeneratedTopology,
    cfgsync_port: u16,
    prometheus_port: u16,
    grafana_port: u16,
) -> Result<PathBuf, ConfigError> {
    debug!(
        cfgsync_port,
        prometheus_port,
        grafana_port,
        workspace_root = %workspace.root.display(),
        "building compose descriptor"
    );
    let descriptor = ComposeDescriptor::builder(descriptors)
        .with_kzg_mount(workspace.use_kzg)
        .with_cfgsync_port(cfgsync_port)
        .with_prometheus_port(prometheus_port)
        .with_grafana_port(grafana_port)
        .build()
        .map_err(|source| ConfigError::Descriptor { source })?;

    let compose_path = workspace.root.join("compose.generated.yml");
    write_compose_file(&descriptor, &compose_path)
        .map_err(|source| ConfigError::Template { source })?;
    debug!(compose_file = %compose_path.display(), "rendered compose file");
    Ok(compose_path)
}

/// Log and wrap `write_compose_artifacts` errors for the runner.
pub fn render_compose_logged(
    workspace: &WorkspaceState,
    descriptors: &GeneratedTopology,
    cfgsync_port: u16,
    prometheus_port: u16,
    grafana_port: u16,
) -> Result<PathBuf, ComposeRunnerError> {
    info!(
        cfgsync_port,
        prometheus_port, grafana_port, "rendering compose file with ports"
    );
    write_compose_artifacts(
        workspace,
        descriptors,
        cfgsync_port,
        prometheus_port,
        grafana_port,
    )
    .map_err(Into::into)
}

/// Bring up docker compose; shut down cfgsync if start-up fails.
pub async fn bring_up_stack(
    compose_path: &Path,
    project_name: &str,
    workspace_root: &Path,
    cfgsync_handle: &mut CfgsyncServerHandle,
) -> Result<(), ComposeRunnerError> {
    if let Err(err) = compose_up(compose_path, project_name, workspace_root).await {
        cfgsync_handle.shutdown();
        return Err(ComposeRunnerError::Compose(err));
    }
    debug!(project = %project_name, "docker compose up completed");
    Ok(())
}

/// Log compose bring-up with context.
pub async fn bring_up_stack_logged(
    compose_path: &Path,
    project_name: &str,
    workspace_root: &Path,
    cfgsync_handle: &mut CfgsyncServerHandle,
) -> Result<(), ComposeRunnerError> {
    info!(project = %project_name, "bringing up docker compose stack");
    bring_up_stack(compose_path, project_name, workspace_root, cfgsync_handle).await
}

/// Prepare workspace, cfgsync, compose artifacts, and launch the stack.
pub async fn prepare_environment(
    descriptors: &GeneratedTopology,
    mut prometheus_port: PortReservation,
    prometheus_port_locked: bool,
) -> Result<StackEnvironment, ComposeRunnerError> {
    let workspace = prepare_workspace_logged()?;
    let cfgsync_port = allocate_cfgsync_port()?;

    let grafana_env = env::var("COMPOSE_GRAFANA_PORT")
        .ok()
        .and_then(|raw| raw.parse::<u16>().ok());
    if let Some(port) = grafana_env {
        info!(port, "using grafana port from env");
    }

    update_cfgsync_logged(&workspace, descriptors, cfgsync_port)?;
    ensure_compose_image().await?;

    let attempts = if prometheus_port_locked {
        1
    } else {
        STACK_BRINGUP_MAX_ATTEMPTS
    };
    let mut last_err = None;

    for _ in 0..attempts {
        let prometheus_port_value = prometheus_port.port();
        let grafana_port_value = grafana_env.unwrap_or(0);
        let compose_path = render_compose_logged(
            &workspace,
            descriptors,
            cfgsync_port,
            prometheus_port_value,
            grafana_port_value,
        )?;

        let project_name = format!("nomos-compose-{}", Uuid::new_v4());
        let mut cfgsync_handle = start_cfgsync_stage(&workspace, cfgsync_port).await?;

        drop(prometheus_port);

        match bring_up_stack_logged(
            &compose_path,
            &project_name,
            &workspace.root,
            &mut cfgsync_handle,
        )
        .await
        {
            Ok(()) => {
                let grafana_port_resolved = resolve_service_port(
                    &compose_path,
                    &project_name,
                    &workspace.root,
                    "grafana",
                    3000,
                )
                .await
                .unwrap_or(grafana_port_value);

                info!(
                    project = %project_name,
                    compose_file = %compose_path.display(),
                    cfgsync_port,
                    prometheus_port = prometheus_port_value,
                    grafana_port = grafana_port_resolved,
                    "compose stack is up"
                );
                return Ok(StackEnvironment::from_workspace(
                    workspace,
                    compose_path,
                    project_name,
                    Some(cfgsync_handle),
                    prometheus_port_value,
                    grafana_port_resolved,
                ));
            }
            Err(err) => {
                // Attempt to capture container logs even when bring-up fails early.
                dump_compose_logs(&compose_path, &project_name, &workspace.root).await;
                cfgsync_handle.shutdown();
                last_err = Some(err);
                if prometheus_port_locked {
                    break;
                }
                warn!(
                    error = %last_err.as_ref().unwrap(),
                    "compose bring-up failed; retrying with a new prometheus port"
                );
                prometheus_port = allocate_prometheus_port()
                    .unwrap_or_else(|| PortReservation::new(DEFAULT_PROMETHEUS_PORT, None));
                debug!(
                    next_prometheus_port = prometheus_port.port(),
                    "retrying compose bring-up"
                );
            }
        }
    }

    Err(last_err.expect("prepare_environment should return or fail with error"))
}

fn allocate_prometheus_port() -> Option<PortReservation> {
    reserve_prometheus_port(DEFAULT_PROMETHEUS_PORT).or_else(|| reserve_prometheus_port(0))
}

fn reserve_prometheus_port(port: u16) -> Option<PortReservation> {
    let listener = StdTcpListener::bind((Ipv4Addr::LOCALHOST, port)).ok()?;
    let actual_port = listener.local_addr().ok()?.port();
    Some(PortReservation::new(actual_port, Some(listener)))
}

async fn resolve_service_port(
    compose_file: &Path,
    project_name: &str,
    root: &Path,
    service: &str,
    container_port: u16,
) -> Result<u16, ComposeRunnerError> {
    let mut cmd = Command::new("docker");
    cmd.arg("compose")
        .arg("-f")
        .arg(compose_file)
        .arg("-p")
        .arg(project_name)
        .arg("port")
        .arg(service)
        .arg(container_port.to_string())
        .current_dir(root);

    let output = timeout(adjust_timeout(COMPOSE_PORT_DISCOVERY_TIMEOUT), cmd.output())
        .await
        .map_err(|_| ComposeRunnerError::PortDiscovery {
            service: service.to_owned(),
            container_port,
            source: anyhow!("docker compose port timed out"),
        })?
        .with_context(|| format!("running docker compose port {service} {container_port}"))
        .map_err(|source| ComposeRunnerError::PortDiscovery {
            service: service.to_owned(),
            container_port,
            source,
        })?;

    if !output.status.success() {
        return Err(ComposeRunnerError::PortDiscovery {
            service: service.to_owned(),
            container_port,
            source: anyhow!("docker compose port exited with {}", output.status),
        });
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(port_str) = line.rsplit(':').next()
            && let Ok(port) = port_str.trim().parse::<u16>()
        {
            return Ok(port);
        }
    }

    Err(ComposeRunnerError::PortDiscovery {
        service: service.to_owned(),
        container_port,
        source: anyhow!("unable to parse docker compose port output: {stdout}"),
    })
}

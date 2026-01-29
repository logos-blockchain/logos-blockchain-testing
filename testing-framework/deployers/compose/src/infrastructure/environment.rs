use std::{
    net::{Ipv4Addr, TcpListener as StdTcpListener},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::anyhow;
use reqwest::Url;
use testing_framework_core::{
    adjust_timeout, scenario::CleanupGuard, topology::generation::GeneratedTopology,
};
use tokio::{
    net::TcpStream,
    process::Command,
    time::{Instant, sleep},
};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::{
    descriptor::ComposeDescriptor,
    docker::{
        commands::{compose_create, compose_up, dump_compose_logs, run_docker_command},
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
const CFGSYNC_READY_TIMEOUT: Duration = Duration::from_secs(60);
const CFGSYNC_READY_POLL: Duration = Duration::from_secs(2);

/// Paths and flags describing the prepared compose workspace.
pub struct WorkspaceState {
    pub workspace: ComposeWorkspace,
    pub root: PathBuf,
    pub cfgsync_path: PathBuf,
}

/// Holds paths and handles for a running docker-compose stack.
pub struct StackEnvironment {
    compose_path: PathBuf,
    project_name: String,
    root: PathBuf,
    workspace: Option<ComposeWorkspace>,
    cfgsync_handle: Option<CfgsyncServerHandle>,
}

impl StackEnvironment {
    /// Builds an environment from the prepared workspace and compose artifacts.
    pub fn from_workspace(
        state: WorkspaceState,
        compose_path: PathBuf,
        project_name: String,
        cfgsync_handle: Option<CfgsyncServerHandle>,
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
        }
    }

    pub fn compose_path(&self) -> &Path {
        &self.compose_path
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
    pub fn take_cleanup(&mut self) -> Result<RunnerCleanup, ComposeRunnerError> {
        let workspace = self
            .workspace
            .take()
            .ok_or(ComposeRunnerError::InternalInvariant {
                message: "workspace must be available while cleaning up",
            })?;

        Ok(RunnerCleanup::new(
            self.compose_path.clone(),
            self.project_name.clone(),
            self.root.clone(),
            workspace,
            self.cfgsync_handle.take(),
        ))
    }

    /// Convert into a cleanup guard, consuming the environment.
    pub fn into_cleanup(self) -> Result<RunnerCleanup, ComposeRunnerError> {
        let workspace = self
            .workspace
            .ok_or(ComposeRunnerError::InternalInvariant {
                message: "workspace must be available while cleaning up",
            })?;

        Ok(RunnerCleanup::new(
            self.compose_path,
            self.project_name,
            self.root,
            workspace,
            self.cfgsync_handle,
        ))
    }

    /// Dump compose logs and trigger cleanup after a failure.
    pub async fn fail(&mut self, reason: &str) {
        error!(
            reason = reason,
            "compose stack failure; dumping docker logs"
        );
        dump_compose_logs(self.compose_path(), self.project_name(), self.root()).await;
        match self.take_cleanup() {
            Ok(cleanup) => Box::new(cleanup).cleanup(),
            Err(err) => error!(error = %err, "failed to acquire cleanup guard"),
        }
    }
}

/// Verifies the topology has at least one node so compose can start.
pub fn ensure_supported_topology(
    descriptors: &GeneratedTopology,
) -> Result<(), ComposeRunnerError> {
    let nodes = descriptors.nodes().len();

    if nodes == 0 {
        return Err(ComposeRunnerError::MissingNode { nodes });
    }

    Ok(())
}

/// Create a temporary workspace with copied testnet assets and derived paths.
pub fn prepare_workspace_state() -> Result<WorkspaceState, WorkspaceError> {
    let workspace = ComposeWorkspace::create().map_err(WorkspaceError::new)?;
    let root = workspace.root_path().to_path_buf();
    let cfgsync_path = workspace.stack_dir().join("cfgsync.yaml");
    let state = WorkspaceState {
        workspace,
        root,
        cfgsync_path,
    };

    debug!(
        root = %state.root.display(),
        cfgsync = %state.cfgsync_path.display(),
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
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<(), ComposeRunnerError> {
    info!(cfgsync_port, "updating cfgsync configuration");

    configure_cfgsync(
        workspace,
        descriptors,
        cfgsync_port,
        metrics_otlp_ingest_url,
    )
    .map_err(Into::into)
}

/// Start the cfgsync server container using the generated config.
pub async fn start_cfgsync_stage(
    workspace: &WorkspaceState,
    cfgsync_port: u16,
    project_name: &str,
) -> Result<CfgsyncServerHandle, ComposeRunnerError> {
    info!(cfgsync_port = cfgsync_port, "launching cfgsync server");

    let network = compose_network_name(project_name);
    let handle = launch_cfgsync(&workspace.cfgsync_path, cfgsync_port, &network).await?;

    wait_for_cfgsync_ready(cfgsync_port, Some(&handle)).await?;

    debug!(container = ?handle, "cfgsync server launched");

    Ok(handle)
}

/// Update cfgsync YAML on disk with topology-derived values.
pub fn configure_cfgsync(
    workspace: &WorkspaceState,
    descriptors: &GeneratedTopology,
    cfgsync_port: u16,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<(), ConfigError> {
    update_cfgsync_config(
        &workspace.cfgsync_path,
        descriptors,
        cfgsync_port,
        metrics_otlp_ingest_url,
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
    network: &str,
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
        .arg("--network")
        .arg(network)
        .arg("--network-alias")
        .arg("cfgsync")
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
        ));
    let circuits_dir = std::env::var("LOGOS_BLOCKCHAIN_CIRCUITS_DOCKER")
        .ok()
        .or_else(|| std::env::var("LOGOS_BLOCKCHAIN_CIRCUITS").ok());
    if let Some(circuits_dir) = circuits_dir {
        let host_path = PathBuf::from(&circuits_dir);
        if host_path.exists() {
            command
                .arg("-e")
                .arg(format!("LOGOS_BLOCKCHAIN_CIRCUITS={circuits_dir}"))
                .arg("-v")
                .arg(format!(
                    "{}:{circuits_dir}:ro",
                    host_path.canonicalize().unwrap_or(host_path).display()
                ));
        }
    }

    if let Ok(pol_mode) = std::env::var("POL_PROOF_DEV_MODE") {
        command
            .arg("-e")
            .arg(format!("POL_PROOF_DEV_MODE={pol_mode}"));
    }

    command.arg(&image).arg("/etc/nomos/cfgsync.yaml");

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
) -> Result<PathBuf, ConfigError> {
    debug!(
        cfgsync_port,
        workspace_root = %workspace.root.display(),
        "building compose descriptor"
    );
    let descriptor = ComposeDescriptor::builder(descriptors)
        .with_cfgsync_port(cfgsync_port)
        .build();

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
) -> Result<PathBuf, ComposeRunnerError> {
    info!(cfgsync_port, "rendering compose file");
    write_compose_artifacts(workspace, descriptors, cfgsync_port).map_err(Into::into)
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
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<StackEnvironment, ComposeRunnerError> {
    let workspace = prepare_workspace_logged()?;
    let cfgsync_port = allocate_cfgsync_port()?;
    update_cfgsync_logged(
        &workspace,
        descriptors,
        cfgsync_port,
        metrics_otlp_ingest_url,
    )?;
    ensure_compose_image().await?;
    let compose_path = render_compose_logged(&workspace, descriptors, cfgsync_port)?;

    let project_name = format!("nomos-compose-{}", Uuid::new_v4());
    compose_create(&compose_path, &project_name, &workspace.root).await?;
    let mut cfgsync_handle = start_cfgsync_stage(&workspace, cfgsync_port, &project_name).await?;

    if let Err(err) = bring_up_stack_logged(
        &compose_path,
        &project_name,
        &workspace.root,
        &mut cfgsync_handle,
    )
    .await
    {
        dump_compose_logs(&compose_path, &project_name, &workspace.root).await;
        cfgsync_handle.shutdown();
        return Err(err);
    }

    info!(
        project = %project_name,
        compose_file = %compose_path.display(),
        cfgsync_port,
        "compose stack is up"
    );

    Ok(StackEnvironment::from_workspace(
        workspace,
        compose_path,
        project_name,
        Some(cfgsync_handle),
    ))
}

/// Prepare workspace, cfgsync, and compose artifacts without starting services.
pub async fn prepare_environment_manual(
    descriptors: &GeneratedTopology,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<StackEnvironment, ComposeRunnerError> {
    let workspace = prepare_workspace_logged()?;
    let cfgsync_port = allocate_cfgsync_port()?;

    update_cfgsync_logged(
        &workspace,
        descriptors,
        cfgsync_port,
        metrics_otlp_ingest_url,
    )?;
    ensure_compose_image().await?;
    let compose_path = render_compose_logged(&workspace, descriptors, cfgsync_port)?;

    let project_name = format!("nomos-compose-{}", Uuid::new_v4());
    compose_create(&compose_path, &project_name, &workspace.root).await?;
    let cfgsync_handle = start_cfgsync_stage(&workspace, cfgsync_port, &project_name).await?;

    info!(
        project = %project_name,
        compose_file = %compose_path.display(),
        cfgsync_port,
        "compose manual environment prepared"
    );

    Ok(StackEnvironment::from_workspace(
        workspace,
        compose_path,
        project_name,
        Some(cfgsync_handle),
    ))
}

async fn wait_for_cfgsync_ready(
    port: u16,
    handle: Option<&CfgsyncServerHandle>,
) -> Result<(), ComposeRunnerError> {
    let deadline = Instant::now() + CFGSYNC_READY_TIMEOUT;
    let addr = format!("127.0.0.1:{port}");

    loop {
        match TcpStream::connect(&addr).await {
            Ok(_) => {
                info!(port, "cfgsync server is reachable");
                return Ok(());
            }
            Err(err) => {
                if Instant::now() >= deadline {
                    dump_cfgsync_logs(handle).await;
                    return Err(ComposeRunnerError::Config(ConfigError::CfgsyncStart {
                        port,
                        source: anyhow!("cfgsync not reachable: {err}"),
                    }));
                }
            }
        }
        sleep(CFGSYNC_READY_POLL).await;
    }
}

async fn dump_cfgsync_logs(handle: Option<&CfgsyncServerHandle>) {
    let Some(name) = handle.and_then(cfgsync_container_name) else {
        return;
    };

    let mut cmd = Command::new("docker");
    cmd.arg("logs").arg(name);

    match cmd.output().await {
        Ok(output) => {
            if !output.stdout.is_empty() {
                warn!(
                    logs = %String::from_utf8_lossy(&output.stdout),
                    container = name,
                    "cfgsync stdout"
                );
            }

            if !output.stderr.is_empty() {
                warn!(
                    logs = %String::from_utf8_lossy(&output.stderr),
                    container = name,
                    "cfgsync stderr"
                );
            }
        }
        Err(err) => warn!(error = ?err, container = name, "failed to collect cfgsync logs"),
    }
}

fn cfgsync_container_name(handle: &CfgsyncServerHandle) -> Option<&str> {
    match handle {
        CfgsyncServerHandle::Container { name, .. } => Some(name.as_str()),
    }
}

fn compose_network_name(project_name: &str) -> String {
    format!("{project_name}_default")
}

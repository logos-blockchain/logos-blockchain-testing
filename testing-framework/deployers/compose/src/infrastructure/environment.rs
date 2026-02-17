use std::{
    net::{Ipv4Addr, TcpListener as StdTcpListener},
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::anyhow;
use reqwest::Url;
use testing_framework_core::{scenario::CleanupGuard, topology::DeploymentDescriptor};
use tokio::{net::TcpStream, process::Command};
use tokio_retry::{Retry, strategy::FixedInterval};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::{
    docker::{
        commands::{compose_create, compose_up, dump_compose_logs},
        ensure_image_present,
        workspace::ComposeWorkspace,
    },
    env::{ComposeDeployEnv, ConfigServerHandle},
    errors::{ComposeRunnerError, ConfigError, WorkspaceError},
    infrastructure::template::write_compose_file,
    lifecycle::cleanup::RunnerCleanup,
};

const CFGSYNC_READY_TIMEOUT: Duration = Duration::from_secs(60);
const CFGSYNC_READY_POLL: Duration = Duration::from_secs(2);
const CFGSYNC_REACHABILITY_ADDR: &str = "127.0.0.1";

/// Prepared workspace paths.
pub struct WorkspaceState {
    pub workspace: ComposeWorkspace,
    pub root: PathBuf,
    pub cfgsync_path: PathBuf,
}

struct PreparedEnvironment {
    workspace: WorkspaceState,
    cfgsync_port: u16,
    compose_path: PathBuf,
    project_name: String,
}

/// Runtime handles for a compose stack.
pub struct StackEnvironment {
    compose_path: PathBuf,
    project_name: String,
    root: PathBuf,
    workspace: Option<ComposeWorkspace>,
    cfgsync_handle: Option<Box<dyn ConfigServerHandle>>,
}

impl StackEnvironment {
    /// Build from prepared workspace artifacts.
    pub fn from_workspace(
        state: WorkspaceState,
        compose_path: PathBuf,
        project_name: String,
        cfgsync_handle: Option<Box<dyn ConfigServerHandle>>,
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

    /// Compose project name.
    pub fn project_name(&self) -> &str {
        &self.project_name
    }

    /// Root directory with generated assets.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Build a cleanup guard without consuming the environment.
    pub fn take_cleanup(&mut self) -> Result<RunnerCleanup, ComposeRunnerError> {
        let workspace = self.workspace.take().ok_or_else(missing_workspace_error)?;

        Ok(build_runner_cleanup(
            self.compose_path.clone(),
            self.project_name.clone(),
            self.root.clone(),
            workspace,
            self.cfgsync_handle.take(),
        ))
    }

    /// Build a cleanup guard and consume the environment.
    pub fn into_cleanup(self) -> Result<RunnerCleanup, ComposeRunnerError> {
        let workspace = self.workspace.ok_or_else(missing_workspace_error)?;

        Ok(build_runner_cleanup(
            self.compose_path,
            self.project_name,
            self.root,
            workspace,
            self.cfgsync_handle,
        ))
    }

    /// Dump logs and trigger cleanup after failure.
    pub async fn fail(&mut self, reason: &str) {
        error!(
            reason = reason,
            "compose stack failure; dumping docker logs"
        );
        dump_compose_logs(self.compose_path(), self.project_name(), self.root()).await;
        self.cleanup_after_failure();
    }

    fn cleanup_after_failure(&mut self) {
        let cleanup = match self.take_cleanup() {
            Ok(cleanup) => cleanup,
            Err(err) => {
                error!(error = %err, "failed to acquire cleanup guard");
                return;
            }
        };

        Box::new(cleanup).cleanup();
    }
}

fn missing_workspace_error() -> ComposeRunnerError {
    ComposeRunnerError::InternalInvariant {
        message: "workspace must be available while cleaning up",
    }
}

/// Ensure topology has at least one node.
pub fn ensure_supported_topology<E: ComposeDeployEnv>(
    descriptors: &E::Deployment,
) -> Result<(), ComposeRunnerError> {
    let nodes = descriptors.node_count();

    if nodes == 0 {
        return Err(ComposeRunnerError::MissingNode { nodes });
    }

    Ok(())
}

/// Create a temporary workspace and derive key paths.
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

/// Prepare the workspace and emit setup logs.
pub fn prepare_workspace_logged() -> Result<WorkspaceState, ComposeRunnerError> {
    info!("preparing compose workspace");

    let workspace = prepare_workspace_state()?;
    Ok(workspace)
}

/// Update cfgsync config and emit setup logs.
pub fn update_cfgsync_logged<E: ComposeDeployEnv>(
    workspace: &WorkspaceState,
    descriptors: &E::Deployment,
    cfgsync_port: u16,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<(), ComposeRunnerError> {
    info!(cfgsync_port, "updating cfgsync configuration");

    configure_cfgsync::<E>(
        workspace,
        descriptors,
        cfgsync_port,
        metrics_otlp_ingest_url,
    )?;

    Ok(())
}

/// Start cfgsync using generated config.
pub async fn start_cfgsync_stage<E: ComposeDeployEnv>(
    workspace: &WorkspaceState,
    cfgsync_port: u16,
    project_name: &str,
) -> Result<Box<dyn ConfigServerHandle>, ComposeRunnerError> {
    info!(cfgsync_port = cfgsync_port, "launching cfgsync server");

    let network = compose_network_name(project_name);
    let handle = E::start_cfgsync(&workspace.cfgsync_path, cfgsync_port, &network)
        .await
        .map_err(|source| {
            ComposeRunnerError::Config(ConfigError::CfgsyncStart {
                port: cfgsync_port,
                source,
            })
        })?;

    wait_for_cfgsync_ready(cfgsync_port, Some(&handle)).await?;
    log_cfgsync_started(&handle);

    Ok(Box::new(handle))
}

/// Write cfgsync YAML from topology data.
pub fn configure_cfgsync<E: ComposeDeployEnv>(
    workspace: &WorkspaceState,
    descriptors: &E::Deployment,
    cfgsync_port: u16,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<(), ConfigError> {
    E::update_cfgsync_config(
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

/// Allocate an ephemeral cfgsync port.
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

/// Render compose file for the current topology.
pub fn write_compose_artifacts<E: ComposeDeployEnv>(
    workspace: &WorkspaceState,
    descriptors: &E::Deployment,
    cfgsync_port: u16,
) -> Result<PathBuf, ConfigError> {
    debug!(
        cfgsync_port,
        workspace_root = %workspace.root.display(),
        "building compose descriptor"
    );
    let descriptor = E::compose_descriptor(descriptors, cfgsync_port);

    let compose_path = workspace.root.join("compose.generated.yml");
    write_compose_file(&descriptor, &compose_path)
        .map_err(|source| ConfigError::Template { source })?;

    debug!(compose_file = %compose_path.display(), "rendered compose file");
    Ok(compose_path)
}

/// Logged wrapper for `write_compose_artifacts`.
pub fn render_compose_logged<E: ComposeDeployEnv>(
    workspace: &WorkspaceState,
    descriptors: &E::Deployment,
    cfgsync_port: u16,
) -> Result<PathBuf, ComposeRunnerError> {
    info!(cfgsync_port, "rendering compose file");

    let compose_path = write_compose_artifacts::<E>(workspace, descriptors, cfgsync_port)?;
    Ok(compose_path)
}

/// Run `docker compose up`; stop cfgsync on failure.
pub async fn bring_up_stack(
    compose_path: &Path,
    project_name: &str,
    workspace_root: &Path,
    cfgsync_handle: &mut dyn ConfigServerHandle,
) -> Result<(), ComposeRunnerError> {
    if let Err(err) = compose_up(compose_path, project_name, workspace_root).await {
        cfgsync_handle.shutdown();
        return Err(ComposeRunnerError::Compose(err));
    }
    debug!(project = %project_name, "docker compose up completed");
    Ok(())
}

/// Logged compose bring-up.
pub async fn bring_up_stack_logged(
    compose_path: &Path,
    project_name: &str,
    workspace_root: &Path,
    cfgsync_handle: &mut dyn ConfigServerHandle,
) -> Result<(), ComposeRunnerError> {
    info!(project = %project_name, "bringing up docker compose stack");
    bring_up_stack(compose_path, project_name, workspace_root, cfgsync_handle).await
}

/// Prepare workspace, cfgsync, compose artifacts, and launch the stack.
pub async fn prepare_environment<E: ComposeDeployEnv>(
    descriptors: &E::Deployment,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<StackEnvironment, ComposeRunnerError> {
    let prepared = prepare_stack_artifacts::<E>(descriptors, metrics_otlp_ingest_url).await?;
    let mut cfgsync_handle = start_cfgsync_for_prepared::<E>(&prepared).await?;
    start_compose_stack(&prepared, cfgsync_handle.as_mut()).await?;
    log_compose_environment_ready(&prepared, "compose stack is up");

    Ok(stack_environment_from_prepared(
        prepared,
        Some(cfgsync_handle),
    ))
}

/// Prepare workspace, cfgsync, and compose artifacts without starting services.
pub async fn prepare_environment_manual<E: ComposeDeployEnv>(
    descriptors: &E::Deployment,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<StackEnvironment, ComposeRunnerError> {
    let prepared = prepare_stack_artifacts::<E>(descriptors, metrics_otlp_ingest_url).await?;
    let cfgsync_handle = start_cfgsync_for_prepared::<E>(&prepared).await?;

    log_compose_environment_ready(&prepared, "compose manual environment prepared");

    Ok(stack_environment_from_prepared(
        prepared,
        Some(cfgsync_handle),
    ))
}

async fn prepare_stack_artifacts<E: ComposeDeployEnv>(
    descriptors: &E::Deployment,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<PreparedEnvironment, ComposeRunnerError> {
    let workspace = prepare_workspace_logged()?;
    let cfgsync_port = allocate_cfgsync_port()?;
    update_cfgsync_logged::<E>(
        &workspace,
        descriptors,
        cfgsync_port,
        metrics_otlp_ingest_url,
    )?;
    ensure_compose_image_present::<E>().await?;
    let compose_path = render_compose_logged::<E>(&workspace, descriptors, cfgsync_port)?;
    let project_name = create_project_name();
    compose_create(&compose_path, &project_name, &workspace.root).await?;

    Ok(PreparedEnvironment {
        workspace,
        cfgsync_port,
        compose_path,
        project_name,
    })
}

async fn ensure_compose_image_present<E: ComposeDeployEnv>() -> Result<(), ComposeRunnerError> {
    let (image, platform) = E::compose_image();
    ensure_image_present(&image, platform.as_deref()).await
}

fn create_project_name() -> String {
    format!("compose-stack-{}", Uuid::new_v4())
}

async fn start_cfgsync_for_prepared<E: ComposeDeployEnv>(
    prepared: &PreparedEnvironment,
) -> Result<Box<dyn ConfigServerHandle>, ComposeRunnerError> {
    start_cfgsync_stage::<E>(
        &prepared.workspace,
        prepared.cfgsync_port,
        &prepared.project_name,
    )
    .await
}

async fn handle_compose_start_failure(
    prepared: &PreparedEnvironment,
    cfgsync_handle: &mut dyn ConfigServerHandle,
) {
    dump_compose_logs(
        &prepared.compose_path,
        &prepared.project_name,
        &prepared.workspace.root,
    )
    .await;
    cfgsync_handle.shutdown();
}

fn stack_environment_from_prepared(
    prepared: PreparedEnvironment,
    cfgsync_handle: Option<Box<dyn ConfigServerHandle>>,
) -> StackEnvironment {
    StackEnvironment::from_workspace(
        prepared.workspace,
        prepared.compose_path,
        prepared.project_name,
        cfgsync_handle,
    )
}

async fn start_compose_stack(
    prepared: &PreparedEnvironment,
    cfgsync_handle: &mut dyn ConfigServerHandle,
) -> Result<(), ComposeRunnerError> {
    if let Err(error) = bring_up_stack_logged(
        &prepared.compose_path,
        &prepared.project_name,
        &prepared.workspace.root,
        cfgsync_handle,
    )
    .await
    {
        handle_compose_start_failure(prepared, cfgsync_handle).await;
        return Err(error);
    }

    Ok(())
}

fn log_compose_environment_ready(prepared: &PreparedEnvironment, message: &str) {
    info!(
        project = %prepared.project_name,
        compose_file = %prepared.compose_path.display(),
        cfgsync_port = prepared.cfgsync_port,
        status = message,
        "compose environment prepared"
    );
}

async fn wait_for_cfgsync_ready(
    port: u16,
    handle: Option<&dyn ConfigServerHandle>,
) -> Result<(), ComposeRunnerError> {
    let addr = format!("{CFGSYNC_REACHABILITY_ADDR}:{port}");
    let strategy = cfgsync_retry_strategy();

    let result = Retry::spawn(strategy, || async { TcpStream::connect(&addr).await }).await;

    if let Err(error) = result {
        dump_cfgsync_logs(handle).await;
        return Err(cfgsync_reachability_error(port, &error.to_string()));
    }

    info!(port, "cfgsync server is reachable");
    Ok(())
}

fn cfgsync_reachability_error(port: u16, details: &str) -> ComposeRunnerError {
    ComposeRunnerError::Config(ConfigError::CfgsyncStart {
        port,
        source: anyhow!("cfgsync not reachable: {details}").into(),
    })
}

fn cfgsync_retry_strategy() -> impl Iterator<Item = Duration> {
    let timeout_ms = CFGSYNC_READY_TIMEOUT.as_millis();
    let poll_ms = CFGSYNC_READY_POLL.as_millis();
    let max_attempts = timeout_ms.div_ceil(poll_ms).max(1) as usize;

    FixedInterval::from_millis(CFGSYNC_READY_POLL.as_millis() as u64).take(max_attempts)
}

async fn dump_cfgsync_logs(handle: Option<&dyn ConfigServerHandle>) {
    let Some(name) = handle.and_then(|handle| handle.container_name()) else {
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

fn compose_network_name(project_name: &str) -> String {
    format!("{project_name}_default")
}

fn log_cfgsync_started(handle: &impl ConfigServerHandle) {
    if let Some(name) = handle.container_name() {
        debug!(container = name, "cfgsync server launched");
        return;
    }

    debug!("cfgsync server launched");
}

fn build_runner_cleanup(
    compose_path: PathBuf,
    project_name: String,
    root: PathBuf,
    workspace: ComposeWorkspace,
    cfgsync_handle: Option<Box<dyn ConfigServerHandle>>,
) -> RunnerCleanup {
    RunnerCleanup::new(compose_path, project_name, root, workspace, cfgsync_handle)
}

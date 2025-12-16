use std::{env, path::PathBuf, thread};

use testing_framework_core::scenario::CleanupGuard;
use tracing::{debug, info, warn};

use crate::{
    docker::{
        commands::{ComposeCommandError, compose_down},
        workspace::ComposeWorkspace,
    },
    infrastructure::cfgsync::CfgsyncServerHandle,
};

/// Cleans up a compose deployment and associated cfgsync container.
pub struct RunnerCleanup {
    pub compose_file: PathBuf,
    pub project_name: String,
    pub root: PathBuf,
    workspace: Option<ComposeWorkspace>,
    cfgsync: Option<CfgsyncServerHandle>,
}

impl RunnerCleanup {
    /// Construct a cleanup guard for the given compose deployment.
    pub fn new(
        compose_file: PathBuf,
        project_name: String,
        root: PathBuf,
        workspace: ComposeWorkspace,
        cfgsync: Option<CfgsyncServerHandle>,
    ) -> Self {
        debug_assert!(
            !compose_file.as_os_str().is_empty() && !project_name.is_empty(),
            "compose cleanup should receive valid identifiers"
        );
        Self {
            compose_file,
            project_name,
            root,
            workspace: Some(workspace),
            cfgsync,
        }
    }

    fn teardown_compose(&self) {
        if let Err(err) =
            run_compose_down_blocking(&self.compose_file, &self.project_name, &self.root)
        {
            warn!(error = ?err, "docker compose down failed");
        }
    }
}

fn run_compose_down_blocking(
    compose_file: &PathBuf,
    project_name: &str,
    root: &PathBuf,
) -> Result<(), ComposeCommandError> {
    let compose_file = compose_file.clone();
    let project_name = project_name.to_owned();
    let root = root.clone();

    let handle = thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| ComposeCommandError::Spawn {
                command: "docker compose down".into(),
                source: std::io::Error::new(std::io::ErrorKind::Other, err),
            })?
            .block_on(compose_down(&compose_file, &project_name, &root))
    });

    handle.join().map_err(|_| ComposeCommandError::Spawn {
        command: "docker compose down".into(),
        source: std::io::Error::new(
            std::io::ErrorKind::Other,
            "join failure running compose down",
        ),
    })?
}
impl CleanupGuard for RunnerCleanup {
    fn cleanup(mut self: Box<Self>) {
        debug!(
            compose_file = %self.compose_file.display(),
            project = %self.project_name,
            root = %self.root.display(),
            preserve = self.should_preserve(),
            "compose cleanup started"
        );
        if self.should_preserve() {
            self.persist_workspace();
            return;
        }

        self.teardown_compose();

        self.shutdown_cfgsync();
    }
}

impl RunnerCleanup {
    fn should_preserve(&self) -> bool {
        env::var("COMPOSE_RUNNER_PRESERVE").is_ok() || env::var("TESTNET_RUNNER_PRESERVE").is_ok()
    }

    fn persist_workspace(&mut self) {
        if let Some(workspace) = self.workspace.take() {
            let keep = workspace.into_inner().keep();
            info!(path = %keep.display(), "preserving docker state");
        }

        info!("compose preserve flag set; skipping docker compose down");
    }

    fn shutdown_cfgsync(&mut self) {
        if let Some(mut handle) = self.cfgsync.take() {
            handle.shutdown();
        }
    }
}

use std::{
    env, io,
    path::{Path, PathBuf},
    thread,
};

use testing_framework_core::scenario::CleanupGuard;
use tracing::{debug, info, warn};

use crate::{
    docker::{
        commands::{ComposeCommandError, compose_down},
        workspace::ComposeWorkspace,
    },
    env::ConfigServerHandle,
};

/// Cleans up a compose deployment and associated cfgsync container.
pub struct RunnerCleanup {
    pub compose_file: PathBuf,
    pub project_name: String,
    pub root: PathBuf,
    workspace: Option<ComposeWorkspace>,
    cfgsync: Option<Box<dyn ConfigServerHandle>>,
}

impl RunnerCleanup {
    /// Construct a cleanup guard for the given compose deployment.
    pub fn new(
        compose_file: PathBuf,
        project_name: String,
        root: PathBuf,
        workspace: ComposeWorkspace,
        cfgsync: Option<Box<dyn ConfigServerHandle>>,
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
    compose_file: &Path,
    project_name: &str,
    root: &Path,
) -> Result<(), ComposeCommandError> {
    let compose_file = compose_file.to_path_buf();
    let project_name = project_name.to_owned();
    let root = root.to_path_buf();

    let handle = thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| ComposeCommandError::Spawn {
                command: "docker compose down".into(),
                source: io::Error::other(err),
            })?
            .block_on(compose_down(&compose_file, &project_name, &root))
    });

    handle.join().map_err(|_| ComposeCommandError::Spawn {
        command: "docker compose down".into(),
        source: io::Error::other("join failure running compose down"),
    })?
}
impl CleanupGuard for RunnerCleanup {
    fn cleanup(mut self: Box<Self>) {
        let preserve = self.should_preserve();

        debug!(
            compose_file = %self.compose_file.display(),
            project = %self.project_name,
            root = %self.root.display(),
            preserve,
            "compose cleanup started"
        );

        if preserve {
            self.persist_workspace();
            return;
        }

        self.shutdown_cfgsync();

        self.teardown_compose();
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

        if let Some(mut cfgsync) = self.cfgsync.take() {
            cfgsync.mark_preserved();
            self.cfgsync = Some(cfgsync);
        }

        info!("compose preserve flag set; skipping docker compose down");
    }

    fn shutdown_cfgsync(&mut self) {
        if let Some(mut handle) = self.cfgsync.take() {
            handle.shutdown();
        }
    }
}

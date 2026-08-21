use std::{env, io, thread};

use testing_framework_core::scenario::CleanupGuard;
use tracing::{debug, info, warn};

use crate::{
    docker::{commands::ComposeCommandError, workspace::ComposeWorkspace},
    env::ConfigServerHandle,
    infrastructure::project::ComposeProject,
};

/// Cleans up a compose deployment and associated cfgsync container.
pub struct RunnerCleanup {
    project: ComposeProject,
    workspace: Option<ComposeWorkspace>,
    cfgsync: Option<Box<dyn ConfigServerHandle>>,
}

impl RunnerCleanup {
    /// Construct a cleanup guard for the given compose deployment.
    pub(crate) fn new(
        project: ComposeProject,
        workspace: ComposeWorkspace,
        cfgsync: Option<Box<dyn ConfigServerHandle>>,
    ) -> Self {
        debug_assert!(
            !project.compose_file().as_os_str().is_empty() && !project.name().is_empty(),
            "compose cleanup should receive valid identifiers"
        );
        Self {
            project,
            workspace: Some(workspace),
            cfgsync,
        }
    }

    fn teardown_compose(&self) {
        if let Err(err) = run_compose_down_blocking(self.project.clone()) {
            warn!(error = ?err, "docker compose down failed");
        }
    }
}

fn run_compose_down_blocking(project: ComposeProject) -> Result<(), ComposeCommandError> {
    let handle = thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| ComposeCommandError::Spawn {
                command: "docker compose down".into(),
                source: io::Error::other(err),
            })?
            .block_on(async {
                let _access = project.lock_mutation().await;
                match project.down_unlocked().await {
                    Ok(()) => Ok(()),
                    Err(primary) => {
                        project
                            .down_with_fallback_unlocked()
                            .await
                            .map_err(|fallback| ComposeCommandError::CleanupFallback {
                                primary: Box::new(primary),
                                fallback: Box::new(fallback),
                            })
                    }
                }
            })
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
            compose_file = %self.project.compose_file().display(),
            project = %self.project.name(),
            root = %self.project.root().display(),
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

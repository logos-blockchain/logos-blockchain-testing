use std::{env, io, sync::Arc, thread};

use testing_framework_core::scenario::CleanupGuard;
use tracing::{debug, info, warn};

use crate::{
    docker::{commands::ComposeCommandError, workspace::ComposeWorkspace},
    env::ConfigServerHandle,
    infrastructure::project::ComposeProject,
    session::{ComposeProvisionerInner, ParticipantId, SessionPreservation},
};

/// Cleans up a compose deployment and associated cfgsync container.
pub struct RunnerCleanup {
    project: ComposeProject,
    workspace: Option<ComposeWorkspace>,
    cfgsync: Option<Box<dyn ConfigServerHandle>>,
    preserve_artifacts: bool,
    session_preserve: Option<SessionPreservation>,
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
            preserve_artifacts: false,
            session_preserve: None,
        }
    }

    /// Request artifact preservation regardless of environment variables.
    #[must_use]
    pub(crate) const fn with_preserve_artifacts(mut self, preserve_artifacts: bool) -> Self {
        self.preserve_artifacts = preserve_artifacts;
        self
    }

    /// Honor session-level preservation requests raised by any participant of
    /// the shared compose session, regardless of deployment order.
    #[must_use]
    pub(crate) fn with_session_preservation(mut self, preserve: SessionPreservation) -> Self {
        self.session_preserve = Some(preserve);
        self
    }

    fn teardown_compose(&self) -> bool {
        if let Err(err) = run_compose_down_blocking(self.project.clone()) {
            warn!(error = ?err, "docker compose down failed");
            return false;
        }
        true
    }

    /// Runs the full non-preserving teardown and reports whether the compose
    /// project actually came down.
    pub(crate) fn run(mut self) -> bool {
        self.shutdown_cfgsync();
        self.teardown_compose()
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

        self.run();
    }
}

pub(crate) fn preserve_requested() -> bool {
    env::var_os("COMPOSE_RUNNER_PRESERVE").is_some()
        || env::var_os("TESTNET_RUNNER_PRESERVE").is_some()
}

/// Aggregated preservation decision for a participant cleanup: the guard's
/// own policy, the environment, or any session participant may request it.
pub(crate) fn preserve_decision(
    preserve_artifacts: bool,
    session_preserve: Option<&SessionPreservation>,
) -> bool {
    preserve_artifacts
        || preserve_requested()
        || session_preserve.is_some_and(SessionPreservation::requested)
}

impl RunnerCleanup {
    fn should_preserve(&self) -> bool {
        preserve_decision(self.preserve_artifacts, self.session_preserve.as_ref())
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

/// Tears down one session participant: its own compose project, its registry
/// entry, and — when it was the last participant — the shared session
/// network.
///
/// When preservation is requested by the guard's own policy, the environment,
/// or any session participant, the participant's containers keep running, its
/// registration stays in place, and the shared network is left behind.
pub(crate) struct ParticipantCleanup {
    inner: Arc<ComposeProvisionerInner>,
    participant: ParticipantId,
    cleanup: Option<RunnerCleanup>,
}

impl ParticipantCleanup {
    pub(crate) fn new(
        inner: Arc<ComposeProvisionerInner>,
        participant: ParticipantId,
        cleanup: RunnerCleanup,
    ) -> Self {
        Self {
            inner,
            participant,
            cleanup: Some(cleanup),
        }
    }
}

impl CleanupGuard for ParticipantCleanup {
    fn cleanup(mut self: Box<Self>) {
        let Some(cleanup) = self.cleanup.take() else {
            return;
        };

        debug!(
            participant = ?self.participant,
            "compose participant cleanup started"
        );

        if cleanup.should_preserve() {
            self.inner.preserve().request();
            Box::new(cleanup).cleanup();
            return;
        }

        if !cleanup.run() {
            warn!(
                participant = ?self.participant,
                "compose teardown failed; keeping the participant registered so its \
                 service names stay reserved"
            );
            return;
        }

        if !self.inner.deregister(self.participant) {
            warn!(
                participant = ?self.participant,
                "stale compose participant cleanup guard"
            );
        }
        release_network_blocking(&self.inner);
    }
}

/// Removes the shared session network when this was the last participant; the
/// liveness re-check runs under the network lock so a concurrent first
/// deployment either keeps the network or recreates it afterwards.
fn release_network_blocking(inner: &Arc<ComposeProvisionerInner>) {
    let inner = Arc::clone(inner);
    let handle = thread::spawn(move || {
        let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        else {
            warn!("failed to build runtime for session network removal");
            return;
        };
        runtime.block_on(inner.release_network_if_unused());
    });
    if handle.join().is_err() {
        warn!("join failure releasing the shared session network");
    }
}

#[cfg(test)]
mod tests {
    use std::{
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use testing_framework_core::scenario::CleanupGuard as _;

    use super::{ParticipantCleanup, RunnerCleanup, preserve_decision};
    use crate::{
        ComposeProvisioner, docker::workspace::ComposeWorkspace, env::ConfigServerHandle,
        infrastructure::project::ComposeProject, session::SessionPreservation,
    };

    #[derive(Default)]
    struct RecordingConfigServer {
        preserved: Arc<AtomicBool>,
        shut_down: Arc<AtomicBool>,
    }

    impl ConfigServerHandle for RecordingConfigServer {
        fn shutdown(&mut self) {
            self.shut_down.store(true, Ordering::SeqCst);
        }

        fn mark_preserved(&mut self) {
            self.preserved.store(true, Ordering::SeqCst);
        }
    }

    struct GuardProbe {
        preserved: Arc<AtomicBool>,
        shut_down: Arc<AtomicBool>,
        guard: Box<ParticipantCleanup>,
    }

    fn participant_guard(
        provisioner: &ComposeProvisioner,
        session_preserve: SessionPreservation,
    ) -> GuardProbe {
        let participant = provisioner
            .inner
            .register_stack(vec!["cleanup-probe".to_owned()])
            .expect("probe participant must register");
        let preserved = Arc::new(AtomicBool::new(false));
        let shut_down = Arc::new(AtomicBool::new(false));
        let cfgsync = RecordingConfigServer {
            preserved: Arc::clone(&preserved),
            shut_down: Arc::clone(&shut_down),
        };
        let cleanup = RunnerCleanup::new(
            ComposeProject::new(
                PathBuf::from("/nonexistent/compose.generated.yml"),
                "cleanup-test",
                PathBuf::from("/"),
            ),
            ComposeWorkspace::create().expect("workspace must be creatable"),
            Some(Box::new(cfgsync)),
        )
        .with_session_preservation(session_preserve);
        GuardProbe {
            preserved,
            shut_down,
            guard: Box::new(ParticipantCleanup::new(
                Arc::clone(&provisioner.inner),
                participant,
                cleanup,
            )),
        }
    }

    #[test]
    fn participant_first_preserve_request_reaches_the_session_cleanup() {
        let session_preserve = SessionPreservation::default();

        assert!(preserve_decision(true, Some(&session_preserve)));
    }

    #[test]
    fn preserve_requests_flip_the_shared_flag_for_later_participants() {
        let session_preserve = SessionPreservation::default();
        assert!(!preserve_decision(false, Some(&session_preserve)));

        session_preserve.request();

        assert!(preserve_decision(false, Some(&session_preserve)));
    }

    #[test]
    fn failed_teardown_keeps_the_participant_registered() {
        let provisioner = ComposeProvisioner::default();
        let probe = participant_guard(&provisioner, SessionPreservation::default());

        probe.guard.cleanup();

        assert!(
            probe.shut_down.load(Ordering::SeqCst),
            "cleanup must shut the participant's cfgsync down"
        );
        assert!(
            !probe.preserved.load(Ordering::SeqCst),
            "cleanup must not preserve without a request"
        );
        assert!(
            provisioner.inner.has_participants(),
            "a participant whose compose teardown failed must stay registered so \
             its service names stay reserved"
        );
    }

    #[test]
    fn session_preserve_request_leaves_a_participant_running_and_registered() {
        let provisioner = ComposeProvisioner::default();
        let shared_preserve = SessionPreservation::default();
        shared_preserve.request();
        let probe = participant_guard(&provisioner, shared_preserve);

        probe.guard.cleanup();

        assert!(
            probe.preserved.load(Ordering::SeqCst),
            "session-wide preserve must mark the participant's cfgsync preserved"
        );
        assert!(
            !probe.shut_down.load(Ordering::SeqCst),
            "session-wide preserve must not shut the participant's cfgsync down"
        );
        assert!(
            provisioner.inner.has_participants(),
            "session-wide preserve must keep the participant registered"
        );
        assert!(
            provisioner.inner.preserve().requested(),
            "a preserving participant must raise the shared session flag"
        );
    }

    #[test]
    fn stale_participant_guard_is_a_no_op() {
        let provisioner = ComposeProvisioner::default();
        let probe = participant_guard(&provisioner, SessionPreservation::default());
        let sibling = provisioner
            .inner
            .register_stack(vec!["sibling".to_owned()])
            .expect("sibling participant must register");

        provisioner.inner.deregister(probe.guard.participant);
        let stale = probe;

        stale.guard.cleanup();

        assert!(
            stale.shut_down.load(Ordering::SeqCst),
            "a guard always shuts its own cfgsync down"
        );
        assert!(
            provisioner.inner.deregister(sibling),
            "a stale guard must leave other participants registered"
        );
    }
}

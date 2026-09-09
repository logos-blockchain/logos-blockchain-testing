use std::{env, io, sync::Arc, thread};

use testing_framework_core::scenario::CleanupGuard;
use tracing::{debug, info, warn};

use crate::{
    docker::{commands::ComposeCommandError, workspace::ComposeWorkspace},
    env::ConfigServerHandle,
    infrastructure::{project::ComposeProject, template::write_compose_file},
    session::{
        ClusterKey, ComposeProvisionerInner, SessionPreservation, descriptor_without_cluster,
    },
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

pub(crate) fn preserve_requested() -> bool {
    env::var_os("COMPOSE_RUNNER_PRESERVE").is_some()
        || env::var_os("TESTNET_RUNNER_PRESERVE").is_some()
}

/// Aggregated preservation decision for a session-owning cleanup: the guard's
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

/// Removes one cluster's services from the shared compose session without
/// touching the container services or the project itself.
pub(crate) struct ClusterServicesCleanup {
    inner: Arc<ComposeProvisionerInner>,
    project: ComposeProject,
    key: ClusterKey,
    services: Vec<String>,
    cfgsync: Option<Box<dyn ConfigServerHandle>>,
    preserve_artifacts: bool,
    session_preserve: SessionPreservation,
    generation: u64,
}

impl ClusterServicesCleanup {
    #[expect(
        clippy::too_many_arguments,
        reason = "cleanup guard captures the full session context it acts on"
    )]
    pub(crate) fn new(
        inner: Arc<ComposeProvisionerInner>,
        project: ComposeProject,
        key: ClusterKey,
        services: Vec<String>,
        cfgsync: Option<Box<dyn ConfigServerHandle>>,
        preserve_artifacts: bool,
        session_preserve: SessionPreservation,
        generation: u64,
    ) -> Self {
        Self {
            inner,
            project,
            key,
            services,
            cfgsync,
            preserve_artifacts,
            session_preserve,
            generation,
        }
    }
}

impl CleanupGuard for ClusterServicesCleanup {
    fn cleanup(mut self: Box<Self>) {
        debug!(
            project = %self.project.name(),
            services = ?self.services,
            "compose cluster services cleanup started"
        );

        if preserve_decision(self.preserve_artifacts, Some(&self.session_preserve)) {
            self.session_preserve.request();
            if let Some(mut cfgsync) = self.cfgsync.take() {
                cfgsync.mark_preserved();
                self.cfgsync = Some(cfgsync);
            }
            info!("compose preserve flag set; leaving cluster services running");
            return;
        }

        if let Err(err) = run_cluster_removal_blocking(
            Arc::clone(&self.inner),
            self.project.clone(),
            self.key.clone(),
            self.services.clone(),
            self.generation,
        ) {
            warn!(
                error = ?err,
                "docker compose cluster service removal failed; keeping the cluster in the \
                 session model"
            );
        }

        if let Some(mut cfgsync) = self.cfgsync.take() {
            cfgsync.shutdown();
        }
    }
}

/// Removes one cluster's services and rewrites the shared compose file while
/// holding the provisioner mutation lock, so the remainder descriptor cannot
/// race a concurrent extension commit; the session snapshot is taken (and its
/// generation re-checked) only after the lock is acquired. A remainder
/// descriptor that fails to build aborts the removal before any mutation, so
/// docker state, the compose file, and the session model stay consistent.
fn run_cluster_removal_blocking(
    inner: Arc<ComposeProvisionerInner>,
    project: ComposeProject,
    key: ClusterKey,
    services: Vec<String>,
    generation: u64,
) -> Result<(), ComposeCommandError> {
    let handle = thread::spawn(move || {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|err| ComposeCommandError::Spawn {
                command: "docker compose rm cluster services".into(),
                source: io::Error::other(err),
            })?
            .block_on(async {
                let _mutation = inner.mutation.lock().await;
                let restored = {
                    let slot = inner
                        .session
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    match slot.as_ref() {
                        Some(session) if session.generation != generation => {
                            warn!(
                                guard_generation = generation,
                                live_generation = session.generation,
                                "stale compose cluster cleanup guard; leaving the live session \
                                 untouched"
                            );
                            return Ok(());
                        }
                        Some(session) => match descriptor_without_cluster(session, &key) {
                            Ok(descriptor) => Some(descriptor),
                            Err(source) => {
                                return Err(ComposeCommandError::Spawn {
                                    command: "rebuild compose descriptor without cluster".into(),
                                    source: io::Error::other(source.to_string()),
                                });
                            }
                        },
                        None => None,
                    }
                };

                {
                    let _access = project.lock_mutation().await;
                    project.remove_services_unlocked(&services).await?;
                    if let Some(descriptor) = restored {
                        write_compose_file(&descriptor, project.compose_file()).map_err(
                            |source| ComposeCommandError::Spawn {
                                command: "write restored compose file".into(),
                                source: io::Error::other(source),
                            },
                        )?;
                    }
                }

                let mut slot = inner
                    .session
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if let Some(session) = slot.as_mut()
                    && session.generation == generation
                {
                    session.clusters.remove(&key);
                }
                Ok(())
            })
    });

    handle.join().map_err(|_| ComposeCommandError::Spawn {
        command: "docker compose rm cluster services".into(),
        source: io::Error::other("join failure removing cluster services"),
    })?
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        path::PathBuf,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
    };

    use testing_framework_core::scenario::CleanupGuard as _;

    use super::{ClusterServicesCleanup, preserve_decision};
    use crate::{
        ComposeProvisioner,
        descriptor::NodeDescriptor,
        env::ConfigServerHandle,
        infrastructure::project::ComposeProject,
        session::{ClusterKey, ClusterServices, ComposeSession, RunnerPorts, SessionPreservation},
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

    fn session_with_cluster(generation: u64) -> ComposeSession {
        let mut clusters = BTreeMap::new();
        clusters.insert(
            ClusterKey::Named("alpha".to_owned()),
            ClusterServices::new(vec![NodeDescriptor::with_loopback_ports(
                "alpha-node-0",
                "cluster-node:local",
                vec!["/bin/node".to_owned()],
                Vec::new(),
                Vec::new(),
                vec![8080],
                Vec::new(),
                None,
            )]),
        );
        ComposeSession {
            project: ComposeProject::new(
                PathBuf::from("/nonexistent/compose.generated.yml"),
                "cleanup-test",
                PathBuf::from("/"),
            ),
            services: Vec::new(),
            runner_ports: RunnerPorts::new(),
            clusters,
            poisoned: false,
            generation,
            preserve: SessionPreservation::default(),
        }
    }

    fn cluster_cleanup(
        provisioner: &ComposeProvisioner,
        generation: u64,
    ) -> Box<ClusterServicesCleanup> {
        Box::new(ClusterServicesCleanup::new(
            Arc::clone(&provisioner.inner),
            ComposeProject::new(
                PathBuf::from("/nonexistent/compose.generated.yml"),
                "cleanup-test",
                PathBuf::from("/"),
            ),
            ClusterKey::Named("alpha".to_owned()),
            vec!["alpha-node-0".to_owned()],
            None,
            false,
            SessionPreservation::default(),
            generation,
        ))
    }

    #[test]
    fn cluster_first_preserve_request_reaches_the_session_cleanup() {
        let session_preserve = SessionPreservation::default();

        assert!(preserve_decision(true, Some(&session_preserve)));
    }

    #[test]
    fn preserving_cluster_joining_a_container_session_flips_the_shared_flag() {
        let session_preserve = SessionPreservation::default();
        assert!(!preserve_decision(false, Some(&session_preserve)));

        session_preserve.request();

        assert!(preserve_decision(false, Some(&session_preserve)));
    }

    #[test]
    fn failed_cluster_removal_keeps_the_cluster_in_the_session_model() {
        let provisioner = ComposeProvisioner::default();
        provisioner.store_session(session_with_cluster(7));

        cluster_cleanup(&provisioner, 7).cleanup();

        let session = provisioner
            .session_snapshot()
            .expect("session must survive a failed removal");
        assert!(
            session
                .clusters
                .contains_key(&ClusterKey::Named("alpha".to_owned())),
            "cluster must stay in the model when docker-level removal fails"
        );
    }

    #[test]
    fn session_preserve_request_leaves_a_joined_cluster_running_and_modeled() {
        let provisioner = ComposeProvisioner::default();
        let session = session_with_cluster(7);
        let shared_preserve = session.preserve.clone();
        shared_preserve.request();
        provisioner.store_session(session);

        let preserved = Arc::new(AtomicBool::new(false));
        let shut_down = Arc::new(AtomicBool::new(false));
        let cfgsync = RecordingConfigServer {
            preserved: Arc::clone(&preserved),
            shut_down: Arc::clone(&shut_down),
        };
        let guard = Box::new(ClusterServicesCleanup::new(
            Arc::clone(&provisioner.inner),
            ComposeProject::new(
                PathBuf::from("/nonexistent/compose.generated.yml"),
                "cleanup-test",
                PathBuf::from("/"),
            ),
            ClusterKey::Named("alpha".to_owned()),
            vec!["alpha-node-0".to_owned()],
            Some(Box::new(cfgsync)),
            false,
            shared_preserve,
            7,
        ));

        guard.cleanup();

        assert!(
            preserved.load(Ordering::SeqCst),
            "session-wide preserve must mark the joined cluster's cfgsync preserved"
        );
        assert!(
            !shut_down.load(Ordering::SeqCst),
            "session-wide preserve must not shut the joined cluster's cfgsync down"
        );
        let session = provisioner
            .session_snapshot()
            .expect("preserved session must stay live");
        assert!(
            session
                .clusters
                .contains_key(&ClusterKey::Named("alpha".to_owned())),
            "session-wide preserve must keep the joined cluster in the model"
        );
    }

    #[test]
    fn stale_cluster_cleanup_guard_leaves_a_newer_session_untouched() {
        let provisioner = ComposeProvisioner::default();
        provisioner.store_session(session_with_cluster(8));

        cluster_cleanup(&provisioner, 7).cleanup();

        let session = provisioner
            .session_snapshot()
            .expect("newer session must survive a stale guard");
        assert_eq!(session.generation, 8);
        assert!(
            session
                .clusters
                .contains_key(&ClusterKey::Named("alpha".to_owned()))
        );
    }
}

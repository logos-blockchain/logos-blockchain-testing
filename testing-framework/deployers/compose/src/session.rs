use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

use testing_framework_container::ContainerServiceSpec;
use testing_framework_core::scenario::{CleanupGuard, DynError};
use tokio::sync::Mutex as AsyncMutex;
use tracing::warn;

use crate::{
    container_stack::container_node_descriptors,
    descriptor::{ComposeDescriptor, NodeDescriptor},
    infrastructure::project::ComposeProject,
    lifecycle::cleanup::RunnerCleanup,
};

pub(crate) type RunnerPorts = BTreeMap<String, BTreeMap<String, u16>>;

/// Provisions portable container stacks and managed clusters through Docker
/// Compose.
///
/// Each provisioner instance owns one shared Compose project: whichever
/// deployment call comes first establishes the project, and later cluster or
/// container-stack deployments extend it on the same default network. Clones
/// share the session, so services provisioned through any clone reach each
/// other by Compose service name.
#[derive(Clone, Default)]
pub struct ComposeProvisioner {
    pub(crate) inner: Arc<ComposeProvisionerInner>,
}

#[derive(Default)]
pub(crate) struct ComposeProvisionerInner {
    pub(crate) mutation: AsyncMutex<()>,
    pub(crate) session: Mutex<Option<ComposeSession>>,
    generation: AtomicU64,
}

impl ComposeProvisioner {
    pub(crate) fn session_snapshot(&self) -> Option<ComposeSession> {
        self.inner
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn store_session(&self, session: ComposeSession) {
        *self
            .inner
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(session);
    }

    /// Allocates the generation id for a newly established session so stale
    /// cleanup guards from earlier sessions cannot clobber it.
    pub(crate) fn next_generation(&self) -> u64 {
        self.inner.generation.fetch_add(1, Ordering::Relaxed) + 1
    }
}

/// Identity of one managed cluster within a shared Compose session.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(crate) enum ClusterKey {
    /// The single unnamed cluster slot keeping the plain `node-N` names.
    Unnamed,
    /// A named cluster whose service names are namespaced by the name.
    Named(String),
}

impl ClusterKey {
    /// Returns the service-name namespace this cluster deploys under.
    pub(crate) fn namespace(&self) -> Option<&str> {
        match self {
            Self::Unnamed => None,
            Self::Named(name) => Some(name),
        }
    }

    /// Returns the cfgsync config file name for this cluster; the plain
    /// `cfgsync.yaml` belongs exclusively to the unnamed cluster, so owners
    /// and joiners of a shared session never overwrite each other's config.
    pub(crate) fn cfgsync_file_name(&self) -> String {
        match self {
            Self::Unnamed => "cfgsync.yaml".to_owned(),
            Self::Named(name) => format!("cfgsync-{name}.yaml"),
        }
    }
}

/// Session-level artifact-preservation request shared by all participants.
///
/// Any joiner of the session can request preservation; the session-owning
/// cleanup honors the flag regardless of the order deployments happened in.
#[derive(Clone, Debug, Default)]
pub(crate) struct SessionPreservation(Arc<AtomicBool>);

impl SessionPreservation {
    pub(crate) fn request(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub(crate) fn requested(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Cluster service definitions rendered into the shared session project.
#[derive(Clone)]
pub(crate) struct ClusterServices {
    nodes: Vec<NodeDescriptor>,
}

impl ClusterServices {
    pub(crate) const fn new(nodes: Vec<NodeDescriptor>) -> Self {
        Self { nodes }
    }

    pub(crate) fn nodes(&self) -> &[NodeDescriptor] {
        &self.nodes
    }

    pub(crate) fn service_names(&self) -> impl Iterator<Item = &str> {
        self.nodes.iter().map(NodeDescriptor::name)
    }
}

/// Shared per-provisioner Compose project state covering container services
/// and any number of keyed managed clusters.
#[derive(Clone)]
pub(crate) struct ComposeSession {
    pub(crate) project: ComposeProject,
    pub(crate) services: Vec<ContainerServiceSpec>,
    pub(crate) runner_ports: RunnerPorts,
    pub(crate) clusters: BTreeMap<ClusterKey, ClusterServices>,
    pub(crate) poisoned: bool,
    pub(crate) generation: u64,
    /// Monotonic mutation counter bumped on every committed extension so
    /// snapshot-based paths can detect that the session gained participants.
    pub(crate) epoch: u64,
    pub(crate) preserve: SessionPreservation,
}

impl ComposeSession {
    pub(crate) fn service_names(&self) -> impl Iterator<Item = &str> {
        self.services.iter().map(ContainerServiceSpec::name).chain(
            self.clusters
                .values()
                .flat_map(ClusterServices::service_names),
        )
    }

    /// Reports whether the session hosts any participant besides the given
    /// cluster.
    pub(crate) fn has_other_participants(&self, key: &ClusterKey) -> bool {
        !self.services.is_empty() || self.clusters.keys().any(|existing| existing != key)
    }
}

/// Builds the full session descriptor covering clusters and container
/// services.
pub(crate) fn session_descriptor(
    clusters: &BTreeMap<ClusterKey, ClusterServices>,
    services: &[ContainerServiceSpec],
    runner_ports: &RunnerPorts,
) -> Result<ComposeDescriptor, DynError> {
    let mut nodes: Vec<NodeDescriptor> = clusters
        .values()
        .flat_map(|cluster| cluster.nodes().iter().cloned())
        .collect();
    nodes.extend(container_node_descriptors(services, runner_ports)?);
    Ok(ComposeDescriptor::new(nodes))
}

/// Renders the descriptor the session would have without the given cluster,
/// leaving the session model untouched.
pub(crate) fn descriptor_without_cluster(
    session: &ComposeSession,
    key: &ClusterKey,
) -> Result<ComposeDescriptor, DynError> {
    let mut clusters = session.clusters.clone();
    clusters.remove(key);
    session_descriptor(&clusters, &session.services, &session.runner_ports)
}

/// Drops one cluster's services from the session model and returns the
/// descriptor that should be restored on disk for the remaining services.
///
/// A stale request from a dead session (mismatched generation) is a no-op so
/// it cannot strip services from an unrelated newer session.
pub(crate) fn strip_cluster_services(
    session: &mut Option<ComposeSession>,
    key: &ClusterKey,
    expected_generation: u64,
) -> Option<Result<ComposeDescriptor, DynError>> {
    let session = session.as_mut()?;
    if session.generation != expected_generation {
        warn!(
            expected_generation,
            live_generation = session.generation,
            "stale compose session strip request; leaving the live session untouched"
        );
        return None;
    }
    session.clusters.remove(key);
    Some(session_descriptor(
        &session.clusters,
        &session.services,
        &session.runner_ports,
    ))
}

/// Clears the shared session and tears down the whole Compose project.
pub(crate) struct ComposeSessionCleanup {
    pub(crate) inner: Arc<ComposeProvisionerInner>,
    pub(crate) cleanup: Option<RunnerCleanup>,
    pub(crate) generation: u64,
}

impl CleanupGuard for ComposeSessionCleanup {
    fn cleanup(mut self: Box<Self>) {
        {
            let mut slot = self
                .inner
                .session
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            match slot.as_ref() {
                Some(session) if session.generation != self.generation => {
                    warn!(
                        guard_generation = self.generation,
                        live_generation = session.generation,
                        "stale compose session cleanup guard; leaving the live session untouched"
                    );
                    return;
                }
                _ => *slot = None,
            }
        }
        if let Some(cleanup) = self.cleanup.take() {
            Box::new(cleanup).cleanup();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, path::PathBuf, sync::Arc};

    use testing_framework_container::{ContainerPort, ContainerServiceSpec};
    use testing_framework_core::scenario::CleanupGuard as _;

    use super::{
        ClusterKey, ClusterServices, ComposeSession, ComposeSessionCleanup, RunnerPorts,
        SessionPreservation, descriptor_without_cluster, session_descriptor,
        strip_cluster_services,
    };
    use crate::{
        ComposeProvisioner, descriptor::NodeDescriptor, infrastructure::project::ComposeProject,
    };

    fn cluster_node(name: &str) -> NodeDescriptor {
        NodeDescriptor::with_loopback_ports(
            name,
            "cluster-node:local",
            vec!["/bin/node".to_owned()],
            Vec::new(),
            Vec::new(),
            vec![8080],
            Vec::new(),
            None,
        )
    }

    fn container_service(name: &str) -> ContainerServiceSpec {
        ContainerServiceSpec::new(name, "example:local")
            .with_command(["/bin/example"])
            .with_port(ContainerPort::new("api", 8080))
    }

    fn session() -> ComposeSession {
        let mut clusters = BTreeMap::new();
        clusters.insert(
            ClusterKey::Unnamed,
            ClusterServices::new(vec![cluster_node("node-0"), cluster_node("node-1")]),
        );
        clusters.insert(
            ClusterKey::Named("alpha".to_owned()),
            ClusterServices::new(vec![cluster_node("alpha-node-0")]),
        );
        ComposeSession {
            project: ComposeProject::new(
                PathBuf::from("/tmp/compose.yml"),
                "session-project",
                PathBuf::from("/tmp"),
            ),
            services: vec![container_service("worker")],
            runner_ports: RunnerPorts::new(),
            clusters,
            poisoned: false,
            generation: 1,
            epoch: 1,
            preserve: SessionPreservation::default(),
        }
    }

    #[test]
    fn descriptor_merges_all_clusters_and_container_services() {
        let session = session();

        let descriptor =
            session_descriptor(&session.clusters, &session.services, &session.runner_ports)
                .unwrap();

        let names: Vec<_> = descriptor
            .nodes()
            .iter()
            .map(NodeDescriptor::name)
            .collect();
        assert_eq!(names, ["node-0", "node-1", "alpha-node-0", "worker"]);
    }

    #[test]
    fn session_names_span_both_service_kinds() {
        let session = session();

        let names: Vec<_> = session.service_names().collect();

        assert_eq!(names, ["worker", "node-0", "node-1", "alpha-node-0"]);
    }

    #[test]
    fn stripping_one_cluster_keeps_other_services_in_the_model() {
        let mut slot = Some(session());

        let restored = strip_cluster_services(&mut slot, &ClusterKey::Unnamed, 1)
            .expect("active session should produce a restore descriptor")
            .expect("remaining descriptor should render");

        let names: Vec<_> = restored.nodes().iter().map(NodeDescriptor::name).collect();
        assert_eq!(names, ["alpha-node-0", "worker"]);
        let session = slot.expect("session should stay active");
        assert_eq!(session.clusters.len(), 1);
        assert!(
            session
                .clusters
                .contains_key(&ClusterKey::Named("alpha".to_owned()))
        );
        assert_eq!(session.services.len(), 1);
    }

    #[test]
    fn stripping_without_a_session_is_a_no_op() {
        let mut slot = None;

        assert!(strip_cluster_services(&mut slot, &ClusterKey::Unnamed, 1).is_none());
    }

    #[test]
    fn stripping_with_a_stale_generation_leaves_a_newer_session_untouched() {
        let mut newer = session();
        newer.generation = 2;
        let mut slot = Some(newer);

        assert!(strip_cluster_services(&mut slot, &ClusterKey::Unnamed, 1).is_none());
        let session = slot.expect("newer session must survive a stale strip");
        assert!(session.clusters.contains_key(&ClusterKey::Unnamed));
    }

    #[test]
    fn cluster_keys_map_to_distinct_cfgsync_files() {
        assert_eq!(ClusterKey::Unnamed.cfgsync_file_name(), "cfgsync.yaml");
        assert_eq!(
            ClusterKey::Named("alpha".to_owned()).cfgsync_file_name(),
            "cfgsync-alpha.yaml"
        );
    }

    #[test]
    fn other_participants_are_detected_across_service_kinds() {
        let session = session();
        let alpha = ClusterKey::Named("alpha".to_owned());

        assert!(session.has_other_participants(&alpha));

        let mut solo = session.clone();
        solo.services.clear();
        solo.clusters.retain(|key, _| *key == alpha);
        assert!(!solo.has_other_participants(&alpha));
        assert!(solo.has_other_participants(&ClusterKey::Unnamed));
    }

    #[test]
    fn peeking_a_remainder_descriptor_leaves_the_model_untouched() {
        let session = session();

        let remainder =
            descriptor_without_cluster(&session, &ClusterKey::Named("alpha".to_owned())).unwrap();

        let names: Vec<_> = remainder.nodes().iter().map(NodeDescriptor::name).collect();
        assert_eq!(names, ["node-0", "node-1", "worker"]);
        assert_eq!(session.clusters.len(), 2);
    }

    #[test]
    fn session_cleanup_clears_a_matching_generation() {
        let provisioner = ComposeProvisioner::default();
        provisioner.store_session(session());

        let guard = ComposeSessionCleanup {
            inner: Arc::clone(&provisioner.inner),
            cleanup: None,
            generation: 1,
        };
        Box::new(guard).cleanup();

        assert!(provisioner.session_snapshot().is_none());
    }

    #[test]
    fn stale_session_cleanup_leaves_a_newer_session_untouched() {
        let provisioner = ComposeProvisioner::default();
        let mut live = session();
        live.generation = 2;
        provisioner.store_session(live);

        let stale = ComposeSessionCleanup {
            inner: Arc::clone(&provisioner.inner),
            cleanup: None,
            generation: 1,
        };
        Box::new(stale).cleanup();

        let snapshot = provisioner
            .session_snapshot()
            .expect("live session must survive a stale guard");
        assert_eq!(snapshot.generation, 2);
    }

    #[test]
    fn preservation_requests_are_shared_between_clones() {
        let preserve = SessionPreservation::default();
        let clone = preserve.clone();

        assert!(!preserve.requested());
        clone.request();
        assert!(preserve.requested());
    }
}

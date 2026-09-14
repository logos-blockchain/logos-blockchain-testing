//! Shared compose session state: one external Docker network plus a
//! participant registry.
//!
//! Every `deploy_cluster` and `deploy_container_stack` call provisions its own
//! workspace, compose file, and compose project; the only state shared between
//! participants of one provisioner is the session network they all attach to,
//! the registry guarding cluster keys and service names, and the session-wide
//! preservation flag.
//!
//! The session network is labeled `testing-framework=session`. A crashed
//! process can leak it (no cleanup guard ever runs); leaked networks are
//! removed with `docker network rm $(docker network ls -q --filter
//! label=testing-framework=session)` once no containers are attached.

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{errors::ComposeRunnerError, infrastructure::network::SharedNetwork};

/// Provisions portable container stacks and managed clusters through Docker
/// Compose.
///
/// Each provisioner instance owns one shared session network: every cluster or
/// container-stack deployment runs in its own Compose project and attaches its
/// services to that network. Clones share the session, so services provisioned
/// through any clone reach each other by Compose service name.
#[derive(Clone, Default)]
pub struct ComposeProvisioner {
    pub(crate) inner: Arc<ComposeProvisionerInner>,
}

pub(crate) struct ComposeProvisionerInner {
    registry: Mutex<ParticipantRegistry>,
    network: SharedNetwork,
    preserve: SessionPreservation,
}

impl Default for ComposeProvisionerInner {
    fn default() -> Self {
        Self {
            registry: Mutex::default(),
            network: SharedNetwork::new(),
            preserve: SessionPreservation::default(),
        }
    }
}

impl ComposeProvisionerInner {
    pub(crate) fn register_cluster(
        &self,
        key: &ClusterKey,
        services: Vec<String>,
    ) -> Result<ParticipantId, ComposeRunnerError> {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .register(Some(key.clone()), services)
    }

    pub(crate) fn register_stack(
        &self,
        services: Vec<String>,
    ) -> Result<ParticipantId, ComposeRunnerError> {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .register(None, services)
    }

    /// Removes the participant's registry entry; returns whether the entry
    /// was still present, so a stale guard's second run becomes a no-op.
    pub(crate) fn deregister(&self, participant: ParticipantId) -> bool {
        self.registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .deregister(participant)
    }

    pub(crate) fn has_participants(&self) -> bool {
        !self
            .registry
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty()
    }

    pub(crate) const fn network(&self) -> &SharedNetwork {
        &self.network
    }

    pub(crate) const fn preserve(&self) -> &SessionPreservation {
        &self.preserve
    }

    /// Removes the session network when the last participant is gone and no
    /// preservation was requested; the emptiness check runs under the network
    /// lock so it cannot race a concurrent first deployment.
    pub(crate) async fn release_network_if_unused(&self) {
        self.network
            .remove_if_unused(|| self.has_participants() || self.preserve.requested())
            .await;
    }
}

/// Opaque identity of one session participant (a cluster or container stack).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct ParticipantId(u64);

#[derive(Default)]
struct ParticipantRegistry {
    next_id: u64,
    participants: BTreeMap<u64, ParticipantEntry>,
}

struct ParticipantEntry {
    cluster: Option<ClusterKey>,
    services: Vec<String>,
}

impl ParticipantRegistry {
    fn register(
        &mut self,
        cluster: Option<ClusterKey>,
        services: Vec<String>,
    ) -> Result<ParticipantId, ComposeRunnerError> {
        if let Some(key) = cluster.as_ref()
            && self
                .participants
                .values()
                .any(|entry| entry.cluster.as_ref() == Some(key))
        {
            return Err(match key {
                ClusterKey::Unnamed => ComposeRunnerError::UnnamedClusterAlreadyProvisioned,
                ClusterKey::Named(name) => {
                    ComposeRunnerError::ClusterAlreadyProvisioned { name: name.clone() }
                }
            });
        }
        if let Some(name) = services.iter().find(|name| {
            self.participants
                .values()
                .flat_map(|entry| entry.services.iter())
                .any(|existing| existing == *name)
        }) {
            return Err(ComposeRunnerError::ServiceNameConflict { name: name.clone() });
        }

        self.next_id += 1;
        self.participants
            .insert(self.next_id, ParticipantEntry { cluster, services });
        Ok(ParticipantId(self.next_id))
    }

    fn deregister(&mut self, participant: ParticipantId) -> bool {
        self.participants.remove(&participant.0).is_some()
    }

    fn is_empty(&self) -> bool {
        self.participants.is_empty()
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

    /// Returns the cfgsync config file name for this cluster within its own
    /// workspace.
    pub(crate) fn cfgsync_file_name(&self) -> String {
        match self {
            Self::Unnamed => "cfgsync.yaml".to_owned(),
            Self::Named(name) => format!("cfgsync-{name}.yaml"),
        }
    }
}

/// Session-level artifact-preservation request shared by all participants.
///
/// Any participant of the session can request preservation; every
/// participant's cleanup honors the flag regardless of the order deployments
/// happened in.
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

#[cfg(test)]
mod tests {
    use super::{ClusterKey, SessionPreservation};
    use crate::{ComposeProvisioner, errors::ComposeRunnerError};

    fn named_key(name: &str) -> ClusterKey {
        ClusterKey::Named(name.to_owned())
    }

    #[test]
    fn cluster_keys_map_to_distinct_cfgsync_files() {
        assert_eq!(ClusterKey::Unnamed.cfgsync_file_name(), "cfgsync.yaml");
        assert_eq!(named_key("alpha").cfgsync_file_name(), "cfgsync-alpha.yaml");
    }

    #[test]
    fn duplicate_cluster_keys_are_rejected() {
        let provisioner = ComposeProvisioner::default();
        provisioner
            .inner
            .register_cluster(&named_key("alpha"), vec!["alpha-node-0".to_owned()])
            .expect("first alpha cluster must register");
        provisioner
            .inner
            .register_cluster(&ClusterKey::Unnamed, vec!["node-0".to_owned()])
            .expect("first unnamed cluster must register");

        let named = provisioner
            .inner
            .register_cluster(&named_key("alpha"), vec!["alpha-node-1".to_owned()])
            .err()
            .expect("duplicate named cluster must be rejected");
        assert!(matches!(
            named,
            ComposeRunnerError::ClusterAlreadyProvisioned { name } if name == "alpha"
        ));

        let unnamed = provisioner
            .inner
            .register_cluster(&ClusterKey::Unnamed, vec!["node-1".to_owned()])
            .err()
            .expect("duplicate unnamed cluster must be rejected");
        assert!(matches!(
            unnamed,
            ComposeRunnerError::UnnamedClusterAlreadyProvisioned
        ));
    }

    #[test]
    fn cross_participant_service_name_conflicts_are_rejected() {
        let provisioner = ComposeProvisioner::default();
        provisioner
            .inner
            .register_stack(vec!["worker".to_owned(), "node-1".to_owned()])
            .expect("stack must register");

        let error = provisioner
            .inner
            .register_cluster(
                &ClusterKey::Unnamed,
                vec!["node-0".to_owned(), "node-1".to_owned()],
            )
            .err()
            .expect("colliding names must be rejected");

        assert!(matches!(
            error,
            ComposeRunnerError::ServiceNameConflict { name } if name == "node-1"
        ));
    }

    #[test]
    fn deregistration_releases_names_and_is_idempotent() {
        let provisioner = ComposeProvisioner::default();
        let participant = provisioner
            .inner
            .register_stack(vec!["worker".to_owned()])
            .expect("stack must register");
        assert!(provisioner.inner.has_participants());

        assert!(provisioner.inner.deregister(participant));
        assert!(!provisioner.inner.has_participants());
        assert!(
            !provisioner.inner.deregister(participant),
            "a stale second deregistration must be a no-op"
        );

        provisioner
            .inner
            .register_stack(vec!["worker".to_owned()])
            .expect("released names must be reusable");
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

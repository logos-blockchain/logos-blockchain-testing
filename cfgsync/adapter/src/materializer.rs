use std::{error::Error, sync::Mutex};

use cfgsync_core::NodeRegistration;
use serde_json::to_string;

use crate::{MaterializedArtifacts, RegistrationSnapshot, ResolvedNodeArtifacts};

/// Type-erased cfgsync adapter error used to preserve source context.
pub type DynCfgsyncError = Box<dyn Error + Send + Sync + 'static>;

/// Adapter-side materialization contract for a single registered node.
pub trait NodeArtifactsMaterializer: Send + Sync {
    /// Resolves one node from the current registration set.
    ///
    /// Returning `Ok(None)` means the node is known but its artifacts are not
    /// ready yet.
    fn materialize(
        &self,
        registration: &NodeRegistration,
        registrations: &RegistrationSnapshot,
    ) -> Result<Option<ResolvedNodeArtifacts>, DynCfgsyncError>;
}

/// Adapter contract for materializing a whole registration snapshot into
/// per-node cfgsync artifacts.
pub trait RegistrationSnapshotMaterializer: Send + Sync {
    /// Materializes the current registration set.
    ///
    /// This is the main registration-driven integration point for cfgsync.
    /// Implementations decide:
    /// - when the current snapshot is ready to serve
    /// - which per-node artifacts should be produced
    /// - which shared artifacts should accompany every node
    fn materialize_snapshot(
        &self,
        registrations: &RegistrationSnapshot,
    ) -> Result<MaterializationResult, DynCfgsyncError>;
}

/// Optional hook for persisting or publishing materialized cfgsync artifacts.
pub trait MaterializedArtifactsSink: Send + Sync {
    /// Persists or publishes a ready materialization result.
    fn persist(&self, artifacts: &MaterializedArtifacts) -> Result<(), DynCfgsyncError>;
}

/// Registration-driven materialization status.
#[derive(Debug, Clone, Default)]
pub enum MaterializationResult {
    #[default]
    NotReady,
    Ready(MaterializedArtifacts),
}

impl MaterializationResult {
    /// Creates a ready materialization result.
    #[must_use]
    pub fn ready(nodes: MaterializedArtifacts) -> Self {
        Self::Ready(nodes)
    }

    /// Returns the ready artifacts when materialization succeeded.
    #[must_use]
    pub fn artifacts(&self) -> Option<&MaterializedArtifacts> {
        match self {
            Self::NotReady => None,
            Self::Ready(artifacts) => Some(artifacts),
        }
    }
}

/// Snapshot materializer wrapper that caches the last materialized result.
pub struct CachedSnapshotMaterializer<M> {
    inner: M,
    cache: Mutex<Option<CachedSnapshot>>,
}

struct CachedSnapshot {
    key: String,
    result: MaterializationResult,
}

impl<M> CachedSnapshotMaterializer<M> {
    /// Wraps a snapshot materializer with deterministic snapshot-result
    /// caching.
    #[must_use]
    pub fn new(inner: M) -> Self {
        Self {
            inner,
            cache: Mutex::new(None),
        }
    }

    fn snapshot_key(registrations: &RegistrationSnapshot) -> Result<String, DynCfgsyncError> {
        Ok(to_string(registrations)?)
    }
}

impl<M> RegistrationSnapshotMaterializer for CachedSnapshotMaterializer<M>
where
    M: RegistrationSnapshotMaterializer,
{
    fn materialize_snapshot(
        &self,
        registrations: &RegistrationSnapshot,
    ) -> Result<MaterializationResult, DynCfgsyncError> {
        let key = Self::snapshot_key(registrations)?;

        {
            let cache = self
                .cache
                .lock()
                .expect("cfgsync snapshot cache should not be poisoned");

            if let Some(cached) = &*cache
                && cached.key == key
            {
                return Ok(cached.result.clone());
            }
        }

        let result = self.inner.materialize_snapshot(registrations)?;
        let mut cache = self
            .cache
            .lock()
            .expect("cfgsync snapshot cache should not be poisoned");

        *cache = Some(CachedSnapshot {
            key,
            result: result.clone(),
        });

        Ok(result)
    }
}

/// Snapshot materializer wrapper that persists ready results through a generic
/// sink. It only persists once per distinct registration snapshot.
pub struct PersistingSnapshotMaterializer<M, S> {
    inner: M,
    sink: S,
    persisted_key: Mutex<Option<String>>,
}

impl<M, S> PersistingSnapshotMaterializer<M, S> {
    /// Wraps a snapshot materializer with one-time persistence for each
    /// distinct registration snapshot.
    #[must_use]
    pub fn new(inner: M, sink: S) -> Self {
        Self {
            inner,
            sink,
            persisted_key: Mutex::new(None),
        }
    }
}

impl<M, S> RegistrationSnapshotMaterializer for PersistingSnapshotMaterializer<M, S>
where
    M: RegistrationSnapshotMaterializer,
    S: MaterializedArtifactsSink,
{
    fn materialize_snapshot(
        &self,
        registrations: &RegistrationSnapshot,
    ) -> Result<MaterializationResult, DynCfgsyncError> {
        let key = CachedSnapshotMaterializer::<M>::snapshot_key(registrations)?;
        let result = self.inner.materialize_snapshot(registrations)?;

        let Some(artifacts) = result.artifacts() else {
            return Ok(result);
        };

        {
            let persisted_key = self
                .persisted_key
                .lock()
                .expect("cfgsync persistence state should not be poisoned");

            if persisted_key.as_deref() == Some(&key) {
                return Ok(result);
            }
        }

        self.sink.persist(artifacts)?;

        let mut persisted_key = self
            .persisted_key
            .lock()
            .expect("cfgsync persistence state should not be poisoned");
        *persisted_key = Some(key);

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use cfgsync_artifacts::ArtifactFile;

    use super::{
        CachedSnapshotMaterializer, DynCfgsyncError, MaterializationResult, MaterializedArtifacts,
        MaterializedArtifactsSink, PersistingSnapshotMaterializer,
        RegistrationSnapshotMaterializer,
    };
    use crate::{ArtifactSet, NodeArtifacts, NodeArtifactsCatalog, RegistrationSnapshot};

    struct CountingMaterializer;

    impl RegistrationSnapshotMaterializer for CountingMaterializer {
        fn materialize_snapshot(
            &self,
            registrations: &RegistrationSnapshot,
        ) -> Result<MaterializationResult, DynCfgsyncError> {
            if registrations.is_empty() {
                return Ok(MaterializationResult::NotReady);
            }

            let nodes = registrations
                .iter()
                .map(|registration| NodeArtifacts {
                    identifier: registration.identifier.clone(),
                    files: vec![ArtifactFile::new("/config.yaml", "ready: true")],
                })
                .collect();

            Ok(MaterializationResult::ready(MaterializedArtifacts::new(
                NodeArtifactsCatalog::new(nodes),
                ArtifactSet::new(vec![ArtifactFile::new("/shared.yaml", "cluster: ready")]),
            )))
        }
    }

    struct CountingSink {
        writes: Arc<AtomicUsize>,
    }

    impl MaterializedArtifactsSink for CountingSink {
        fn persist(&self, _artifacts: &MaterializedArtifacts) -> Result<(), DynCfgsyncError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn persisting_snapshot_materializer_writes_ready_snapshots_once() {
        let writes = Arc::new(AtomicUsize::new(0));
        let materializer = CachedSnapshotMaterializer::new(PersistingSnapshotMaterializer::new(
            CountingMaterializer,
            CountingSink {
                writes: Arc::clone(&writes),
            },
        ));

        let empty = RegistrationSnapshot::default();
        let ready = RegistrationSnapshot::new(vec![cfgsync_core::NodeRegistration::new(
            "node-0",
            "127.0.0.1".parse().expect("parse ip"),
        )]);

        let _ = materializer
            .materialize_snapshot(&empty)
            .expect("not-ready snapshot");
        let _ = materializer
            .materialize_snapshot(&ready)
            .expect("ready snapshot");
        let _ = materializer
            .materialize_snapshot(&ready)
            .expect("cached ready snapshot");

        assert_eq!(writes.load(Ordering::SeqCst), 1);
    }
}

use std::{error::Error, sync::Mutex};

use cfgsync_core::NodeRegistration;
use serde_json::to_string;

use crate::{ArtifactSet, NodeArtifactsCatalog, RegistrationSnapshot};

/// Type-erased cfgsync adapter error used to preserve source context.
pub type DynCfgsyncError = Box<dyn Error + Send + Sync + 'static>;

/// Adapter-side materialization contract for a single registered node.
pub trait NodeArtifactsMaterializer: Send + Sync {
    fn materialize(
        &self,
        registration: &NodeRegistration,
        registrations: &RegistrationSnapshot,
    ) -> Result<Option<ArtifactSet>, DynCfgsyncError>;
}

/// Adapter contract for materializing a whole registration snapshot into
/// per-node cfgsync artifacts.
pub trait RegistrationSnapshotMaterializer: Send + Sync {
    fn materialize_snapshot(
        &self,
        registrations: &RegistrationSnapshot,
    ) -> Result<Option<NodeArtifactsCatalog>, DynCfgsyncError>;
}

/// Snapshot materializer wrapper that caches the last materialized result.
pub struct CachedSnapshotMaterializer<M> {
    inner: M,
    cache: Mutex<Option<CachedSnapshot>>,
}

struct CachedSnapshot {
    key: String,
    catalog: Option<NodeArtifactsCatalog>,
}

impl<M> CachedSnapshotMaterializer<M> {
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
    ) -> Result<Option<NodeArtifactsCatalog>, DynCfgsyncError> {
        let key = Self::snapshot_key(registrations)?;

        {
            let cache = self
                .cache
                .lock()
                .expect("cfgsync snapshot cache should not be poisoned");

            if let Some(cached) = &*cache
                && cached.key == key
            {
                return Ok(cached.catalog.clone());
            }
        }

        let catalog = self.inner.materialize_snapshot(registrations)?;
        let mut cache = self
            .cache
            .lock()
            .expect("cfgsync snapshot cache should not be poisoned");

        *cache = Some(CachedSnapshot {
            key,
            catalog: catalog.clone(),
        });

        Ok(catalog)
    }
}

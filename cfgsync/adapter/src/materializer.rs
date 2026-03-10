use std::error::Error;

use cfgsync_core::NodeRegistration;

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

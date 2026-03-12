mod artifacts;
mod deployment;
mod materializer;
mod registrations;
mod sources;

pub use artifacts::MaterializedArtifacts;
pub use materializer::{
    CachedSnapshotMaterializer, DynCfgsyncError, MaterializationResult, MaterializedArtifactsSink,
    PersistingSnapshotMaterializer, RegistrationSnapshotMaterializer,
};
pub use registrations::RegistrationSnapshot;
pub use sources::RegistrationConfigSource;

/// Static deployment helpers for precomputed cfgsync artifact generation.
///
/// This module is intentionally secondary to the registration-backed
/// materializer flow. Use it when artifacts are already determined by a
/// deployment plan and do not need runtime registration to become available.
pub mod static_deployment {
    pub use super::deployment::{
        BuildCfgsyncNodesError, DeploymentAdapter, build_materialized_artifacts,
    };
}

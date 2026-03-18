#![doc(hidden)]

pub use crate::{
    bundle::{NodeArtifactsBundle as CfgSyncBundle, NodeArtifactsBundleEntry as CfgSyncBundleNode},
    client::Client as CfgSyncClient,
    protocol::{
        CfgsyncErrorCode as CfgSyncErrorCode, CfgsyncErrorResponse as CfgSyncErrorResponse,
        ConfigResolveResponse as RepoResponse, NodeArtifactFile as CfgSyncFile,
        NodeArtifactsPayload as CfgSyncPayload, RegisterNodeResponse as RegistrationResponse,
    },
    server::{
        CfgsyncServerState as CfgSyncState, build_legacy_cfgsync_router as cfgsync_app,
        serve_cfgsync as run_cfgsync,
    },
    source::{
        BundleConfigSource as FileConfigProvider,
        BundleConfigSourceError as FileConfigProviderError, NodeConfigSource as ConfigProvider,
        StaticConfigSource as ConfigRepo,
    },
};

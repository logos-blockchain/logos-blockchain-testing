pub mod bundle;
pub mod client;
pub mod render;
pub mod repo;
pub mod server;

#[doc(hidden)]
pub use bundle::{CfgSyncBundle, CfgSyncBundleNode};
pub use bundle::{NodeArtifactsBundle, NodeArtifactsBundleEntry};
pub use client::{CfgSyncClient, ClientError, ConfigFetchStatus};
pub use render::{
    CfgsyncConfigOverrides, CfgsyncOutputPaths, RenderedCfgsync, apply_cfgsync_overrides,
    apply_timeout_floor, ensure_bundle_path, load_cfgsync_template_yaml,
    render_cfgsync_yaml_from_template, write_rendered_cfgsync,
};
pub use repo::{
    BundleConfigSource, BundleConfigSourceError, CFGSYNC_SCHEMA_VERSION, CfgSyncErrorCode,
    CfgSyncErrorResponse, ConfigResolveResponse, NodeArtifactFile, NodeArtifactsPayload,
    NodeConfigSource, NodeRegistration, RegisterNodeResponse, RegistrationPayload,
    StaticConfigSource,
};
#[doc(hidden)]
pub use repo::{
    CfgSyncFile, CfgSyncPayload, ConfigProvider, ConfigRepo, FileConfigProvider,
    FileConfigProviderError, RegistrationResponse, RepoResponse,
};
#[doc(hidden)]
pub use server::CfgSyncState;
pub use server::{CfgsyncServerState, RunCfgsyncError, cfgsync_app, run_cfgsync};

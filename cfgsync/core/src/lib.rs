pub mod bundle;
pub mod client;
#[doc(hidden)]
pub mod compat;
pub mod protocol;
pub mod render;
pub mod server;
pub mod source;

pub use bundle::{NodeArtifactsBundle, NodeArtifactsBundleEntry};
pub use client::{Client, ClientError, ConfigFetchStatus};
pub use protocol::{
    CFGSYNC_SCHEMA_VERSION, CfgsyncErrorCode, CfgsyncErrorResponse, ConfigResolveResponse,
    NodeArtifactFile, NodeArtifactsPayload, NodeRegistration, RegisterNodeResponse,
    RegistrationPayload, ReplaceNodeArtifactsRequest,
};
pub use render::{
    CfgsyncConfigOverrides, CfgsyncOutputPaths, RenderedCfgsync, apply_cfgsync_overrides,
    apply_timeout_floor, ensure_artifacts_path, load_cfgsync_template_yaml,
    render_cfgsync_yaml_from_template, write_rendered_cfgsync,
};
pub use server::{CfgsyncServerState, RunCfgsyncError, build_cfgsync_router, serve_cfgsync};
pub use source::{
    BundleConfigSource, BundleConfigSourceError, BundleLoadError, NodeConfigSource,
    StaticConfigSource, bundle_to_payload_map, load_bundle,
};

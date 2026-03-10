pub use cfgsync_core as core;

mod client;
mod server;

pub use client::{
    ArtifactOutputMap, fetch_and_write_artifacts, register_and_fetch_artifacts,
    run_cfgsync_client_from_env,
};
pub use server::{
    CfgsyncServerConfig, CfgsyncServerSource, LoadCfgsyncServerConfigError,
    build_persisted_snapshot_cfgsync_router, build_snapshot_cfgsync_router,
    serve_cfgsync_from_config, serve_persisted_snapshot_cfgsync, serve_snapshot_cfgsync,
};

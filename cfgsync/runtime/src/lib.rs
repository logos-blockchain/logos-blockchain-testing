pub use cfgsync_core as core;

mod client;
mod server;

pub use client::{Client, OutputMap, run_client_from_env};
pub use server::{
    LoadServerConfigError, ServerConfig, ServerSource, build_persisted_router, build_router, serve,
    serve_from_config, serve_persisted,
};

pub use cfgsync_core as core;

mod client;
mod server;

pub use client::run_cfgsync_client_from_env;
pub use server::{CfgSyncServerConfig, run_cfgsync_server};

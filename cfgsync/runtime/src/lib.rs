pub use cfgsync_core as core;

mod client;
mod server;

pub use client::run_cfgsync_client_from_env;
#[doc(hidden)]
pub use server::CfgSyncServerConfig;
pub use server::{
    CfgsyncServerConfig, CfgsyncServingMode, LoadCfgsyncServerConfigError, run_cfgsync_server,
};

pub mod binary;
mod deployer;
pub mod env;
mod external;
mod manual;
mod node_control;
pub mod process;

pub use binary::{BinaryConfig, BinaryResolver};
pub use deployer::{ProcessDeployer, ProcessDeployerError};
pub use env::{BuiltNodeConfig, LocalDeployerEnv, NodeConfigEntry};
pub use manual::{ManualCluster, ManualClusterError};
pub use node_control::{NodeManager, NodeManagerError, NodeManagerSeed};
pub use process::{
    LaunchEnvVar, LaunchFile, LaunchSpec, NodeEndpointPort, NodeEndpoints, ProcessNode,
    ProcessSpawnError,
};

const KEEP_LOGS_ENV: &str = "TF_KEEP_LOGS";

pub(crate) fn keep_tempdir_from_env() -> bool {
    env_enabled(KEEP_LOGS_ENV)
}

fn env_enabled(key: &str) -> bool {
    std::env::var(key).ok().is_some_and(|value| {
        value == "1" || value.eq_ignore_ascii_case("true") || value.eq_ignore_ascii_case("yes")
    })
}

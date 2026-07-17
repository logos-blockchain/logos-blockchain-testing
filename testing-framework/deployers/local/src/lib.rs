pub mod binary;
mod deployer;
pub mod env;
mod external;
mod manual;
mod node_control;
pub mod process;

pub use binary::{
    BinaryProvider, BinaryProviderError, BinaryProviderRef, BuildBinaryProvider, BuildCommand,
    DownloadBinaryProvider, DownloadChecksum, DownloadProcessor, DownloadProcessorError,
    DownloadProcessorFn, DownloadUrl, EnvBinaryProvider, FallbackBinaryProvider,
    PathBinaryProvider,
};
pub use deployer::{ProcessDeployer, ProcessDeployerError};
pub use env::{
    BuiltNodeConfig, LocalBinaryApp, LocalBuildContext, LocalConfigArgMode, LocalDeployerEnv,
    LocalNodePorts, LocalPeerNode, LocalProcessSpec, LocalReadinessProbe, NodeConfigEntry,
    build_indexed_http_peers, build_indexed_node_configs, build_local_cluster_node_config,
    build_local_peer_nodes, default_yaml_launch_spec, discovered_node_access, preallocate_ports,
    reserve_local_node_ports, single_http_node_endpoints, text_config_launch_spec,
    text_node_config, yaml_config_launch_spec, yaml_node_config,
};
pub use manual::{ManualCluster, ManualClusterError};
pub use node_control::{NodeManager, NodeManagerError, NodeManagerSeed};
pub use process::{
    LaunchEnvVar, LaunchFile, LaunchSpec, NodeEndpointPort, NodeEndpoints, ProcessNode,
    ProcessSpawnError, allocate_available_port,
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

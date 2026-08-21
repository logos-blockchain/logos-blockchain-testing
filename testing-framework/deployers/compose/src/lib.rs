mod container_stack;
pub mod deployer;
pub mod descriptor;
pub mod docker;
pub mod env;
pub mod errors;
pub mod infrastructure;
pub mod lifecycle;

pub use container_stack::ComposeContainerProvisioner;
pub use deployer::{ComposeDeployer, ComposeDeploymentMetadata};
pub use descriptor::{
    BinaryConfigNodeSpec, ComposeDescriptor, EnvEntry, LoopbackNodeRuntimeSpec, NodeDescriptor,
    binary_config_node_runtime_spec, build_binary_config_node_descriptors,
    build_binary_config_node_descriptors_with_file_name, build_loopback_node_descriptors,
};
pub use docker::{
    commands::{ComposeCommandError, compose_down, compose_up, dump_compose_logs},
    config_server::{
        DockerConfigServerHandle, DockerConfigServerSpec, DockerPortBinding, DockerVolumeMount,
        start_docker_config_server,
    },
    platform::host_gateway_entry,
};
pub use env::{
    ComposeBinaryApp, ComposeConfigServerMode, ComposeDeployEnv, ComposeNodeConfigFileName,
    ComposeReadinessProbe, ConfigServerHandle, discovered_node_access,
    write_registration_server_compose_configs,
};
pub use errors::ComposeRunnerError;
pub use infrastructure::{
    ports::{HostPortMapping, NodeHostPorts, compose_runner_host, node_identifier},
    template::{TemplateError, repository_root, write_compose_file},
};

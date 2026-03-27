pub mod deployer;
pub mod descriptor;
pub mod docker;
pub mod env;
pub mod errors;
pub mod infrastructure;
pub mod lifecycle;

pub use deployer::{ComposeDeployer, ComposeDeploymentMetadata};
pub use descriptor::{ComposeDescriptor, EnvEntry, NodeDescriptor};
pub use docker::{
    commands::{ComposeCommandError, compose_down, compose_up, dump_compose_logs},
    config_server::{
        DockerConfigServerHandle, DockerConfigServerSpec, DockerPortBinding, DockerVolumeMount,
        start_docker_config_server,
    },
    platform::host_gateway_entry,
};
pub use env::{ComposeDeployEnv, ConfigServerHandle};
pub use errors::ComposeRunnerError;
pub use infrastructure::{
    ports::{HostPortMapping, NodeHostPorts, compose_runner_host, node_identifier},
    template::{TemplateError, repository_root, write_compose_file},
};

pub mod deployer;
pub mod descriptor;
pub mod docker;
pub mod errors;
pub mod infrastructure;
pub mod lifecycle;

pub use deployer::ComposeDeployer;
pub use descriptor::{ComposeDescriptor, ComposeDescriptorBuilder, EnvEntry, NodeDescriptor};
pub use docker::{
    commands::{ComposeCommandError, compose_down, compose_up, dump_compose_logs},
    platform::{host_gateway_entry, resolve_image},
};
pub use errors::ComposeRunnerError;
pub use infrastructure::{
    ports::{HostPortMapping, NodeHostPorts},
    template::{TemplateError, repository_root, write_compose_file},
};

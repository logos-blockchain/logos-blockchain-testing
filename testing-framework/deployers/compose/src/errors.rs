use std::path::PathBuf;

use testing_framework_core::{
    scenario::{
        MetricsError,
        http_probe::{HttpReadinessError, NodeRole},
    },
    topology::readiness::ReadinessError,
};
use url::ParseError;

use crate::{docker::commands::ComposeCommandError, infrastructure::template::TemplateError};

#[derive(Debug, thiserror::Error)]
/// Top-level compose runner errors.
pub enum ComposeRunnerError {
    #[error("compose runner requires at least one validator (validators={validators})")]
    MissingValidator { validators: usize },
    #[error("docker does not appear to be available on this host")]
    DockerUnavailable,
    #[error("failed to resolve host port for {service} container port {container_port}: {source}")]
    PortDiscovery {
        service: String,
        container_port: u16,
        #[source]
        source: anyhow::Error,
    },
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error(transparent)]
    Config(#[from] ConfigError),
    #[error(transparent)]
    Compose(#[from] ComposeCommandError),
    #[error(transparent)]
    Readiness(#[from] StackReadinessError),
    #[error(transparent)]
    NodeClients(#[from] NodeClientError),
    #[error(transparent)]
    Telemetry(#[from] MetricsError),
    #[error("block feed requires at least one validator client")]
    BlockFeedMissing,
    #[error("failed to start block feed: {source}")]
    BlockFeed {
        #[source]
        source: anyhow::Error,
    },
    #[error(
        "docker image '{image}' is not available; set NOMOS_TESTNET_IMAGE or build the image manually"
    )]
    MissingImage { image: String },
    #[error("failed to prepare docker image: {source}")]
    ImageBuild {
        #[source]
        source: anyhow::Error,
    },
    #[error("internal invariant violated: {message}")]
    InternalInvariant { message: &'static str },
}

#[derive(Debug, thiserror::Error)]
#[error("failed to prepare compose workspace: {source}")]
/// Wraps workspace preparation failures.
pub struct WorkspaceError {
    #[source]
    source: anyhow::Error,
}

impl WorkspaceError {
    pub const fn new(source: anyhow::Error) -> Self {
        Self { source }
    }
}

#[derive(Debug, thiserror::Error)]
/// Configuration-related failures while preparing compose runs.
pub enum ConfigError {
    #[error("failed to update cfgsync configuration at {path}: {source}")]
    Cfgsync {
        path: PathBuf,
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to allocate cfgsync port: {source}")]
    Port {
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to start cfgsync server on port {port}: {source}")]
    CfgsyncStart {
        port: u16,
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to render compose template: {source}")]
    Template {
        #[source]
        source: TemplateError,
    },
}

#[derive(Debug, thiserror::Error)]
/// Readiness probe failures surfaced to callers.
pub enum StackReadinessError {
    #[error(transparent)]
    Http(#[from] HttpReadinessError),
    #[error("failed to build readiness URL for {role} port {port}: {source}", role = role.label())]
    Endpoint {
        role: NodeRole,
        port: u16,
        #[source]
        source: ParseError,
    },
    #[error("remote readiness probe failed: {source}")]
    Remote {
        #[source]
        source: ReadinessError,
    },
}

#[derive(Debug, thiserror::Error)]
/// Node client construction failures.
pub enum NodeClientError {
    #[error(
        "failed to build {endpoint} client URL for {role} port {port}: {source}",
        role = role.label()
    )]
    Endpoint {
        role: NodeRole,
        endpoint: &'static str,
        port: u16,
        #[source]
        source: ParseError,
    },
}

use std::path::PathBuf;

use testing_framework_core::scenario::{DynError, MetricsError};
use url::ParseError;

use crate::{docker::commands::ComposeCommandError, infrastructure::template::TemplateError};

#[derive(Debug, thiserror::Error)]
/// Top-level compose runner errors.
pub enum ComposeRunnerError {
    #[error("compose runner requires at least one node (nodes={nodes})")]
    MissingNode { nodes: usize },
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
    #[error("runtime preflight failed: no node clients available")]
    RuntimePreflight,
    #[error("runtime extension setup failed: {source}")]
    RuntimeExtensions {
        #[source]
        source: DynError,
    },
    #[error("source orchestration failed: {source}")]
    SourceOrchestration {
        #[source]
        source: DynError,
    },
    #[error("docker image '{image}' is not available; build or load it locally")]
    MissingImage { image: String },
    #[error("failed to prepare docker image: {source}")]
    ImageBuild {
        #[source]
        source: anyhow::Error,
    },
    #[error("compose provisioner does not support on-demand start")]
    OnDemandUnsupported,
    #[error(
        "compose session already hosts a managed cluster named '{name}'; pick a distinct name \
         for each cluster"
    )]
    ClusterAlreadyProvisioned { name: String },
    #[error(
        "compose session already hosts an unnamed managed cluster; name clusters via \
         `ClusterRequest::with_name` to deploy several in one session"
    )]
    UnnamedClusterAlreadyProvisioned,
    #[error(
        "invalid compose cluster name '{name}'; use a short lowercase DNS label (letters, \
         digits, and dashes)"
    )]
    InvalidClusterName { name: String },
    #[error("service name '{name}' is already provisioned in the shared compose session")]
    ServiceNameConflict { name: String },
    #[error("internal invariant violated: {message}")]
    InternalInvariant { message: &'static str },
}

impl ComposeRunnerError {
    /// Returns whether the error is terminal for a provisioning attempt, so
    /// retrying with backoff cannot succeed and would only mask the root
    /// cause.
    #[must_use]
    pub const fn is_terminal(&self) -> bool {
        matches!(
            self,
            Self::ClusterAlreadyProvisioned { .. }
                | Self::UnnamedClusterAlreadyProvisioned
                | Self::InvalidClusterName { .. }
                | Self::ServiceNameConflict { .. }
                | Self::OnDemandUnsupported
        )
    }
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
    #[error("failed to build compose descriptor: {source}")]
    Descriptor {
        #[source]
        source: DynError,
    },
    #[error("failed to update cfgsync configuration at {path:?}: {source}")]
    Cfgsync {
        path: PathBuf,
        #[source]
        source: DynError,
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
        source: DynError,
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
    #[error("failed to build readiness URL for {role} port {port}: {source}")]
    Endpoint {
        role: &'static str,
        port: u16,
        #[source]
        source: ParseError,
    },
    #[error("remote readiness probe failed: {source}")]
    Remote {
        #[source]
        source: DynError,
    },
    #[error("expected readiness URLs for {nodes} nodes but none were provided")]
    MissingUrls { nodes: usize },
}

#[derive(Debug, thiserror::Error)]
/// Node client construction failures.
pub enum NodeClientError {
    #[error("failed to build {endpoint} client URL for {role} port {port}: {source}")]
    Endpoint {
        role: &'static str,
        endpoint: &'static str,
        port: u16,
        #[source]
        source: ParseError,
    },
    #[error("failed to build node clients: {source}")]
    Build {
        #[source]
        source: DynError,
    },
}

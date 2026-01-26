use std::{env, sync::LazyLock, time::Duration};

use kube::Error as KubeError;
use thiserror::Error;

mod deployment;
mod forwarding;
mod http_probe;
mod orchestrator;
mod ports;

pub use forwarding::PortForwardHandle;
pub use orchestrator::wait_for_cluster_ready;
use testing_framework_config::constants::{
    DEFAULT_HTTP_POLL_INTERVAL, DEFAULT_K8S_DEPLOYMENT_TIMEOUT, DEFAULT_NODE_HTTP_PROBE_TIMEOUT,
    DEFAULT_NODE_HTTP_TIMEOUT,
};

/// Container and host-side HTTP ports for a node in the Helm chart values.
#[derive(Clone, Copy, Debug)]
pub struct NodeConfigPorts {
    pub api: u16,
    pub testing: u16,
}

/// Host-facing NodePorts for a node.
#[derive(Clone, Copy, Debug)]
pub struct NodePortAllocation {
    pub api: u16,
    pub testing: u16,
}

/// Host/port pair reachable from the machine running the runner.
#[derive(Clone, Debug)]
pub struct HostPort {
    pub host: String,
    pub port: u16,
}

/// All port assignments for the cluster.
#[derive(Debug)]
pub struct ClusterPorts {
    pub nodes: Vec<NodePortAllocation>,
    pub node_host: String,
}

/// Success result from waiting for the cluster: host ports and forward handles.
#[derive(Debug)]
pub struct ClusterReady {
    pub ports: ClusterPorts,
    pub port_forwards: Vec<PortForwardHandle>,
}

#[derive(Debug, Error)]
/// Failures while waiting for Kubernetes deployments or endpoints.
pub enum ClusterWaitError {
    #[error("deployment {name} in namespace {namespace} did not become ready within {timeout:?}")]
    DeploymentTimeout {
        name: String,
        namespace: String,
        timeout: Duration,
    },
    #[error("failed to fetch deployment {name}: {source}")]
    DeploymentFetch {
        name: String,
        #[source]
        source: KubeError,
    },
    #[error("failed to fetch service {service}: {source}")]
    ServiceFetch {
        service: String,
        #[source]
        source: KubeError,
    },
    #[error("service {service} did not allocate a node port for {port}")]
    NodePortUnavailable { service: String, port: u16 },
    #[error("cluster must have at least one node")]
    MissingNode,
    #[error("timeout waiting for {role} HTTP endpoint on port {port} after {timeout:?}")]
    NodeHttpTimeout {
        role: &'static str,
        port: u16,
        timeout: Duration,
    },
    #[error("failed to start port-forward for service {service} port {port}: {source}")]
    PortForward {
        service: String,
        port: u16,
        #[source]
        source: anyhow::Error,
    },
    #[error("port-forward task failed: {source}")]
    PortForwardTask {
        #[source]
        source: anyhow::Error,
    },
}

static DEPLOYMENT_TIMEOUT: LazyLock<Duration> = LazyLock::new(|| {
    env_duration_secs(
        "K8S_RUNNER_DEPLOYMENT_TIMEOUT_SECS",
        DEFAULT_K8S_DEPLOYMENT_TIMEOUT,
    )
});

static NODE_HTTP_TIMEOUT: LazyLock<Duration> =
    LazyLock::new(|| env_duration_secs("K8S_RUNNER_HTTP_TIMEOUT_SECS", DEFAULT_NODE_HTTP_TIMEOUT));

static NODE_HTTP_PROBE_TIMEOUT: LazyLock<Duration> = LazyLock::new(|| {
    env_duration_secs(
        "K8S_RUNNER_HTTP_PROBE_TIMEOUT_SECS",
        DEFAULT_NODE_HTTP_PROBE_TIMEOUT,
    )
});

static HTTP_POLL_INTERVAL: LazyLock<Duration> = LazyLock::new(|| {
    env_duration_secs(
        "K8S_RUNNER_HTTP_POLL_INTERVAL_SECS",
        DEFAULT_HTTP_POLL_INTERVAL,
    )
});

pub(crate) fn deployment_timeout() -> Duration {
    *DEPLOYMENT_TIMEOUT
}

pub(crate) fn node_http_timeout() -> Duration {
    *NODE_HTTP_TIMEOUT
}

pub(crate) fn node_http_probe_timeout() -> Duration {
    *NODE_HTTP_PROBE_TIMEOUT
}

pub(crate) fn http_poll_interval() -> Duration {
    *HTTP_POLL_INTERVAL
}

fn env_duration_secs(key: &str, default: Duration) -> Duration {
    env::var(key)
        .ok()
        .and_then(|raw| raw.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(default)
}

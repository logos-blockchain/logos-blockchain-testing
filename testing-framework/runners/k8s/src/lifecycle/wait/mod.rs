use std::time::Duration;

use kube::Error as KubeError;
use testing_framework_core::{
    constants::{
        DEFAULT_HTTP_POLL_INTERVAL, DEFAULT_K8S_DEPLOYMENT_TIMEOUT,
        DEFAULT_NODE_HTTP_PROBE_TIMEOUT, DEFAULT_NODE_HTTP_TIMEOUT, DEFAULT_PROMETHEUS_HTTP_PORT,
        DEFAULT_PROMETHEUS_HTTP_PROBE_TIMEOUT, DEFAULT_PROMETHEUS_HTTP_TIMEOUT,
        DEFAULT_PROMETHEUS_SERVICE_NAME,
    },
    scenario::http_probe::NodeRole,
};
use thiserror::Error;

mod deployment;
mod forwarding;
mod http_probe;
mod orchestrator;
mod ports;
mod prometheus;

pub use orchestrator::wait_for_cluster_ready;

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

/// All port assignments for the cluster plus Prometheus.
#[derive(Debug)]
pub struct ClusterPorts {
    pub validators: Vec<NodePortAllocation>,
    pub executors: Vec<NodePortAllocation>,
    pub prometheus: u16,
}

/// Success result from waiting for the cluster: host ports and forward handles.
#[derive(Debug)]
pub struct ClusterReady {
    pub ports: ClusterPorts,
    pub port_forwards: Vec<std::process::Child>,
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
    #[error("cluster must have at least one validator")]
    MissingValidator,
    #[error(
        "timeout waiting for {role} HTTP endpoint on port {port} after {timeout:?}",
        role = role.label()
    )]
    NodeHttpTimeout {
        role: NodeRole,
        port: u16,
        timeout: Duration,
    },
    #[error("timeout waiting for prometheus readiness on NodePort {port}")]
    PrometheusTimeout { port: u16 },
    #[error("failed to start port-forward for service {service} port {port}: {source}")]
    PortForward {
        service: String,
        port: u16,
        #[source]
        source: anyhow::Error,
    },
}

pub(crate) const DEPLOYMENT_TIMEOUT: Duration = DEFAULT_K8S_DEPLOYMENT_TIMEOUT;
pub(crate) const NODE_HTTP_TIMEOUT: Duration = DEFAULT_NODE_HTTP_TIMEOUT;
pub(crate) const NODE_HTTP_PROBE_TIMEOUT: Duration = DEFAULT_NODE_HTTP_PROBE_TIMEOUT;
pub(crate) const HTTP_POLL_INTERVAL: Duration = DEFAULT_HTTP_POLL_INTERVAL;
pub(crate) const PROMETHEUS_HTTP_PORT: u16 = DEFAULT_PROMETHEUS_HTTP_PORT;
pub(crate) const PROMETHEUS_HTTP_TIMEOUT: Duration = DEFAULT_PROMETHEUS_HTTP_TIMEOUT;
pub(crate) const PROMETHEUS_HTTP_PROBE_TIMEOUT: Duration = DEFAULT_PROMETHEUS_HTTP_PROBE_TIMEOUT;
pub(crate) const PROMETHEUS_SERVICE_NAME: &str = DEFAULT_PROMETHEUS_SERVICE_NAME;

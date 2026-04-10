use std::time::Duration;

use testing_framework_core::scenario::HttpReadinessRequirement;

use super::{ClusterWaitError, http_poll_interval, node_http_probe_timeout, node_http_timeout};
use crate::{
    env::{K8sDeployEnv, wait_for_node_http},
    host::node_host,
};

const LOCALHOST: &str = "127.0.0.1";
const READINESS_REQUIREMENT: HttpReadinessRequirement = HttpReadinessRequirement::AllNodesReady;

pub async fn wait_for_node_http_nodeport<E: K8sDeployEnv>(
    ports: &[u16],
    role: &'static str,
) -> Result<(), ClusterWaitError> {
    let host = node_host();
    wait_for_node_http_on_host::<E>(ports, role, &host, node_http_probe_timeout()).await
}

pub async fn wait_for_node_http_port_forward<E: K8sDeployEnv>(
    ports: &[u16],
    role: &'static str,
) -> Result<(), ClusterWaitError> {
    wait_for_node_http_on_host::<E>(ports, role, LOCALHOST, node_http_timeout()).await
}

async fn wait_for_node_http_on_host<E: K8sDeployEnv>(
    ports: &[u16],
    role: &'static str,
    host: &str,
    timeout: Duration,
) -> Result<(), ClusterWaitError> {
    wait_for_node_http::<E>(
        ports,
        role,
        host,
        timeout,
        http_poll_interval(),
        READINESS_REQUIREMENT,
    )
    .await
    .map_err(|source| ClusterWaitError::NodeHttp { role, source })
}

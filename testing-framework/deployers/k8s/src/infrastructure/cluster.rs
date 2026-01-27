use std::env;

use kube::Client;
use reqwest::Url;
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{CleanupGuard, NodeClients, http_probe::NODE_ROLE},
    topology::{generation::GeneratedTopology, readiness::ReadinessError},
};
use tracing::{debug, info};
use url::ParseError;
use uuid::Uuid;

use crate::{
    infrastructure::assets::RunnerAssets,
    lifecycle::{cleanup::RunnerCleanup, logs::dump_namespace_logs},
    wait::{
        ClusterPorts, ClusterReady, NodeConfigPorts, PortForwardHandle, wait_for_cluster_ready,
    },
};

#[derive(Default)]
pub struct PortSpecs {
    pub nodes: Vec<NodeConfigPorts>,
}

/// Holds k8s namespace, Helm release, port forwards, and cleanup guard.
pub struct ClusterEnvironment {
    client: Client,
    namespace: String,
    release: String,
    cleanup: Option<RunnerCleanup>,
    node_host: String,
    node_api_ports: Vec<u16>,
    node_testing_ports: Vec<u16>,
    port_forwards: Vec<PortForwardHandle>,
}

#[derive(Debug, thiserror::Error)]
pub enum ClusterEnvironmentError {
    #[error("cleanup guard is missing (it may have already been consumed)")]
    MissingCleanupGuard,
}

impl ClusterEnvironment {
    pub fn new(
        client: Client,
        namespace: String,
        release: String,
        cleanup: RunnerCleanup,
        ports: &ClusterPorts,
        port_forwards: Vec<PortForwardHandle>,
    ) -> Self {
        let node_api_ports = ports.nodes.iter().map(|ports| ports.api).collect();
        let node_testing_ports = ports.nodes.iter().map(|ports| ports.testing).collect();

        Self {
            client,
            namespace,
            release,
            cleanup: Some(cleanup),
            node_host: ports.node_host.clone(),
            node_api_ports,
            node_testing_ports,
            port_forwards,
        }
    }

    pub async fn fail(&mut self, reason: &str) {
        tracing::error!(
            reason = reason,
            namespace = %self.namespace,
            release = %self.release,
            "k8s stack failure; collecting diagnostics"
        );
        dump_namespace_logs(&self.client, &self.namespace).await;
        kill_port_forwards(&mut self.port_forwards);
        if let Some(guard) = self.cleanup.take() {
            CleanupGuard::cleanup(Box::new(guard));
        }
    }

    pub fn into_cleanup(
        self,
    ) -> Result<(RunnerCleanup, Vec<PortForwardHandle>), ClusterEnvironmentError> {
        let cleanup = self
            .cleanup
            .ok_or(ClusterEnvironmentError::MissingCleanupGuard)?;
        Ok((cleanup, self.port_forwards))
    }

    #[allow(dead_code)]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    #[allow(dead_code)]
    pub fn release(&self) -> &str {
        &self.release
    }

    pub fn node_ports(&self) -> (&[u16], &[u16]) {
        (&self.node_api_ports, &self.node_testing_ports)
    }
}

#[derive(Debug, thiserror::Error)]
/// Failures while building node clients against forwarded ports.
pub enum NodeClientError {
    #[error("failed to build {endpoint} client URL for {role} port {port}: {source}")]
    Endpoint {
        role: &'static str,
        endpoint: &'static str,
        port: u16,
        #[source]
        source: ParseError,
    },
}

#[derive(Debug, thiserror::Error)]
/// Readiness check failures for the remote cluster endpoints.
pub enum RemoteReadinessError {
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
        source: ReadinessError,
    },
}

pub fn collect_port_specs(descriptors: &GeneratedTopology) -> PortSpecs {
    let nodes = descriptors
        .nodes()
        .iter()
        .map(|node| NodeConfigPorts {
            api: node.general.api_config.address.port(),
            testing: node.general.api_config.testing_http_address.port(),
        })
        .collect();

    let specs = PortSpecs { nodes };

    debug!(nodes = specs.nodes.len(), "collected k8s port specs");

    specs
}

pub fn build_node_clients(cluster: &ClusterEnvironment) -> Result<NodeClients, NodeClientError> {
    let nodes = cluster
        .node_api_ports
        .iter()
        .copied()
        .zip(cluster.node_testing_ports.iter().copied())
        .map(|(api_port, testing_port)| {
            api_client_from_ports(&cluster.node_host, NODE_ROLE, api_port, testing_port)
        })
        .collect::<Result<Vec<_>, _>>()?;

    debug!(nodes = nodes.len(), "built k8s node clients");

    Ok(NodeClients::new(nodes))
}

pub async fn ensure_cluster_readiness(
    descriptors: &GeneratedTopology,
    cluster: &ClusterEnvironment,
) -> Result<(), RemoteReadinessError> {
    info!("waiting for remote readiness (API + membership)");
    let (node_api, _node_testing) = cluster.node_ports();

    let node_urls = readiness_urls(node_api, NODE_ROLE, &cluster.node_host)?;

    descriptors
        .wait_remote_readiness(&node_urls)
        .await
        .map_err(|source| RemoteReadinessError::Remote { source })?;

    info!(
        node_api_ports = ?node_api,
        "k8s remote readiness confirmed"
    );

    Ok(())
}

pub fn cluster_identifiers() -> (String, String) {
    if let Ok(namespace) = env::var("K8S_RUNNER_NAMESPACE")
        && !namespace.is_empty()
    {
        let release = env::var("K8S_RUNNER_RELEASE")
            .ok()
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| namespace.clone());
        return (namespace, release);
    }

    let run_id = Uuid::new_v4().simple().to_string();
    let namespace = format!("nomos-k8s-{run_id}");
    (namespace.clone(), namespace)
}

pub async fn install_stack(
    client: &Client,
    assets: &RunnerAssets,
    namespace: &str,
    release: &str,
    nodes: usize,
) -> Result<RunnerCleanup, crate::deployer::K8sRunnerError> {
    tracing::info!(
        release = %release,
        namespace = %namespace,
        "installing helm release"
    );
    crate::infrastructure::helm::install_release(assets, release, namespace, nodes).await?;
    tracing::info!(release = %release, "helm install succeeded");

    let preserve = env::var("K8S_RUNNER_PRESERVE").is_ok();
    Ok(RunnerCleanup::new(
        client.clone(),
        namespace.to_owned(),
        release.to_owned(),
        preserve,
    ))
}

pub async fn wait_for_ports_or_cleanup(
    client: &Client,
    namespace: &str,
    release: &str,
    specs: &PortSpecs,
    cleanup_guard: &mut Option<RunnerCleanup>,
) -> Result<ClusterReady, crate::deployer::K8sRunnerError> {
    info!(
        nodes = specs.nodes.len(),
        %namespace,
        %release,
        "waiting for cluster port-forwards"
    );
    match wait_for_cluster_ready(client, namespace, release, &specs.nodes).await {
        Ok(ports) => {
            info!(
                node_ports = ?ports.ports.nodes,
                "cluster port-forwards established"
            );
            Ok(ports)
        }
        Err(err) => {
            cleanup_pending(client, namespace, cleanup_guard).await;
            Err(err.into())
        }
    }
}

pub fn kill_port_forwards(handles: &mut Vec<PortForwardHandle>) {
    for handle in handles.iter_mut() {
        handle.shutdown();
    }
    handles.clear();
}

async fn cleanup_pending(client: &Client, namespace: &str, guard: &mut Option<RunnerCleanup>) {
    crate::lifecycle::logs::dump_namespace_logs(client, namespace).await;
    if let Some(guard) = guard.take() {
        CleanupGuard::cleanup(Box::new(guard));
    }
}

fn readiness_urls(
    ports: &[u16],
    role: &'static str,
    host: &str,
) -> Result<Vec<Url>, RemoteReadinessError> {
    ports
        .iter()
        .copied()
        .map(|port| readiness_url(host, role, port))
        .collect()
}

fn readiness_url(host: &str, role: &'static str, port: u16) -> Result<Url, RemoteReadinessError> {
    cluster_host_url(host, port).map_err(|source| RemoteReadinessError::Endpoint {
        role,
        port,
        source,
    })
}

fn cluster_host_url(host: &str, port: u16) -> Result<Url, ParseError> {
    Url::parse(&format!("http://{host}:{port}/"))
}

fn api_client_from_ports(
    host: &str,
    role: &'static str,
    api_port: u16,
    testing_port: u16,
) -> Result<ApiClient, NodeClientError> {
    let base_endpoint =
        cluster_host_url(host, api_port).map_err(|source| NodeClientError::Endpoint {
            role,
            endpoint: "api",
            port: api_port,
            source,
        })?;
    let testing_endpoint =
        Some(
            cluster_host_url(host, testing_port).map_err(|source| NodeClientError::Endpoint {
                role,
                endpoint: "testing",
                port: testing_port,
                source,
            })?,
        );
    Ok(ApiClient::from_urls(base_endpoint, testing_endpoint))
}

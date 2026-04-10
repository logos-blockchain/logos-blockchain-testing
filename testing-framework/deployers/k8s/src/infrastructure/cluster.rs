use kube::Client;
use reqwest::Url;
use testing_framework_core::scenario::{
    DynError, HttpReadinessRequirement, NodeClients, internal::CleanupGuard,
};
use tracing::{debug, info};
use url::ParseError;

use crate::{
    env::{
        K8sDeployEnv, build_node_clients as build_k8s_node_clients,
        collect_port_specs as collect_k8s_port_specs, node_role, wait_remote_readiness,
    },
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
    node_auxiliary_ports: Vec<u16>,
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
        let node_auxiliary_ports = ports.nodes.iter().map(|ports| ports.auxiliary).collect();

        Self {
            client,
            namespace,
            release,
            cleanup: Some(cleanup),
            node_host: ports.node_host.clone(),
            node_api_ports,
            node_auxiliary_ports,
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

    pub fn client(&self) -> &Client {
        &self.client
    }

    pub fn node_ports(&self) -> (&[u16], &[u16]) {
        (&self.node_api_ports, &self.node_auxiliary_ports)
    }
}

#[derive(Debug, thiserror::Error)]
/// Failures while building node clients against forwarded ports.
pub enum NodeClientError {
    #[error("failed to build node clients: {source}")]
    Build {
        #[source]
        source: DynError,
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
        source: DynError,
    },
}

pub fn collect_port_specs<E: K8sDeployEnv>(descriptors: &E::Deployment) -> PortSpecs {
    let specs = collect_k8s_port_specs::<E>(descriptors);
    debug!(nodes = specs.nodes.len(), "collected k8s port specs");
    specs
}

pub fn build_node_clients<E: K8sDeployEnv>(
    cluster: &ClusterEnvironment,
) -> Result<NodeClients<E>, NodeClientError> {
    let nodes = build_k8s_node_clients::<E>(
        &cluster.node_host,
        &cluster.node_api_ports,
        &cluster.node_auxiliary_ports,
    )
    .map_err(|source| NodeClientError::Build { source })?;

    debug!(nodes = nodes.len(), "built k8s node clients");

    Ok(NodeClients::new(nodes))
}

pub async fn ensure_cluster_readiness<E: K8sDeployEnv>(
    descriptors: &E::Deployment,
    cluster: &ClusterEnvironment,
    requirement: HttpReadinessRequirement,
) -> Result<(), RemoteReadinessError> {
    info!("waiting for remote readiness (API + membership)");
    let (node_api, _node_auxiliary) = cluster.node_ports();

    let node_urls = readiness_urls(node_api, node_role::<E>(), &cluster.node_host)?;

    wait_remote_readiness::<E>(descriptors, &node_urls, requirement)
        .await
        .map_err(|source| RemoteReadinessError::Remote { source })?;

    info!(
        node_api_ports = ?node_api,
        "k8s remote readiness confirmed"
    );

    Ok(())
}

pub async fn wait_for_ports_or_cleanup<E: K8sDeployEnv>(
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
    match wait_for_cluster_ready::<E>(client, namespace, release, &specs.nodes).await {
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

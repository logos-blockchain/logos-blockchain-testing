use std::env;

use kube::Client;
use reqwest::Url;
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{CleanupGuard, Metrics, MetricsError, NodeClients, http_probe::NodeRole},
    topology::{generation::GeneratedTopology, readiness::ReadinessError},
};
use tracing::{debug, info};
use url::ParseError;
use uuid::Uuid;

use crate::{
    host::node_host,
    infrastructure::assets::RunnerAssets,
    lifecycle::{cleanup::RunnerCleanup, logs::dump_namespace_logs},
    wait::{
        ClusterPorts, ClusterReady, NodeConfigPorts, PortForwardHandle, wait_for_cluster_ready,
    },
};

#[derive(Default)]
pub struct PortSpecs {
    pub validators: Vec<NodeConfigPorts>,
    pub executors: Vec<NodeConfigPorts>,
}

/// Holds k8s namespace, Helm release, port forwards, and cleanup guard.
pub struct ClusterEnvironment {
    client: Client,
    namespace: String,
    release: String,
    cleanup: Option<RunnerCleanup>,
    validator_api_ports: Vec<u16>,
    validator_testing_ports: Vec<u16>,
    executor_api_ports: Vec<u16>,
    executor_testing_ports: Vec<u16>,
    prometheus_port: u16,
    port_forwards: Vec<PortForwardHandle>,
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
        let validator_api_ports = ports.validators.iter().map(|ports| ports.api).collect();
        let validator_testing_ports = ports.validators.iter().map(|ports| ports.testing).collect();
        let executor_api_ports = ports.executors.iter().map(|ports| ports.api).collect();
        let executor_testing_ports = ports.executors.iter().map(|ports| ports.testing).collect();

        Self {
            client,
            namespace,
            release,
            cleanup: Some(cleanup),
            validator_api_ports,
            validator_testing_ports,
            executor_api_ports,
            executor_testing_ports,
            prometheus_port: ports.prometheus,
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

    pub fn into_cleanup(self) -> (RunnerCleanup, Vec<PortForwardHandle>) {
        (
            self.cleanup.expect("cleanup guard should be available"),
            self.port_forwards,
        )
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn release(&self) -> &str {
        &self.release
    }

    pub fn prometheus_port(&self) -> u16 {
        self.prometheus_port
    }

    pub fn validator_ports(&self) -> (&[u16], &[u16]) {
        (&self.validator_api_ports, &self.validator_testing_ports)
    }

    pub fn executor_ports(&self) -> (&[u16], &[u16]) {
        (&self.executor_api_ports, &self.executor_testing_ports)
    }
}

#[derive(Debug, thiserror::Error)]
/// Failures while building node clients against forwarded ports.
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

#[derive(Debug, thiserror::Error)]
/// Readiness check failures for the remote cluster endpoints.
pub enum RemoteReadinessError {
    #[error(
        "failed to build readiness URL for {role} port {port}: {source}",
        role = role.label()
    )]
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

pub fn collect_port_specs(descriptors: &GeneratedTopology) -> PortSpecs {
    let validators = descriptors
        .validators()
        .iter()
        .map(|node| NodeConfigPorts {
            api: node.general.api_config.address.port(),
            testing: node.general.api_config.testing_http_address.port(),
        })
        .collect();
    let executors = descriptors
        .executors()
        .iter()
        .map(|node| NodeConfigPorts {
            api: node.general.api_config.address.port(),
            testing: node.general.api_config.testing_http_address.port(),
        })
        .collect();

    let specs = PortSpecs {
        validators,
        executors,
    };

    debug!(
        validators = specs.validators.len(),
        executors = specs.executors.len(),
        "collected k8s port specs"
    );

    specs
}

pub fn build_node_clients(cluster: &ClusterEnvironment) -> Result<NodeClients, NodeClientError> {
    let validators = cluster
        .validator_api_ports
        .iter()
        .copied()
        .zip(cluster.validator_testing_ports.iter().copied())
        .map(|(api_port, testing_port)| {
            api_client_from_ports(NodeRole::Validator, api_port, testing_port)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let executors = cluster
        .executor_api_ports
        .iter()
        .copied()
        .zip(cluster.executor_testing_ports.iter().copied())
        .map(|(api_port, testing_port)| {
            api_client_from_ports(NodeRole::Executor, api_port, testing_port)
        })
        .collect::<Result<Vec<_>, _>>()?;

    debug!(
        validators = validators.len(),
        executors = executors.len(),
        "built k8s node clients"
    );

    Ok(NodeClients::new(validators, executors))
}

pub fn metrics_handle_from_port(port: u16) -> Result<Metrics, MetricsError> {
    let url = cluster_host_url(port)
        .map_err(|err| MetricsError::new(format!("invalid prometheus url: {err}")))?;
    Metrics::from_prometheus(url)
}

pub async fn ensure_cluster_readiness(
    descriptors: &GeneratedTopology,
    cluster: &ClusterEnvironment,
) -> Result<(), RemoteReadinessError> {
    info!("waiting for remote readiness (API + membership)");
    let (validator_api, validator_testing) = cluster.validator_ports();
    let (executor_api, executor_testing) = cluster.executor_ports();

    let validator_urls = readiness_urls(validator_api, NodeRole::Validator)?;
    let executor_urls = readiness_urls(executor_api, NodeRole::Executor)?;
    let validator_membership_urls = readiness_urls(validator_testing, NodeRole::Validator)?;
    let executor_membership_urls = readiness_urls(executor_testing, NodeRole::Executor)?;

    descriptors
        .wait_remote_readiness(
            &validator_urls,
            &executor_urls,
            Some(&validator_membership_urls),
            Some(&executor_membership_urls),
        )
        .await
        .map_err(|source| RemoteReadinessError::Remote { source })?;

    info!(
        validator_api_ports = ?validator_api,
        executor_api_ports = ?executor_api,
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
    validators: usize,
    executors: usize,
) -> Result<RunnerCleanup, crate::deployer::K8sRunnerError> {
    tracing::info!(
        release = %release,
        namespace = %namespace,
        "installing helm release"
    );
    crate::infrastructure::helm::install_release(assets, release, namespace, validators, executors)
        .await?;
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
        validators = specs.validators.len(),
        executors = specs.executors.len(),
        %namespace,
        %release,
        "waiting for cluster port-forwards"
    );
    match wait_for_cluster_ready(
        client,
        namespace,
        release,
        &specs.validators,
        &specs.executors,
    )
    .await
    {
        Ok(ports) => {
            info!(
                prometheus_port = ports.ports.prometheus,
                validator_ports = ?ports.ports.validators,
                executor_ports = ?ports.ports.executors,
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

fn readiness_urls(ports: &[u16], role: NodeRole) -> Result<Vec<Url>, RemoteReadinessError> {
    ports
        .iter()
        .copied()
        .map(|port| readiness_url(role, port))
        .collect()
}

fn readiness_url(role: NodeRole, port: u16) -> Result<Url, RemoteReadinessError> {
    cluster_host_url(port).map_err(|source| RemoteReadinessError::Endpoint { role, port, source })
}

fn cluster_host_url(port: u16) -> Result<Url, ParseError> {
    Url::parse(&format!("http://{}:{port}/", node_host()))
}

fn api_client_from_ports(
    role: NodeRole,
    api_port: u16,
    testing_port: u16,
) -> Result<ApiClient, NodeClientError> {
    let base_endpoint = cluster_host_url(api_port).map_err(|source| NodeClientError::Endpoint {
        role,
        endpoint: "api",
        port: api_port,
        source,
    })?;
    let testing_endpoint =
        Some(
            cluster_host_url(testing_port).map_err(|source| NodeClientError::Endpoint {
                role,
                endpoint: "testing",
                port: testing_port,
                source,
            })?,
        );
    Ok(ApiClient::from_urls(base_endpoint, testing_endpoint))
}

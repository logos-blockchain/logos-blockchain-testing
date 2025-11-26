use std::{
    net::{Ipv4Addr, TcpListener, TcpStream},
    process::{Command as StdCommand, Stdio},
    thread,
    time::Duration,
};

use k8s_openapi::api::{apps::v1::Deployment, core::v1::Service};
use kube::{Api, Client, Error as KubeError};
use testing_framework_core::scenario::http_probe::{self, HttpReadinessError, NodeRole};
use thiserror::Error;
use tokio::time::sleep;

use crate::host::node_host;

const DEPLOYMENT_TIMEOUT: Duration = Duration::from_secs(180);
const NODE_HTTP_TIMEOUT: Duration = Duration::from_secs(240);
const NODE_HTTP_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const HTTP_POLL_INTERVAL: Duration = Duration::from_secs(1);
const PROMETHEUS_HTTP_PORT: u16 = 9090;
const PROMETHEUS_HTTP_TIMEOUT: Duration = Duration::from_secs(240);
const PROMETHEUS_HTTP_PROBE_TIMEOUT: Duration = Duration::from_secs(30);
const PROMETHEUS_SERVICE_NAME: &str = "prometheus";

#[derive(Clone, Copy)]
pub struct NodeConfigPorts {
    pub api: u16,
    pub testing: u16,
}

#[derive(Clone, Copy)]
pub struct NodePortAllocation {
    pub api: u16,
    pub testing: u16,
}

pub struct ClusterPorts {
    pub validators: Vec<NodePortAllocation>,
    pub executors: Vec<NodePortAllocation>,
    pub prometheus: u16,
}

pub struct ClusterReady {
    pub ports: ClusterPorts,
    pub port_forwards: Vec<std::process::Child>,
}

#[derive(Debug, Error)]
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
    #[error("timeout waiting for {role} HTTP endpoint on port {port} after {timeout:?}")]
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

pub async fn wait_for_deployment_ready(
    client: &Client,
    namespace: &str,
    name: &str,
    timeout: Duration,
) -> Result<(), ClusterWaitError> {
    let mut elapsed = Duration::ZERO;
    let interval = Duration::from_secs(2);

    while elapsed <= timeout {
        match Api::<Deployment>::namespaced(client.clone(), namespace)
            .get(name)
            .await
        {
            Ok(deployment) => {
                let desired = deployment
                    .spec
                    .as_ref()
                    .and_then(|spec| spec.replicas)
                    .unwrap_or(1);
                let ready = deployment
                    .status
                    .as_ref()
                    .and_then(|status| status.ready_replicas)
                    .unwrap_or(0);
                if ready >= desired {
                    return Ok(());
                }
            }
            Err(err) => {
                return Err(ClusterWaitError::DeploymentFetch {
                    name: name.to_owned(),
                    source: err,
                });
            }
        }

        sleep(interval).await;
        elapsed += interval;
    }

    Err(ClusterWaitError::DeploymentTimeout {
        name: name.to_owned(),
        namespace: namespace.to_owned(),
        timeout,
    })
}

pub async fn find_node_port(
    client: &Client,
    namespace: &str,
    service_name: &str,
    service_port: u16,
) -> Result<u16, ClusterWaitError> {
    let interval = Duration::from_secs(1);
    for _ in 0..120 {
        match Api::<Service>::namespaced(client.clone(), namespace)
            .get(service_name)
            .await
        {
            Ok(service) => {
                if let Some(spec) = service.spec.clone()
                    && let Some(ports) = spec.ports
                {
                    for port in ports {
                        if port.port == i32::from(service_port)
                            && let Some(node_port) = port.node_port
                        {
                            return Ok(node_port as u16);
                        }
                    }
                }
            }
            Err(err) => {
                return Err(ClusterWaitError::ServiceFetch {
                    service: service_name.to_owned(),
                    source: err,
                });
            }
        }
        sleep(interval).await;
    }

    Err(ClusterWaitError::NodePortUnavailable {
        service: service_name.to_owned(),
        port: service_port,
    })
}

pub async fn wait_for_cluster_ready(
    client: &Client,
    namespace: &str,
    release: &str,
    validator_ports: &[NodeConfigPorts],
    executor_ports: &[NodeConfigPorts],
) -> Result<ClusterReady, ClusterWaitError> {
    if validator_ports.is_empty() {
        return Err(ClusterWaitError::MissingValidator);
    }

    let mut validator_allocations = Vec::with_capacity(validator_ports.len());

    for (index, ports) in validator_ports.iter().enumerate() {
        let name = format!("{release}-validator-{index}");
        wait_for_deployment_ready(client, namespace, &name, DEPLOYMENT_TIMEOUT).await?;
        let api_port = find_node_port(client, namespace, &name, ports.api).await?;
        let testing_port = find_node_port(client, namespace, &name, ports.testing).await?;
        validator_allocations.push(NodePortAllocation {
            api: api_port,
            testing: testing_port,
        });
    }

    let mut port_forwards = Vec::new();

    let validator_api_ports: Vec<u16> = validator_allocations
        .iter()
        .map(|ports| ports.api)
        .collect();
    if wait_for_node_http_nodeport(
        &validator_api_ports,
        NodeRole::Validator,
        NODE_HTTP_PROBE_TIMEOUT,
    )
    .await
    .is_err()
    {
        // Fall back to port-forwarding when NodePorts are unreachable from the host.
        validator_allocations.clear();
        port_forwards = port_forward_group(
            namespace,
            release,
            "validator",
            validator_ports,
            &mut validator_allocations,
        )?;
        let validator_api_ports: Vec<u16> = validator_allocations
            .iter()
            .map(|ports| ports.api)
            .collect();
        if let Err(err) =
            wait_for_node_http_port_forward(&validator_api_ports, NodeRole::Validator).await
        {
            kill_port_forwards(&mut port_forwards);
            return Err(err);
        }
    }

    let mut executor_allocations = Vec::with_capacity(executor_ports.len());
    for (index, ports) in executor_ports.iter().enumerate() {
        let name = format!("{release}-executor-{index}");
        wait_for_deployment_ready(client, namespace, &name, DEPLOYMENT_TIMEOUT).await?;
        let api_port = find_node_port(client, namespace, &name, ports.api).await?;
        let testing_port = find_node_port(client, namespace, &name, ports.testing).await?;
        executor_allocations.push(NodePortAllocation {
            api: api_port,
            testing: testing_port,
        });
    }

    let executor_api_ports: Vec<u16> = executor_allocations.iter().map(|ports| ports.api).collect();
    if !executor_allocations.is_empty()
        && wait_for_node_http_nodeport(
            &executor_api_ports,
            NodeRole::Executor,
            NODE_HTTP_PROBE_TIMEOUT,
        )
        .await
        .is_err()
    {
        executor_allocations.clear();
        match port_forward_group(
            namespace,
            release,
            "executor",
            executor_ports,
            &mut executor_allocations,
        ) {
            Ok(forwards) => port_forwards.extend(forwards),
            Err(err) => {
                kill_port_forwards(&mut port_forwards);
                return Err(err);
            }
        }
        let executor_api_ports: Vec<u16> =
            executor_allocations.iter().map(|ports| ports.api).collect();
        if let Err(err) =
            wait_for_node_http_port_forward(&executor_api_ports, NodeRole::Executor).await
        {
            kill_port_forwards(&mut port_forwards);
            return Err(err);
        }
    }

    let mut prometheus_port = find_node_port(
        client,
        namespace,
        PROMETHEUS_SERVICE_NAME,
        PROMETHEUS_HTTP_PORT,
    )
    .await?;
    if wait_for_prometheus_http_nodeport(prometheus_port, PROMETHEUS_HTTP_PROBE_TIMEOUT)
        .await
        .is_err()
    {
        let (local_port, forward) =
            port_forward_service(namespace, PROMETHEUS_SERVICE_NAME, PROMETHEUS_HTTP_PORT)
                .map_err(|err| {
                    kill_port_forwards(&mut port_forwards);
                    err
                })?;
        prometheus_port = local_port;
        port_forwards.push(forward);
        if let Err(err) =
            wait_for_prometheus_http_port_forward(prometheus_port, PROMETHEUS_HTTP_TIMEOUT).await
        {
            kill_port_forwards(&mut port_forwards);
            return Err(err);
        }
    }

    Ok(ClusterReady {
        ports: ClusterPorts {
            validators: validator_allocations,
            executors: executor_allocations,
            prometheus: prometheus_port,
        },
        port_forwards,
    })
}

async fn wait_for_node_http_nodeport(
    ports: &[u16],
    role: NodeRole,
    timeout: Duration,
) -> Result<(), ClusterWaitError> {
    let host = node_host();
    wait_for_node_http_on_host(ports, role, &host, timeout).await
}

async fn wait_for_node_http_port_forward(
    ports: &[u16],
    role: NodeRole,
) -> Result<(), ClusterWaitError> {
    wait_for_node_http_on_host(ports, role, "127.0.0.1", NODE_HTTP_TIMEOUT).await
}

async fn wait_for_node_http_on_host(
    ports: &[u16],
    role: NodeRole,
    host: &str,
    timeout: Duration,
) -> Result<(), ClusterWaitError> {
    http_probe::wait_for_http_ports_with_host(ports, role, host, timeout, HTTP_POLL_INTERVAL)
        .await
        .map_err(map_http_error)
}

const fn map_http_error(error: HttpReadinessError) -> ClusterWaitError {
    ClusterWaitError::NodeHttpTimeout {
        role: error.role(),
        port: error.port(),
        timeout: error.timeout(),
    }
}

pub async fn wait_for_prometheus_http_nodeport(
    port: u16,
    timeout: Duration,
) -> Result<(), ClusterWaitError> {
    let host = node_host();
    wait_for_prometheus_http(&host, port, timeout).await
}

pub async fn wait_for_prometheus_http_port_forward(
    port: u16,
    timeout: Duration,
) -> Result<(), ClusterWaitError> {
    wait_for_prometheus_http("127.0.0.1", port, timeout).await
}

pub async fn wait_for_prometheus_http(
    host: &str,
    port: u16,
    timeout: Duration,
) -> Result<(), ClusterWaitError> {
    let client = reqwest::Client::new();
    let url = format!("http://{host}:{port}/-/ready");

    for _ in 0..timeout.as_secs() {
        if let Ok(resp) = client.get(&url).send().await
            && resp.status().is_success()
        {
            return Ok(());
        }
        sleep(Duration::from_secs(1)).await;
    }

    Err(ClusterWaitError::PrometheusTimeout { port })
}

fn port_forward_group(
    namespace: &str,
    release: &str,
    kind: &str,
    ports: &[NodeConfigPorts],
    allocations: &mut Vec<NodePortAllocation>,
) -> Result<Vec<std::process::Child>, ClusterWaitError> {
    let mut forwards = Vec::new();
    for (index, ports) in ports.iter().enumerate() {
        let service = format!("{release}-{kind}-{index}");
        let (api_port, api_forward) = match port_forward_service(namespace, &service, ports.api) {
            Ok(forward) => forward,
            Err(err) => {
                kill_port_forwards(&mut forwards);
                return Err(err);
            }
        };
        let (testing_port, testing_forward) =
            match port_forward_service(namespace, &service, ports.testing) {
                Ok(forward) => forward,
                Err(err) => {
                    kill_port_forwards(&mut forwards);
                    return Err(err);
                }
            };
        allocations.push(NodePortAllocation {
            api: api_port,
            testing: testing_port,
        });
        forwards.push(api_forward);
        forwards.push(testing_forward);
    }
    Ok(forwards)
}

fn port_forward_service(
    namespace: &str,
    service: &str,
    remote_port: u16,
) -> Result<(u16, std::process::Child), ClusterWaitError> {
    let local_port = allocate_local_port().map_err(|source| ClusterWaitError::PortForward {
        service: service.to_owned(),
        port: remote_port,
        source,
    })?;

    let mut child = StdCommand::new("kubectl")
        .arg("port-forward")
        .arg("-n")
        .arg(namespace)
        .arg(format!("svc/{service}"))
        .arg(format!("{local_port}:{remote_port}"))
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|source| ClusterWaitError::PortForward {
            service: service.to_owned(),
            port: remote_port,
            source: source.into(),
        })?;

    for _ in 0..20 {
        if let Ok(Some(status)) = child.try_wait() {
            return Err(ClusterWaitError::PortForward {
                service: service.to_owned(),
                port: remote_port,
                source: anyhow::anyhow!("kubectl exited with {status}"),
            });
        }
        if TcpStream::connect((Ipv4Addr::LOCALHOST, local_port)).is_ok() {
            return Ok((local_port, child));
        }
        thread::sleep(Duration::from_millis(250));
    }

    let _ = child.kill();
    Err(ClusterWaitError::PortForward {
        service: service.to_owned(),
        port: remote_port,
        source: anyhow::anyhow!("port-forward did not become ready"),
    })
}

fn allocate_local_port() -> anyhow::Result<u16> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

fn kill_port_forwards(handles: &mut Vec<std::process::Child>) {
    for handle in handles.iter_mut() {
        let _ = handle.kill();
        let _ = handle.wait();
    }
    handles.clear();
}

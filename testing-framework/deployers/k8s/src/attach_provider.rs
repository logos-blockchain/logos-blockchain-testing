use std::{marker::PhantomData, time::Duration};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Service;
use kube::{
    Api, Client,
    api::{ListParams, ObjectList},
};
use testing_framework_core::scenario::{
    CleanupGuard, ClusterWaitHandle, DynError, ExistingCluster, ExternalNodeSource,
    HttpReadinessRequirement, wait_for_http_ports_with_host_and_requirement, wait_http_readiness,
};
use tokio::net::TcpStream;
use url::Url;

use crate::{
    env::{K8sDeployEnv, node_readiness_path},
    host::node_host,
    lifecycle::wait::{PortForwardHandle, PortForwardSpawn, port_forward_service},
};

const LOCALHOST: &str = "127.0.0.1";
const DIRECT_PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, thiserror::Error)]
enum K8sAttachDiscoveryError {
    #[error("k8s attach source requires a non-empty label selector")]
    EmptyLabelSelector,
    #[error("existing cluster descriptor is not supported by this provider: {attach_source:?}")]
    UnsupportedSource { attach_source: ExistingCluster },
    #[error("no services matched label selector '{selector}' in namespace '{namespace}'")]
    NoMatchingServices { namespace: String, selector: String },
    #[error("k8s service has no metadata.name")]
    MissingServiceName,
    #[error("service '{service}' has no TCP node ports exposed")]
    ServiceHasNoNodePorts { service: String },
    #[error(
        "service '{service}' has multiple candidate API node ports ({ports}); explicit API port required"
    )]
    ServiceHasMultipleNodePorts { service: String, ports: String },
    #[error(
        "attached k8s cluster is unreachable: direct endpoint {host}:{port} did not accept a TCP \
         connection within {timeout:?}, and the kubectl port-forward fallback failed: {source}"
    )]
    EndpointsUnreachable {
        host: String,
        port: u16,
        timeout: Duration,
        #[source]
        source: DynError,
    },
}

/// How discovered node services are reached from the runner.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AttachAccessMode {
    /// Discovered endpoints answer directly; build clients against them.
    Direct,
    /// Discovered endpoints are unreachable; reach the services through
    /// `kubectl port-forward`.
    PortForward,
}

/// Decides how to reach an attached cluster from the direct-endpoint probe
/// result: a reachable endpoint keeps the direct-client behavior, an
/// unreachable one selects the port-forward fallback.
const fn attach_access_mode(direct_endpoint_reachable: bool) -> AttachAccessMode {
    if direct_endpoint_reachable {
        AttachAccessMode::Direct
    } else {
        AttachAccessMode::PortForward
    }
}

/// Resolved node access for an attached cluster.
pub(crate) enum AttachedAccess {
    /// Node services are reached directly on their discovered `NodePorts`.
    Direct,
    /// Node services are reached through local `kubectl port-forward`
    /// listeners on these API ports.
    Forwarded { api_ports: Vec<u16> },
}

/// Clients and access details discovered for one attached cluster.
///
/// When the fallback spawned port-forwards, `forwards` owns the kubectl
/// processes; the provisioner must attach it to the cluster unit so the
/// forwards live exactly as long as the unit.
pub(crate) struct AttachedDiscovery<E: K8sDeployEnv> {
    pub(crate) clients: Vec<E::NodeClient>,
    pub(crate) access: AttachedAccess,
    pub(crate) forwards: Option<Box<dyn CleanupGuard>>,
}

/// Owns the `kubectl port-forward` processes spawned for an attached cluster.
///
/// Both explicit cleanup and a plain drop kill the processes, so the forwards
/// die with the cluster unit that owns this guard.
struct AttachedPortForwards {
    handles: Vec<PortForwardHandle>,
}

impl CleanupGuard for AttachedPortForwards {
    fn cleanup(mut self: Box<Self>) {
        for handle in &mut self.handles {
            handle.shutdown();
        }
    }
}

/// Service and node port resolved for one node service's API endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ApiServicePort {
    pub(super) service_port: u16,
    pub(super) node_port: u16,
}

/// One discovered node service together with its resolved API ports.
struct ServiceEndpoint {
    name: String,
    ports: ApiServicePort,
}

pub(crate) struct K8sAttachProvider<E: K8sDeployEnv> {
    client: Client,
    _env: PhantomData<E>,
}

pub(crate) struct K8sAttachedClusterWait<E: K8sDeployEnv> {
    client: Client,
    source: ExistingCluster,
    access: AttachedAccess,
    _env: PhantomData<E>,
}

struct K8sAttachRequest<'a> {
    namespace: &'a str,
    label_selector: &'a str,
}

impl<E: K8sDeployEnv> K8sAttachProvider<E> {
    pub(crate) fn new(client: Client) -> Self {
        Self {
            client,
            _env: PhantomData,
        }
    }
}

impl<E: K8sDeployEnv> K8sAttachedClusterWait<E> {
    pub(crate) fn try_new(
        client: Client,
        source: &ExistingCluster,
        access: AttachedAccess,
    ) -> Result<Self, DynError> {
        let _ = k8s_wait_request(source)?;

        Ok(Self {
            client,
            source: source.clone(),
            access,
            _env: PhantomData,
        })
    }

    async fn wait_direct_network_ready(&self) -> Result<(), DynError> {
        let request = k8s_wait_request(&self.source)?;
        let services =
            discover_services(&self.client, request.namespace, request.label_selector).await?;
        let host = node_host();
        let endpoints = collect_readiness_endpoints::<E>(&host, &services.items)?;

        wait_http_readiness(&endpoints, HttpReadinessRequirement::AllNodesReady).await?;

        Ok(())
    }
}

impl<E: K8sDeployEnv> K8sAttachProvider<E> {
    /// Discovers node clients for the requested existing k8s cluster.
    ///
    /// Probes one discovered endpoint first: when it is reachable the clients
    /// are built directly against the discovered `NodePorts`, otherwise the
    /// discovery falls back to `kubectl port-forward` and builds the clients
    /// against the forwarded local ports.
    pub(crate) async fn discover(
        &self,
        source: &ExistingCluster,
    ) -> Result<AttachedDiscovery<E>, DynError> {
        let request = k8s_attach_request(source)?;
        let namespace = request.namespace.to_owned();
        let services =
            discover_services(&self.client, request.namespace, request.label_selector).await?;
        let endpoints = collect_service_endpoints(&services.items)?;
        let Some(first) = endpoints.first() else {
            return Err(K8sAttachDiscoveryError::NoMatchingServices {
                namespace,
                selector: request.label_selector.to_owned(),
            }
            .into());
        };

        let host = node_host();
        let probe_port = first.ports.node_port;
        let reachable = direct_endpoint_reachable(&host, probe_port, DIRECT_PROBE_TIMEOUT).await;

        match attach_access_mode(reachable) {
            AttachAccessMode::Direct => direct_discovery::<E>(&host, &endpoints),
            AttachAccessMode::PortForward => {
                forwarded_discovery::<E>(&namespace, &host, probe_port, endpoints).await
            }
        }
    }
}

/// Builds clients straight against the discovered `NodePorts` (the historical
/// attached-cluster behavior).
fn direct_discovery<E: K8sDeployEnv>(
    host: &str,
    endpoints: &[ServiceEndpoint],
) -> Result<AttachedDiscovery<E>, DynError> {
    let mut clients = Vec::with_capacity(endpoints.len());

    for endpoint in endpoints {
        clients.push(attached_node_client::<E>(
            host,
            endpoint.ports.node_port,
            &endpoint.name,
        )?);
    }

    Ok(AttachedDiscovery {
        clients,
        access: AttachedAccess::Direct,
        forwards: None,
    })
}

/// Spawns one `kubectl port-forward` per discovered node service and builds
/// the clients against the forwarded local ports.
///
/// A forwarding failure is reported together with the failed direct probe so
/// the attach error names both attempts.
async fn forwarded_discovery<E: K8sDeployEnv>(
    namespace: &str,
    probed_host: &str,
    probed_node_port: u16,
    endpoints: Vec<ServiceEndpoint>,
) -> Result<AttachedDiscovery<E>, DynError> {
    let spawned = spawn_attach_forwards(namespace.to_owned(), endpoints)
        .await
        .map_err(|source| K8sAttachDiscoveryError::EndpointsUnreachable {
            host: probed_host.to_owned(),
            port: probed_node_port,
            timeout: DIRECT_PROBE_TIMEOUT,
            source,
        })?;

    let mut clients = Vec::with_capacity(spawned.len());
    let mut api_ports = Vec::with_capacity(spawned.len());
    let mut handles = Vec::with_capacity(spawned.len());

    for (service_name, forward) in spawned {
        clients.push(attached_node_client::<E>(
            LOCALHOST,
            forward.local_port,
            &service_name,
        )?);
        api_ports.push(forward.local_port);
        handles.push(forward.handle);
    }

    Ok(AttachedDiscovery {
        clients,
        access: AttachedAccess::Forwarded { api_ports },
        forwards: Some(Box::new(AttachedPortForwards { handles })),
    })
}

/// Spawns the per-service forwards on a blocking thread, reusing the managed
/// path's `port_forward_service` machinery.
async fn spawn_attach_forwards(
    namespace: String,
    endpoints: Vec<ServiceEndpoint>,
) -> Result<Vec<(String, PortForwardSpawn)>, DynError> {
    tokio::task::spawn_blocking(move || {
        let mut spawned = Vec::with_capacity(endpoints.len());
        for endpoint in endpoints {
            let forward =
                port_forward_service(&namespace, &endpoint.name, endpoint.ports.service_port)?;
            spawned.push((endpoint.name, forward));
        }
        Ok::<_, DynError>(spawned)
    })
    .await
    .map_err(|source| DynError::from(format!("port-forward task failed: {source}")))?
}

fn attached_node_client<E: K8sDeployEnv>(
    host: &str,
    port: u16,
    service_name: &str,
) -> Result<E::NodeClient, DynError> {
    let endpoint = format!("http://{host}:{port}/");
    let source = ExternalNodeSource::new(service_name.to_owned(), endpoint);

    E::external_node_client(&source)
}

/// Reports whether one direct endpoint accepts a TCP connection within the
/// timeout.
async fn direct_endpoint_reachable(host: &str, port: u16, timeout: Duration) -> bool {
    matches!(
        tokio::time::timeout(timeout, TcpStream::connect((host, port))).await,
        Ok(Ok(_))
    )
}

fn collect_service_endpoints(services: &[Service]) -> Result<Vec<ServiceEndpoint>, DynError> {
    let mut endpoints = Vec::with_capacity(services.len());

    for service in services {
        let name = service
            .metadata
            .name
            .clone()
            .ok_or(K8sAttachDiscoveryError::MissingServiceName)?;
        let ports = extract_api_port(service)?;
        endpoints.push(ServiceEndpoint { name, ports });
    }

    Ok(endpoints)
}

fn k8s_attach_request(source: &ExistingCluster) -> Result<K8sAttachRequest<'_>, DynError> {
    let Some(label_selector) = source.k8s_label_selector() else {
        return Err(K8sAttachDiscoveryError::UnsupportedSource {
            attach_source: source.clone(),
        }
        .into());
    };

    if label_selector.trim().is_empty() {
        return Err(K8sAttachDiscoveryError::EmptyLabelSelector.into());
    }

    Ok(K8sAttachRequest {
        namespace: source.k8s_namespace().unwrap_or("default"),
        label_selector,
    })
}

pub(super) async fn discover_services(
    client: &Client,
    namespace: &str,
    selector: &str,
) -> Result<ObjectList<Service>, DynError> {
    let services: Api<Service> = Api::namespaced(client.clone(), namespace);
    let params = ListParams::default().labels(selector);
    let services = services.list(&params).await?;
    let services = filter_services_with_tcp_node_ports(services);

    if services.items.is_empty() {
        return Err(K8sAttachDiscoveryError::NoMatchingServices {
            namespace: namespace.to_owned(),
            selector: selector.to_owned(),
        }
        .into());
    }

    Ok(services)
}

fn filter_services_with_tcp_node_ports(services: ObjectList<Service>) -> ObjectList<Service> {
    ObjectList {
        items: services
            .items
            .into_iter()
            .filter(|service| !tcp_api_port_pairs(service).is_empty())
            .collect(),
        metadata: services.metadata,
        types: services.types,
    }
}

fn tcp_api_port_pairs(service: &Service) -> Vec<(String, ApiServicePort)> {
    service
        .spec
        .as_ref()
        .into_iter()
        .flat_map(|spec| spec.ports.as_ref())
        .flat_map(|ports| ports.iter())
        .filter_map(|port| {
            let node_port = port.node_port.and_then(|value| u16::try_from(value).ok())?;
            let service_port = u16::try_from(port.port).ok()?;
            let protocol = port.protocol.as_deref().unwrap_or("TCP");
            if protocol != "TCP" {
                return None;
            }

            Some((
                port.name.clone().unwrap_or_default(),
                ApiServicePort {
                    service_port,
                    node_port,
                },
            ))
        })
        .collect()
}

pub(super) fn extract_api_port(service: &Service) -> Result<ApiServicePort, DynError> {
    let service_name = service
        .metadata
        .name
        .clone()
        .unwrap_or_else(|| "<unknown>".to_owned());
    let ports = api_port_candidates(tcp_api_port_pairs(service));

    match ports.as_slice() {
        [] => Err(K8sAttachDiscoveryError::ServiceHasNoNodePorts {
            service: service_name,
        }
        .into()),
        [port] => Ok(*port),
        _ => Err(K8sAttachDiscoveryError::ServiceHasMultipleNodePorts {
            service: service_name,
            ports: ports
                .iter()
                .map(|port| port.node_port.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        }
        .into()),
    }
}

pub(super) fn extract_api_node_port(service: &Service) -> Result<u16, DynError> {
    extract_api_port(service).map(|ports| ports.node_port)
}

fn api_port_candidates(ports: Vec<(String, ApiServicePort)>) -> Vec<ApiServicePort> {
    let explicit_api: Vec<ApiServicePort> = ports
        .iter()
        .filter_map(|(name, port)| (name == "http" || name == "api").then_some(*port))
        .collect();
    if !explicit_api.is_empty() {
        return explicit_api;
    }

    ports.into_iter().map(|(_, port)| port).collect()
}

#[async_trait]
impl<E: K8sDeployEnv> ClusterWaitHandle<E> for K8sAttachedClusterWait<E> {
    async fn wait_network_ready(&self) -> Result<(), DynError> {
        match &self.access {
            AttachedAccess::Direct => self.wait_direct_network_ready().await,
            AttachedAccess::Forwarded { api_ports } => {
                wait_for_http_ports_with_host_and_requirement(
                    api_ports,
                    LOCALHOST,
                    node_readiness_path::<E>(),
                    HttpReadinessRequirement::AllNodesReady,
                )
                .await
                .map_err(Into::into)
            }
        }
    }
}

fn k8s_wait_request(source: &ExistingCluster) -> Result<K8sAttachRequest<'_>, DynError> {
    let label_selector = source.k8s_label_selector().ok_or_else(|| {
        DynError::from("k8s cluster wait requires a k8s existing-cluster descriptor")
    })?;

    if label_selector.trim().is_empty() {
        return Err(K8sAttachDiscoveryError::EmptyLabelSelector.into());
    }

    Ok(K8sAttachRequest {
        namespace: source.k8s_namespace().unwrap_or("default"),
        label_selector,
    })
}

fn collect_readiness_endpoints<E: K8sDeployEnv>(
    host: &str,
    services: &[Service],
) -> Result<Vec<Url>, DynError> {
    let mut endpoints = Vec::with_capacity(services.len());

    for service in services {
        let api_port = extract_api_node_port(service)?;
        let mut endpoint = Url::parse(&format!("http://{host}:{api_port}/"))?;
        endpoint.set_path(node_readiness_path::<E>());
        endpoints.push(endpoint);
    }

    Ok(endpoints)
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, time::Duration};

    use k8s_openapi::api::core::v1::{Service, ServicePort, ServiceSpec};

    use super::{
        AttachAccessMode, attach_access_mode, direct_endpoint_reachable, extract_api_node_port,
        extract_api_port,
    };

    #[test]
    fn extract_api_node_port_returns_single_port() {
        let service = Service {
            metadata: Default::default(),
            spec: Some(ServiceSpec {
                ports: Some(vec![ServicePort {
                    port: 8080,
                    node_port: Some(31234),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let port = extract_api_node_port(&service).expect("single port should resolve");
        assert_eq!(port, 31234);
    }

    #[test]
    fn extract_api_node_port_prefers_http_name() {
        let service = Service {
            metadata: Default::default(),
            spec: Some(ServiceSpec {
                ports: Some(vec![
                    ServicePort {
                        name: Some("testing-http".to_owned()),
                        port: 8081,
                        node_port: Some(31234),
                        ..Default::default()
                    },
                    ServicePort {
                        name: Some("http".to_owned()),
                        port: 8080,
                        node_port: Some(31235),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let port = extract_api_node_port(&service).expect("http-named port should resolve");
        assert_eq!(port, 31235);
    }

    #[test]
    fn extract_api_port_pairs_service_and_node_ports() {
        let service = Service {
            metadata: Default::default(),
            spec: Some(ServiceSpec {
                ports: Some(vec![ServicePort {
                    name: Some("api".to_owned()),
                    port: 8080,
                    node_port: Some(31234),
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let ports = extract_api_port(&service).expect("api port should resolve");
        assert_eq!(ports.service_port, 8080);
        assert_eq!(ports.node_port, 31234);
    }

    #[test]
    fn reachable_endpoint_keeps_direct_access() {
        assert_eq!(attach_access_mode(true), AttachAccessMode::Direct);
    }

    #[test]
    fn unreachable_endpoint_selects_port_forward_fallback() {
        assert_eq!(attach_access_mode(false), AttachAccessMode::PortForward);
    }

    #[tokio::test]
    async fn direct_probe_reflects_listener_state() {
        let listener =
            std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).expect("bind probe listener");
        let port = listener.local_addr().expect("listener address").port();

        assert!(direct_endpoint_reachable("127.0.0.1", port, Duration::from_secs(1)).await);

        drop(listener);

        assert!(!direct_endpoint_reachable("127.0.0.1", port, Duration::from_secs(1)).await);
    }
}

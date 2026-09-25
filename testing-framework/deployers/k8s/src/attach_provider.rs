use std::{
    marker::PhantomData,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Service;
use kube::{
    Api, Client,
    api::{ListParams, ObjectList},
};
use testing_framework_core::scenario::{
    CleanupGuard, ClusterWaitHandle, DEFAULT_READINESS_POLL_INTERVAL, DEFAULT_READINESS_TIMEOUT,
    DynError, ExistingCluster, ReadinessRequirement, wait_for_readiness_ports, wait_readiness,
};
use tokio::net::TcpStream;
use url::Url;

use crate::{
    env::K8sDeployEnv,
    host::node_host,
    lifecycle::wait::{
        ForwardSpec, PortForwardHandle, PortForwardSpawn, port_forward_service, respawn_forward,
    },
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
    #[error("service '{service}' has no TCP ports exposed")]
    ServiceHasNoTcpPorts { service: String },
    #[error("service '{service}' exposes no node port for direct access")]
    ServiceHasNoNodePort { service: String },
    #[error(
        "service '{service}' has multiple candidate API ports ({ports}); explicit API port required"
    )]
    ServiceHasMultipleApiPorts { service: String, ports: String },
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
    #[error(
        "attached k8s cluster is unreachable: the matched services expose no node ports for a \
         direct connection, and the kubectl port-forward fallback failed: {source}"
    )]
    ClusterIpForwardFailed {
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
#[derive(Clone)]
pub(crate) enum AttachedAccess {
    /// Node services are reached directly on their discovered `NodePorts`.
    Direct,
    /// Node services are reached through local `kubectl port-forward`
    /// listeners tracked by this registry.
    Forwarded { forwards: AttachedForwardRegistry },
}

/// Clients and access details discovered for one attached cluster.
///
/// When the fallback spawned port-forwards, `forwards` owns the kubectl
/// processes; the provisioner must attach it to the cluster unit so the
/// forwards live exactly as long as the unit.
pub(crate) struct AttachedDiscovery<E: K8sDeployEnv> {
    pub(crate) namespace: String,
    pub(crate) clients: Vec<(String, E::NodeClient)>,
    pub(crate) access: AttachedAccess,
    pub(crate) forwards: Option<Box<dyn CleanupGuard>>,
}

/// One attached service's live forward together with the spec to recreate it.
struct AttachedForwardEntry {
    service: String,
    spec: ForwardSpec,
    handle: PortForwardHandle,
}

/// Shared registry of the `kubectl port-forward` processes spawned for an
/// attached cluster, keyed by service name.
///
/// A restarted node's pod kills the forward bound to it, so the attached node
/// control respawns that service's forward on its original local port. Both
/// explicit cleanup and dropping the last clone kill the processes, so the
/// forwards die with the cluster unit that owns the cleanup guard.
#[derive(Clone, Default)]
pub(crate) struct AttachedForwardRegistry {
    entries: Arc<Mutex<Vec<AttachedForwardEntry>>>,
}

impl AttachedForwardRegistry {
    fn register(&self, service: String, spec: ForwardSpec, handle: PortForwardHandle) {
        self.lock().push(AttachedForwardEntry {
            service,
            spec,
            handle,
        });
    }

    /// Returns the local API ports of every registered forward.
    pub(crate) fn local_api_ports(&self) -> Vec<u16> {
        self.lock()
            .iter()
            .map(|entry| entry.spec.local_port)
            .collect()
    }

    /// Returns the local API port forwarded for the given service.
    pub(crate) fn local_port(&self, service: &str) -> Option<u16> {
        self.lock()
            .iter()
            .find(|entry| entry.service == service)
            .map(|entry| entry.spec.local_port)
    }

    /// Respawns the given service's forward on its original local port.
    ///
    /// Blocking: spawns a `kubectl` process and polls the local port for
    /// readiness.
    pub(crate) fn respawn(&self, service: &str) -> Result<(), DynError> {
        let mut entries = self.lock();
        let entry = entries
            .iter_mut()
            .find(|entry| entry.service == service)
            .ok_or_else(|| {
                DynError::from(format!(
                    "no port-forward is registered for service '{service}'"
                ))
            })?;
        entry.handle.shutdown();
        entry.handle = respawn_forward(&entry.spec)?;
        Ok(())
    }

    fn shutdown_all(&self) {
        let entries = {
            let mut entries = self.lock();
            std::mem::take(&mut *entries)
        };
        for mut entry in entries {
            entry.handle.shutdown();
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<AttachedForwardEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl CleanupGuard for AttachedForwardRegistry {
    fn cleanup(self: Box<Self>) {
        self.shutdown_all();
    }
}

/// Service port resolved for one node service's API endpoint, together with
/// the node port when the service exposes one.
///
/// A `ClusterIP`-style service has no node port: it cannot be probed or
/// reached directly and is served through `kubectl port-forward` instead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ApiServicePort {
    pub(super) service_port: u16,
    pub(super) node_port: Option<u16>,
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
        let endpoints = collect_readiness_endpoints(&host, &services.items)?;

        wait_readiness(
            &endpoints,
            E::node_readiness_probe(),
            ReadinessRequirement::AllNodesReady,
            DEFAULT_READINESS_TIMEOUT,
            DEFAULT_READINESS_POLL_INTERVAL,
        )
        .await?;

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
        if endpoints.is_empty() {
            return Err(K8sAttachDiscoveryError::NoMatchingServices {
                namespace,
                selector: request.label_selector.to_owned(),
            }
            .into());
        }

        let host = node_host();
        let Some(probe_port) = direct_probe_port(&endpoints) else {
            return forwarded_discovery::<E>(namespace, None, endpoints).await;
        };
        let reachable = direct_endpoint_reachable(&host, probe_port, DIRECT_PROBE_TIMEOUT).await;

        match attach_access_mode(reachable) {
            AttachAccessMode::Direct => direct_discovery::<E>(namespace, &host, &endpoints),
            AttachAccessMode::PortForward => {
                forwarded_discovery::<E>(namespace, Some((host, probe_port)), endpoints).await
            }
        }
    }
}

/// Returns the node port to probe for direct access when every discovered
/// service exposes one.
///
/// Any service without a node port cannot be reached directly, so the whole
/// attachment skips the direct probe and goes straight to port-forwarding.
fn direct_probe_port(endpoints: &[ServiceEndpoint]) -> Option<u16> {
    endpoints
        .iter()
        .map(|endpoint| endpoint.ports.node_port)
        .collect::<Option<Vec<_>>>()?
        .first()
        .copied()
}

/// Builds clients straight against the discovered `NodePorts` (the historical
/// attached-cluster behavior).
fn direct_discovery<E: K8sDeployEnv>(
    namespace: String,
    host: &str,
    endpoints: &[ServiceEndpoint],
) -> Result<AttachedDiscovery<E>, DynError> {
    let mut clients = Vec::with_capacity(endpoints.len());

    for endpoint in endpoints {
        let node_port = endpoint.ports.node_port.ok_or_else(|| {
            K8sAttachDiscoveryError::ServiceHasNoNodePort {
                service: endpoint.name.clone(),
            }
        })?;
        clients.push((
            endpoint.name.clone(),
            attached_node_client::<E>(host, node_port)?,
        ));
    }

    Ok(AttachedDiscovery {
        namespace,
        clients,
        access: AttachedAccess::Direct,
        forwards: None,
    })
}

/// Spawns one `kubectl port-forward` per discovered node service and builds
/// the clients against the forwarded local ports.
///
/// When the fallback follows a failed direct probe (`probe` names the probed
/// endpoint), a forwarding failure is reported together with that probe so
/// the attach error names both attempts; without node ports there was no
/// probe to report.
async fn forwarded_discovery<E: K8sDeployEnv>(
    namespace: String,
    probe: Option<(String, u16)>,
    endpoints: Vec<ServiceEndpoint>,
) -> Result<AttachedDiscovery<E>, DynError> {
    let spawned = spawn_attach_forwards(namespace.clone(), endpoints)
        .await
        .map_err(|source| match probe {
            Some((host, port)) => K8sAttachDiscoveryError::EndpointsUnreachable {
                host,
                port,
                timeout: DIRECT_PROBE_TIMEOUT,
                source,
            },
            None => K8sAttachDiscoveryError::ClusterIpForwardFailed { source },
        })?;

    let mut clients = Vec::with_capacity(spawned.len());
    let forwards = AttachedForwardRegistry::default();

    for (endpoint, forward) in spawned {
        clients.push((
            endpoint.name.clone(),
            attached_node_client::<E>(LOCALHOST, forward.local_port)?,
        ));
        forwards.register(
            endpoint.name.clone(),
            ForwardSpec {
                namespace: namespace.clone(),
                service: endpoint.name,
                local_port: forward.local_port,
                remote_port: endpoint.ports.service_port,
            },
            forward.handle,
        );
    }

    Ok(AttachedDiscovery {
        namespace,
        clients,
        access: AttachedAccess::Forwarded {
            forwards: forwards.clone(),
        },
        forwards: Some(Box::new(forwards)),
    })
}

/// Spawns the per-service forwards on a blocking thread, reusing the managed
/// path's `port_forward_service` machinery.
async fn spawn_attach_forwards(
    namespace: String,
    endpoints: Vec<ServiceEndpoint>,
) -> Result<Vec<(ServiceEndpoint, PortForwardSpawn)>, DynError> {
    tokio::task::spawn_blocking(move || {
        let mut spawned = Vec::with_capacity(endpoints.len());
        for endpoint in endpoints {
            let forward =
                port_forward_service(&namespace, &endpoint.name, endpoint.ports.service_port)?;
            spawned.push((endpoint, forward));
        }
        Ok::<_, DynError>(spawned)
    })
    .await
    .map_err(|source| DynError::from(format!("port-forward task failed: {source}")))?
}

/// Builds one attached node client the same way the managed path builds its
/// clients: through `node_client_from_ports` over discovered node access.
///
/// Attach discovery resolves a single API port per service, so that port
/// serves as both the API and the auxiliary port, mirroring the managed
/// path's port pairing when only one port is available.
fn attached_node_client<E: K8sDeployEnv>(host: &str, port: u16) -> Result<E::NodeClient, DynError> {
    E::node_client_from_ports(host, port, port)
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
    let services = filter_services_with_tcp_ports(services);

    if services.items.is_empty() {
        return Err(K8sAttachDiscoveryError::NoMatchingServices {
            namespace: namespace.to_owned(),
            selector: selector.to_owned(),
        }
        .into());
    }

    Ok(services)
}

fn filter_services_with_tcp_ports(services: ObjectList<Service>) -> ObjectList<Service> {
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
            let node_port = port.node_port.and_then(|value| u16::try_from(value).ok());
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
        [] => Err(K8sAttachDiscoveryError::ServiceHasNoTcpPorts {
            service: service_name,
        }
        .into()),
        [port] => Ok(*port),
        _ => Err(K8sAttachDiscoveryError::ServiceHasMultipleApiPorts {
            service: service_name,
            ports: ports
                .iter()
                .map(|port| port.service_port.to_string())
                .collect::<Vec<_>>()
                .join(", "),
        }
        .into()),
    }
}

/// Resolves the node port used for direct access; a service without a node
/// port cannot be reached directly and is rejected here.
pub(super) fn extract_api_node_port(service: &Service) -> Result<u16, DynError> {
    let ports = extract_api_port(service)?;
    ports.node_port.map_or_else(
        || {
            Err(K8sAttachDiscoveryError::ServiceHasNoNodePort {
                service: service
                    .metadata
                    .name
                    .clone()
                    .unwrap_or_else(|| "<unknown>".to_owned()),
            }
            .into())
        },
        Ok,
    )
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
            AttachedAccess::Forwarded { forwards } => wait_for_readiness_ports(
                &forwards.local_api_ports(),
                LOCALHOST,
                E::node_readiness_probe(),
                ReadinessRequirement::AllNodesReady,
                DEFAULT_READINESS_TIMEOUT,
                DEFAULT_READINESS_POLL_INTERVAL,
            )
            .await
            .map_err(Into::into),
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

fn collect_readiness_endpoints(host: &str, services: &[Service]) -> Result<Vec<Url>, DynError> {
    let mut endpoints = Vec::with_capacity(services.len());

    for service in services {
        let api_port = extract_api_node_port(service)?;
        let endpoint = Url::parse(&format!("http://{host}:{api_port}/"))?;
        endpoints.push(endpoint);
    }

    Ok(endpoints)
}

#[cfg(test)]
mod tests {
    use std::{net::Ipv4Addr, time::Duration};

    use k8s_openapi::api::core::v1::{Service, ServicePort, ServiceSpec};

    use super::{
        AttachAccessMode, ServiceEndpoint, attach_access_mode, direct_endpoint_reachable,
        direct_probe_port, extract_api_node_port, extract_api_port,
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
        assert_eq!(ports.node_port, Some(31234));
    }

    #[test]
    fn cluster_ip_service_resolves_to_forward_path() {
        let service = Service {
            metadata: Default::default(),
            spec: Some(ServiceSpec {
                ports: Some(vec![ServicePort {
                    name: Some("api".to_owned()),
                    port: 8080,
                    node_port: None,
                    ..Default::default()
                }]),
                ..Default::default()
            }),
            ..Default::default()
        };

        let ports = extract_api_port(&service).expect("a ClusterIP service must resolve");
        assert_eq!(ports.service_port, 8080);
        assert_eq!(ports.node_port, None);

        let endpoints = vec![ServiceEndpoint {
            name: "kv-node-0".to_owned(),
            ports,
        }];
        assert_eq!(
            direct_probe_port(&endpoints),
            None,
            "a service without a node port must skip the direct probe and go straight to \
             port-forwarding"
        );

        let error = extract_api_node_port(&service)
            .expect_err("direct access must be rejected without a node port");
        assert!(error.to_string().contains("no node port"), "got: {error}");
    }

    #[test]
    fn node_port_services_keep_the_direct_probe() {
        let endpoints = vec![
            ServiceEndpoint {
                name: "kv-node-0".to_owned(),
                ports: super::ApiServicePort {
                    service_port: 8080,
                    node_port: Some(31234),
                },
            },
            ServiceEndpoint {
                name: "kv-node-1".to_owned(),
                ports: super::ApiServicePort {
                    service_port: 8080,
                    node_port: Some(31235),
                },
            },
        ];

        assert_eq!(direct_probe_port(&endpoints), Some(31234));
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

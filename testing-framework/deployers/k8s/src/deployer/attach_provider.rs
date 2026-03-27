use std::marker::PhantomData;

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Service;
use kube::{
    Api, Client,
    api::{ListParams, ObjectList},
};
use testing_framework_core::scenario::{
    ClusterWaitHandle, DynError, ExistingCluster, ExternalNodeSource, HttpReadinessRequirement,
    internal::{AttachProvider, AttachProviderError, AttachedNode},
    wait_http_readiness,
};
use url::Url;

use crate::{env::K8sDeployEnv, host::node_host};

#[derive(Debug, thiserror::Error)]
enum K8sAttachDiscoveryError {
    #[error("k8s attach source requires a non-empty label selector")]
    EmptyLabelSelector,
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
}

pub(super) struct K8sAttachProvider<E: K8sDeployEnv> {
    client: Client,
    _env: PhantomData<E>,
}

pub(super) struct K8sAttachedClusterWait<E: K8sDeployEnv> {
    client: Client,
    source: ExistingCluster,
    _env: PhantomData<E>,
}

struct K8sAttachRequest<'a> {
    namespace: &'a str,
    label_selector: &'a str,
}

impl<E: K8sDeployEnv> K8sAttachProvider<E> {
    pub(super) fn new(client: Client) -> Self {
        Self {
            client,
            _env: PhantomData,
        }
    }
}

impl<E: K8sDeployEnv> K8sAttachedClusterWait<E> {
    pub(super) fn try_new(client: Client, source: &ExistingCluster) -> Result<Self, DynError> {
        let _ = k8s_wait_request(source)?;

        Ok(Self {
            client,
            source: source.clone(),
            _env: PhantomData,
        })
    }
}

#[async_trait]
impl<E: K8sDeployEnv> AttachProvider<E> for K8sAttachProvider<E> {
    async fn discover(
        &self,
        source: &ExistingCluster,
    ) -> Result<Vec<AttachedNode<E>>, AttachProviderError> {
        let request = k8s_attach_request(source)?;
        let services = discover_services(&self.client, request.namespace, request.label_selector)
            .await
            .map_err(to_discovery_error)?;
        let host = node_host();
        let mut attached = Vec::with_capacity(services.items.len());

        for service in services.items {
            attached.push(build_attached_node::<E>(&host, service).map_err(to_discovery_error)?);
        }

        Ok(attached)
    }
}

fn to_discovery_error(source: DynError) -> AttachProviderError {
    AttachProviderError::Discovery { source }
}

fn k8s_attach_request(
    source: &ExistingCluster,
) -> Result<K8sAttachRequest<'_>, AttachProviderError> {
    let Some(label_selector) = source.k8s_label_selector() else {
        return Err(AttachProviderError::UnsupportedSource {
            attach_source: source.clone(),
        });
    };

    if label_selector.trim().is_empty() {
        return Err(AttachProviderError::Discovery {
            source: K8sAttachDiscoveryError::EmptyLabelSelector.into(),
        });
    }

    Ok(K8sAttachRequest {
        namespace: source.k8s_namespace().unwrap_or("default"),
        label_selector,
    })
}

fn build_attached_node<E: K8sDeployEnv>(
    host: &str,
    service: Service,
) -> Result<AttachedNode<E>, DynError> {
    let service_name = service
        .metadata
        .name
        .clone()
        .ok_or(K8sAttachDiscoveryError::MissingServiceName)?;

    let api_port = extract_api_node_port(&service)?;
    let endpoint = format!("http://{host}:{api_port}/");
    let source = ExternalNodeSource::new(service_name.clone(), endpoint);
    let client = E::external_node_client(&source)?;

    Ok(AttachedNode {
        identity_hint: Some(service_name),
        client,
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
            .filter(|service| !tcp_node_ports(service).is_empty())
            .collect(),
        metadata: services.metadata,
    }
}

fn tcp_node_ports(service: &Service) -> Vec<(String, u16)> {
    service
        .spec
        .as_ref()
        .into_iter()
        .flat_map(|spec| spec.ports.as_ref())
        .flat_map(|ports| ports.iter())
        .filter_map(|port| {
            let node_port = port.node_port.and_then(|value| u16::try_from(value).ok())?;
            let protocol = port.protocol.as_deref().unwrap_or("TCP");
            if protocol != "TCP" {
                return None;
            }

            Some((port.name.clone().unwrap_or_default(), node_port))
        })
        .collect()
}

pub(super) fn extract_api_node_port(service: &Service) -> Result<u16, DynError> {
    let service_name = service
        .metadata
        .name
        .clone()
        .unwrap_or_else(|| "<unknown>".to_owned());
    let ports = api_port_candidates(tcp_node_ports(service));

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
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", "),
        }
        .into()),
    }
}

fn api_port_candidates(ports: Vec<(String, u16)>) -> Vec<u16> {
    let explicit_api: Vec<u16> = ports
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
        let request = k8s_wait_request(&self.source)?;
        let services =
            discover_services(&self.client, request.namespace, request.label_selector).await?;
        let host = node_host();
        let endpoints = collect_readiness_endpoints::<E>(&host, &services.items)?;

        wait_http_readiness(&endpoints, HttpReadinessRequirement::AllNodesReady).await?;

        Ok(())
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
        endpoint.set_path(E::readiness_path());
        endpoints.push(endpoint);
    }

    Ok(endpoints)
}

#[cfg(test)]
mod tests {
    use k8s_openapi::api::core::v1::{Service, ServicePort, ServiceSpec};

    use super::extract_api_node_port;

    #[test]
    fn extract_api_node_port_returns_single_port() {
        let service = Service {
            metadata: Default::default(),
            spec: Some(ServiceSpec {
                ports: Some(vec![ServicePort {
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
                        node_port: Some(31234),
                        ..Default::default()
                    },
                    ServicePort {
                        name: Some("http".to_owned()),
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
}

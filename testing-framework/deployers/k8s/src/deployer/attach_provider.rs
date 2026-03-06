use std::marker::PhantomData;

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Service;
use kube::{
    Api, Client,
    api::{ListParams, ObjectList},
};
use testing_framework_core::scenario::{
    AttachProvider, AttachProviderError, AttachSource, AttachedNode, DynError, ExternalNodeSource,
};

use crate::{env::K8sDeployEnv, host::node_host};

#[derive(Debug, thiserror::Error)]
enum K8sAttachDiscoveryError {
    #[error("k8s attach source requires a non-empty label selector")]
    EmptyLabelSelector,
    #[error("no services matched label selector '{selector}' in namespace '{namespace}'")]
    NoMatchingServices { namespace: String, selector: String },
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

impl<E: K8sDeployEnv> K8sAttachProvider<E> {
    pub(super) fn new(client: Client) -> Self {
        Self {
            client,
            _env: PhantomData,
        }
    }
}

#[async_trait]
impl<E: K8sDeployEnv> AttachProvider<E> for K8sAttachProvider<E> {
    async fn discover(
        &self,
        source: &AttachSource,
    ) -> Result<Vec<AttachedNode<E>>, AttachProviderError> {
        let (namespace, label_selector) = match source {
            AttachSource::K8s {
                namespace,
                label_selector,
            } => (namespace, label_selector),
            _ => {
                return Err(AttachProviderError::UnsupportedSource {
                    attach_source: source.clone(),
                });
            }
        };

        if label_selector.trim().is_empty() {
            return Err(AttachProviderError::Discovery {
                source: K8sAttachDiscoveryError::EmptyLabelSelector.into(),
            });
        }

        let namespace = namespace.as_deref().unwrap_or("default");
        let services = discover_services(&self.client, namespace, label_selector)
            .await
            .map_err(to_discovery_error)?;
        let host = node_host();
        let mut attached = Vec::with_capacity(services.items.len());

        for service in services.items {
            let service_name =
                service
                    .metadata
                    .name
                    .clone()
                    .ok_or_else(|| AttachProviderError::Discovery {
                        source: "k8s service has no metadata.name".into(),
                    })?;

            let api_port = extract_api_node_port(&service).map_err(to_discovery_error)?;
            let endpoint = format!("http://{host}:{api_port}/");
            let source = ExternalNodeSource::new(service_name.clone(), endpoint);
            let client = E::external_node_client(&source).map_err(to_discovery_error)?;

            attached.push(AttachedNode {
                identity_hint: Some(service_name),
                client,
            });
        }

        Ok(attached)
    }
}

fn to_discovery_error(source: DynError) -> AttachProviderError {
    AttachProviderError::Discovery { source }
}

async fn discover_services(
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

fn extract_api_node_port(service: &Service) -> Result<u16, DynError> {
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

    let non_testing: Vec<u16> = ports
        .iter()
        .filter_map(|(name, port)| (!name.contains("testing")).then_some(*port))
        .collect();
    if !non_testing.is_empty() {
        return non_testing;
    }

    ports.into_iter().map(|(_, port)| port).collect()
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

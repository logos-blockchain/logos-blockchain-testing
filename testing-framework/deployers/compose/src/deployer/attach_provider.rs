use std::marker::PhantomData;

use async_trait::async_trait;
use testing_framework_core::scenario::{
    AttachProvider, AttachProviderError, AttachSource, AttachedNode, DynError, ExternalNodeSource,
};
use url::Url;

use crate::{
    docker::attached::{
        discover_attachable_services, discover_service_container_id, inspect_mapped_tcp_ports,
    },
    env::ComposeDeployEnv,
};

pub(super) struct ComposeAttachProvider<E: ComposeDeployEnv> {
    host: String,
    _env: PhantomData<E>,
}

#[derive(Debug, thiserror::Error)]
enum ComposeAttachDiscoveryError {
    #[error("compose attach source requires an explicit project name")]
    MissingProjectName,
}

impl<E: ComposeDeployEnv> ComposeAttachProvider<E> {
    pub(super) fn new(host: String) -> Self {
        Self {
            host,
            _env: PhantomData,
        }
    }
}

#[async_trait]
impl<E: ComposeDeployEnv> AttachProvider<E> for ComposeAttachProvider<E> {
    async fn discover(
        &self,
        source: &AttachSource,
    ) -> Result<Vec<AttachedNode<E>>, AttachProviderError> {
        let (project, services) = match source {
            AttachSource::Compose { project, services } => (project, services),
            _ => {
                return Err(AttachProviderError::UnsupportedSource {
                    attach_source: source.clone(),
                });
            }
        };

        let project = project
            .as_ref()
            .ok_or_else(|| AttachProviderError::Discovery {
                source: ComposeAttachDiscoveryError::MissingProjectName.into(),
            })?;

        let services = resolve_services(project, services)
            .await
            .map_err(to_discovery_error)?;

        let mut attached = Vec::with_capacity(services.len());
        for service in &services {
            let container_id = discover_service_container_id(project, service)
                .await
                .map_err(to_discovery_error)?;

            let api_port = discover_api_port(&container_id)
                .await
                .map_err(to_discovery_error)?;

            let endpoint =
                build_service_endpoint(&self.host, api_port).map_err(to_discovery_error)?;
            let source = ExternalNodeSource::new(service.clone(), endpoint.to_string());
            let client = E::external_node_client(&source).map_err(to_discovery_error)?;

            attached.push(AttachedNode {
                identity_hint: Some(service.clone()),
                client,
            });
        }

        Ok(attached)
    }
}

fn to_discovery_error(source: DynError) -> AttachProviderError {
    AttachProviderError::Discovery { source }
}

async fn resolve_services(project: &str, requested: &[String]) -> Result<Vec<String>, DynError> {
    if !requested.is_empty() {
        return Ok(requested.to_owned());
    }

    discover_attachable_services(project).await
}

async fn discover_api_port(container_id: &str) -> Result<u16, DynError> {
    let mapped_ports = inspect_mapped_tcp_ports(container_id).await?;
    match mapped_ports.as_slice() {
        [] => Err(format!(
            "no mapped tcp ports discovered for attached compose service container '{container_id}'"
        )
        .into()),
        [port] => Ok(port.host_port),
        _ => {
            let mapped_ports = mapped_ports
                .iter()
                .map(|port| format!("{}->{}", port.container_port, port.host_port))
                .collect::<Vec<_>>()
                .join(", ");

            Err(format!(
                "attached compose service container '{container_id}' has multiple mapped tcp ports ({mapped_ports}); provide a single exposed API port"
            )
            .into())
        }
    }
}

fn build_service_endpoint(host: &str, port: u16) -> Result<Url, DynError> {
    let endpoint = Url::parse(&format!("http://{host}:{port}/"))?;
    Ok(endpoint)
}

#[cfg(test)]
mod tests {
    use super::build_service_endpoint;
    use crate::docker::attached::parse_mapped_tcp_ports;

    #[test]
    fn parse_mapped_tcp_ports_skips_non_tcp_and_invalid_keys() {
        let raw = r#"{
          "18018/tcp":[{"HostIp":"0.0.0.0","HostPort":"32001"}],
          "9999/udp":[{"HostIp":"0.0.0.0","HostPort":"39999"}],
          "invalid":[{"HostIp":"0.0.0.0","HostPort":"12345"}]
        }"#;

        let mapped = parse_mapped_tcp_ports(raw).expect("mapped ports should parse");
        assert_eq!(mapped.len(), 1);
        assert_eq!(mapped[0].container_port, 18018);
        assert_eq!(mapped[0].host_port, 32001);
    }

    #[test]
    fn parse_mapped_tcp_ports_returns_sorted_ports() {
        let raw = r#"{
          "18019/tcp":[{"HostIp":"0.0.0.0","HostPort":"32002"}],
          "18018/tcp":[{"HostIp":"0.0.0.0","HostPort":"32001"}]
        }"#;

        let mapped = parse_mapped_tcp_ports(raw).expect("mapped ports should parse");
        assert_eq!(mapped[0].container_port, 18018);
        assert_eq!(mapped[1].container_port, 18019);
    }

    #[test]
    fn build_service_endpoint_formats_http_url() {
        let endpoint = build_service_endpoint("127.0.0.1", 32001).expect("endpoint should parse");
        assert_eq!(endpoint.as_str(), "http://127.0.0.1:32001/");
    }
}

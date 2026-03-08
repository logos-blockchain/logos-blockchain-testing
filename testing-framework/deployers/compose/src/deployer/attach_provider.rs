use std::marker::PhantomData;

use async_trait::async_trait;
use testing_framework_core::scenario::{
    AttachProvider, AttachProviderError, AttachSource, AttachedNode, ClusterWaitHandle, DynError,
    ExternalNodeSource, HttpReadinessRequirement, wait_http_readiness,
};
use url::Url;

use crate::{
    docker::attached::{
        discover_attachable_services, discover_service_container_id,
        inspect_api_container_port_label, inspect_mapped_tcp_ports,
    },
    env::ComposeDeployEnv,
};

pub(super) struct ComposeAttachProvider<E: ComposeDeployEnv> {
    host: String,
    _env: PhantomData<E>,
}

pub(super) struct ComposeAttachedClusterWait<E: ComposeDeployEnv> {
    host: String,
    source: AttachSource,
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

impl<E: ComposeDeployEnv> ComposeAttachedClusterWait<E> {
    pub(super) fn new(host: String, source: AttachSource) -> Self {
        Self {
            host,
            source,
            _env: PhantomData,
        }
    }
}

struct ComposeAttachRequest<'a> {
    project: &'a str,
    services: &'a [String],
}

#[async_trait]
impl<E: ComposeDeployEnv> AttachProvider<E> for ComposeAttachProvider<E> {
    async fn discover(
        &self,
        source: &AttachSource,
    ) -> Result<Vec<AttachedNode<E>>, AttachProviderError> {
        let request = compose_attach_request(source)?;
        let services = resolve_services(request.project, request.services)
            .await
            .map_err(to_discovery_error)?;

        let mut attached = Vec::with_capacity(services.len());
        for service in &services {
            attached.push(
                build_attached_node::<E>(&self.host, request.project, service)
                    .await
                    .map_err(to_discovery_error)?,
            );
        }

        Ok(attached)
    }
}

fn to_discovery_error(source: DynError) -> AttachProviderError {
    AttachProviderError::Discovery { source }
}

fn compose_attach_request(
    source: &AttachSource,
) -> Result<ComposeAttachRequest<'_>, AttachProviderError> {
    let AttachSource::Compose { project, services } = source else {
        return Err(AttachProviderError::UnsupportedSource {
            attach_source: source.clone(),
        });
    };

    let project = project
        .as_deref()
        .ok_or_else(|| AttachProviderError::Discovery {
            source: ComposeAttachDiscoveryError::MissingProjectName.into(),
        })?;

    Ok(ComposeAttachRequest { project, services })
}

async fn build_attached_node<E: ComposeDeployEnv>(
    host: &str,
    project: &str,
    service: &str,
) -> Result<AttachedNode<E>, DynError> {
    let container_id = discover_service_container_id(project, service).await?;
    let api_port = discover_api_port(&container_id).await?;
    let endpoint = build_service_endpoint(host, api_port)?;
    let source = ExternalNodeSource::new(service.to_owned(), endpoint.to_string());
    let client = E::external_node_client(&source)?;

    Ok(AttachedNode {
        identity_hint: Some(service.to_owned()),
        client,
    })
}

pub(super) async fn resolve_services(
    project: &str,
    requested: &[String],
) -> Result<Vec<String>, DynError> {
    if !requested.is_empty() {
        return Ok(requested.to_owned());
    }

    discover_attachable_services(project).await
}

pub(super) async fn discover_api_port(container_id: &str) -> Result<u16, DynError> {
    let mapped_ports = inspect_mapped_tcp_ports(container_id).await?;
    let api_container_port = inspect_api_container_port_label(container_id).await?;
    let Some(api_port) = mapped_ports
        .iter()
        .find(|port| port.container_port == api_container_port)
        .map(|port| port.host_port)
    else {
        let mapped_ports = mapped_ports
            .iter()
            .map(|port| format!("{}->{}", port.container_port, port.host_port))
            .collect::<Vec<_>>()
            .join(", ");

        return Err(format!(
            "attached compose service container '{container_id}' does not expose labeled API container port {api_container_port}; mapped tcp ports: {mapped_ports}"
        )
        .into());
    };

    Ok(api_port)
}

pub(super) fn build_service_endpoint(host: &str, port: u16) -> Result<Url, DynError> {
    let endpoint = Url::parse(&format!("http://{host}:{port}/"))?;
    Ok(endpoint)
}

#[async_trait]
impl<E: ComposeDeployEnv> ClusterWaitHandle<E> for ComposeAttachedClusterWait<E> {
    async fn wait_network_ready(&self) -> Result<(), DynError> {
        let request = compose_wait_request(&self.source)?;
        let services = resolve_services(request.project, request.services).await?;
        let endpoints =
            collect_readiness_endpoints::<E>(&self.host, request.project, &services).await?;

        wait_http_readiness(&endpoints, HttpReadinessRequirement::AllNodesReady).await?;

        Ok(())
    }
}

fn compose_wait_request(source: &AttachSource) -> Result<ComposeAttachRequest<'_>, DynError> {
    let AttachSource::Compose { project, services } = source else {
        return Err("compose cluster wait requires a compose attach source".into());
    };

    let project = project
        .as_deref()
        .ok_or(ComposeAttachDiscoveryError::MissingProjectName)?;

    Ok(ComposeAttachRequest { project, services })
}

async fn collect_readiness_endpoints<E: ComposeDeployEnv>(
    host: &str,
    project: &str,
    services: &[String],
) -> Result<Vec<Url>, DynError> {
    let mut endpoints = Vec::with_capacity(services.len());

    for service in services {
        let container_id = discover_service_container_id(project, service).await?;
        let api_port = discover_api_port(&container_id).await?;
        let mut endpoint = build_service_endpoint(host, api_port)?;
        endpoint.set_path(E::readiness_path());
        endpoints.push(endpoint);
    }

    Ok(endpoints)
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

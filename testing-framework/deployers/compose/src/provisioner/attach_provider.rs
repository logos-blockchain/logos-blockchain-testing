use std::marker::PhantomData;

use async_trait::async_trait;
use testing_framework_core::scenario::{
    ClusterWaitHandle, DynError, ExistingCluster, ExternalNodeSource, HttpReadinessRequirement,
    wait_http_readiness,
};
use url::Url;

use crate::{
    docker::attached::{
        discover_all_services, discover_attachable_services, discover_running_attachable_services,
        discover_running_services, discover_service_container_id, inspect_api_container_port_label,
        inspect_mapped_tcp_ports,
    },
    env::{ComposeDeployEnv, readiness_http_path},
};

pub(crate) struct ComposeAttachProvider<E: ComposeDeployEnv> {
    host: String,
    _env: PhantomData<E>,
}

pub(crate) struct ComposeAttachedClusterWait<E: ComposeDeployEnv> {
    host: String,
    source: ExistingCluster,
    _env: PhantomData<E>,
}

#[derive(Debug, thiserror::Error)]
enum ComposeAttachDiscoveryError {
    #[error("compose attach source requires an explicit project name")]
    MissingProjectName,
    #[error("existing cluster descriptor is not supported by this provider: {attach_source:?}")]
    UnsupportedSource { attach_source: ExistingCluster },
    #[error(
        "unknown compose services requested for project '{project}': {unknown:?}; available \
         services: {available:?}"
    )]
    UnknownServices {
        project: String,
        unknown: Vec<String>,
        available: Vec<String>,
    },
    #[error("no running compose services remain to await readiness in project '{project}'")]
    NoRunningServices { project: String },
}

impl<E: ComposeDeployEnv> ComposeAttachProvider<E> {
    pub(crate) fn new(host: String) -> Self {
        Self {
            host,
            _env: PhantomData,
        }
    }
}

impl<E: ComposeDeployEnv> ComposeAttachedClusterWait<E> {
    pub(crate) fn try_new(host: String, source: &ExistingCluster) -> Result<Self, DynError> {
        let _ = compose_wait_request(source)?;

        Ok(Self {
            host,
            source: source.clone(),
            _env: PhantomData,
        })
    }
}

struct ComposeAttachRequest<'a> {
    project: &'a str,
    services: &'a [String],
}

impl<E: ComposeDeployEnv> ComposeAttachProvider<E> {
    /// Discovers node service names and clients for the requested existing
    /// compose cluster.
    pub(crate) async fn discover(
        &self,
        source: &ExistingCluster,
    ) -> Result<Vec<(String, E::NodeClient)>, DynError> {
        let request = compose_attach_request(source)?;
        let services = resolve_services(request.project, request.services).await?;

        let mut nodes = Vec::with_capacity(services.len());
        for service in services {
            let client = build_attached_client::<E>(&self.host, request.project, &service).await?;
            nodes.push((service, client));
        }

        Ok(nodes)
    }
}

fn compose_attach_request(source: &ExistingCluster) -> Result<ComposeAttachRequest<'_>, DynError> {
    let services = source.compose_services().ok_or_else(|| {
        ComposeAttachDiscoveryError::UnsupportedSource {
            attach_source: source.clone(),
        }
    })?;

    let project = source
        .compose_project()
        .ok_or(ComposeAttachDiscoveryError::MissingProjectName)?;

    Ok(ComposeAttachRequest { project, services })
}

async fn build_attached_client<E: ComposeDeployEnv>(
    host: &str,
    project: &str,
    service: &str,
) -> Result<E::NodeClient, DynError> {
    let container_id = discover_service_container_id(project, service).await?;
    let api_port = discover_api_port(&container_id).await?;
    let endpoint = build_service_endpoint(host, api_port)?;
    let source = ExternalNodeSource::new(service.to_owned(), endpoint.to_string());

    E::external_node_client(&source)
}

pub(super) async fn resolve_services(
    project: &str,
    requested: &[String],
) -> Result<Vec<String>, DynError> {
    if requested.is_empty() {
        return discover_attachable_services(project).await;
    }

    let known = discover_all_services(project).await?;
    ensure_known_services(project, requested, &known)?;
    Ok(requested.to_owned())
}

/// Rejects requested service names that do not exist in the compose project
/// at all, so typos fail loudly instead of silently shrinking the wait set.
fn ensure_known_services(
    project: &str,
    requested: &[String],
    known: &[String],
) -> Result<(), ComposeAttachDiscoveryError> {
    let unknown: Vec<String> = requested
        .iter()
        .filter(|service| !known.contains(service))
        .cloned()
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(ComposeAttachDiscoveryError::UnknownServices {
        project: project.to_owned(),
        unknown,
        available: known.to_vec(),
    })
}

/// Keeps only running services in the wait set; every requested service being
/// stopped is an error rather than a vacuously satisfied wait.
fn partition_running_wait_services(
    project: &str,
    requested: &[String],
    running: &[String],
) -> Result<Vec<String>, ComposeAttachDiscoveryError> {
    let kept: Vec<String> = requested
        .iter()
        .filter(|service| running.contains(service))
        .cloned()
        .collect();
    if kept.is_empty() {
        return Err(ComposeAttachDiscoveryError::NoRunningServices {
            project: project.to_owned(),
        });
    }
    Ok(kept)
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
        let services = resolve_running_wait_services(request.project, request.services).await?;
        let endpoints =
            collect_readiness_endpoints::<E>(&self.host, request.project, &services).await?;

        wait_http_readiness(&endpoints, HttpReadinessRequirement::AllNodesReady).await?;

        Ok(())
    }
}

async fn resolve_running_wait_services(
    project: &str,
    requested: &[String],
) -> Result<Vec<String>, DynError> {
    if requested.is_empty() {
        let running = discover_running_attachable_services(project).await?;
        if running.is_empty() {
            return Err(ComposeAttachDiscoveryError::NoRunningServices {
                project: project.to_owned(),
            }
            .into());
        }
        return Ok(running);
    }

    let known = discover_all_services(project).await?;
    ensure_known_services(project, requested, &known)?;
    let running = discover_running_services(project).await?;
    Ok(partition_running_wait_services(
        project, requested, &running,
    )?)
}

fn compose_wait_request(source: &ExistingCluster) -> Result<ComposeAttachRequest<'_>, DynError> {
    let project = source.compose_project().ok_or_else(|| {
        DynError::from("compose cluster wait requires a compose existing-cluster descriptor")
    })?;
    let services = source.compose_services().ok_or_else(|| {
        DynError::from("compose cluster wait requires a compose existing-cluster descriptor")
    })?;

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
        endpoint.set_path(readiness_http_path::<E>());
        endpoints.push(endpoint);
    }

    Ok(endpoints)
}

#[cfg(test)]
mod tests {
    use super::{
        ComposeAttachDiscoveryError, build_service_endpoint, ensure_known_services,
        partition_running_wait_services,
    };
    use crate::docker::attached::parse_mapped_tcp_ports;

    fn names(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn unknown_requested_services_fail_with_available_inventory() {
        let requested = names(&["node-0", "node-9"]);
        let known = names(&["node-0", "node-1"]);

        let error = ensure_known_services("demo", &requested, &known)
            .err()
            .expect("unknown service names must be rejected");

        assert!(matches!(
            &error,
            ComposeAttachDiscoveryError::UnknownServices { project, unknown, available }
                if project == "demo"
                    && unknown == &names(&["node-9"])
                    && available == &known
        ));
        let message = error.to_string();
        assert!(message.contains("node-9"));
        assert!(message.contains("node-1"));
    }

    #[test]
    fn fully_known_requested_services_pass_validation() {
        let requested = names(&["node-0"]);
        let known = names(&["node-0", "node-1"]);

        assert!(ensure_known_services("demo", &requested, &known).is_ok());
    }

    #[test]
    fn stopped_services_are_filtered_while_some_nodes_run() {
        let requested = names(&["node-0", "node-1"]);
        let running = names(&["node-1"]);

        let kept = partition_running_wait_services("demo", &requested, &running)
            .expect("a partially running wait set must remain valid");

        assert_eq!(kept, names(&["node-1"]));
    }

    #[test]
    fn all_stopped_services_fail_the_wait_instead_of_passing_vacuously() {
        let requested = names(&["node-0", "node-1"]);
        let running = Vec::new();

        let error = partition_running_wait_services("demo", &requested, &running)
            .err()
            .expect("an all-stopped wait set must be rejected");

        assert!(matches!(
            error,
            ComposeAttachDiscoveryError::NoRunningServices { project } if project == "demo"
        ));
    }

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

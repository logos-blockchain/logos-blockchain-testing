use std::{path::Path, time::Duration};

use async_trait::async_trait;
use reqwest::Url;
use testing_framework_core::{
    cfgsync::{
        CfgsyncOutputPaths, MaterializedArtifacts, RegistrationServerRenderOptions,
        StaticArtifactRenderer, render_and_write_registration_server,
    },
    scenario::{
        Application, DynError, HttpReadinessRequirement, NodeClients,
        wait_for_http_ports_with_host_and_requirement, wait_http_readiness,
    },
};

use crate::{
    descriptor::{ComposeDescriptor, NodeDescriptor},
    docker::config_server::DockerConfigServerSpec,
    infrastructure::ports::{
        HostPortMapping, NodeContainerPorts, NodeHostPorts, compose_runner_host,
    },
};

/// Handle returned by a compose config server (cfgsync or equivalent).
pub trait ConfigServerHandle: Send + Sync {
    fn shutdown(&mut self);
    fn mark_preserved(&mut self);
    fn container_name(&self) -> Option<&str> {
        None
    }
}

/// Compose-specific topology surface needed by the runner.
#[async_trait]
pub trait ComposeDeployEnv: Application {
    /// Produce the compose descriptor for the given topology.
    fn compose_descriptor(
        topology: &<Self as Application>::Deployment,
        cfgsync_port: u16,
    ) -> ComposeDescriptor;

    /// Container ports (API/testing) per node, used for docker-compose port
    /// discovery.
    fn node_container_ports(
        topology: &<Self as Application>::Deployment,
    ) -> Vec<NodeContainerPorts> {
        let descriptor = Self::compose_descriptor(topology, 0);
        descriptor
            .nodes()
            .iter()
            .enumerate()
            .filter_map(|(index, node)| parse_node_container_ports(index, node))
            .collect()
    }

    /// Hostnames used when rewriting node configs for cfgsync delivery.
    fn cfgsync_hostnames(topology: &<Self as Application>::Deployment) -> Vec<String>;

    /// App-specific cfgsync artifact enrichment.
    fn enrich_cfgsync_artifacts(
        _topology: &<Self as Application>::Deployment,
        _artifacts: &mut MaterializedArtifacts,
    ) -> Result<(), DynError> {
        Ok(())
    }

    /// Render and write cfgsync runtime files for the current topology.
    fn write_cfgsync_config(
        path: &Path,
        topology: &<Self as Application>::Deployment,
        port: u16,
        metrics_otlp_ingest_url: Option<&Url>,
    ) -> Result<(), DynError>
    where
        Self: Sized + StaticArtifactRenderer<Deployment = <Self as Application>::Deployment>,
    {
        let _ = metrics_otlp_ingest_url;
        let options = RegistrationServerRenderOptions {
            port: Some(port),
            artifacts_path: None,
        };
        let artifacts_path = cfgsync_artifacts_path(path);
        let output = CfgsyncOutputPaths {
            config_path: path,
            artifacts_path: &artifacts_path,
        };

        render_and_write_registration_server::<Self, _>(
            topology,
            &Self::cfgsync_hostnames(topology),
            options,
            output,
            |artifacts| Self::enrich_cfgsync_artifacts(topology, artifacts),
        )?;

        Ok(())
    }

    /// Build the config server container specification.
    fn cfgsync_container_spec(
        cfgsync_path: &Path,
        port: u16,
        network: &str,
    ) -> Result<DockerConfigServerSpec, DynError>;

    /// Timeout used when launching the config server container.
    fn cfgsync_start_timeout() -> Duration {
        Duration::from_secs(180)
    }

    /// Build node clients from discovered host ports.
    fn node_client_from_ports(
        ports: &NodeHostPorts,
        host: &str,
    ) -> Result<Self::NodeClient, DynError>;

    /// Build node clients from discovered host ports.
    fn build_node_clients(
        _topology: &<Self as Application>::Deployment,
        host_ports: &HostPortMapping,
        host: &str,
    ) -> Result<NodeClients<Self>, DynError>
    where
        Self: Sized,
    {
        let clients = host_ports
            .nodes
            .iter()
            .map(|ports| Self::node_client_from_ports(ports, host))
            .collect::<Result<_, _>>()?;
        Ok(NodeClients::new(clients))
    }

    /// Path used by default readiness checks.
    fn readiness_path() -> &'static str {
        "/"
    }

    /// Host used by default remote readiness checks.
    fn compose_runner_host() -> String {
        compose_runner_host()
    }

    /// Remote readiness probe for node APIs.
    async fn wait_remote_readiness(
        _topology: &<Self as Application>::Deployment,
        mapping: &HostPortMapping,
        requirement: HttpReadinessRequirement,
    ) -> Result<(), DynError> {
        let host = Self::compose_runner_host();
        let urls = readiness_urls(&host, mapping, Self::readiness_path())?;
        wait_http_readiness(&urls, requirement).await?;
        Ok(())
    }

    /// Wait for HTTP readiness on node ports.
    async fn wait_for_nodes(
        ports: &[u16],
        host: &str,
        requirement: HttpReadinessRequirement,
    ) -> Result<(), DynError> {
        wait_for_http_ports_with_host_and_requirement(
            ports,
            host,
            Self::readiness_path(),
            requirement,
        )
        .await?;
        Ok(())
    }
}

pub trait ComposeCfgsyncEnv:
    ComposeDeployEnv + StaticArtifactRenderer<Deployment = <Self as Application>::Deployment>
{
}

impl<T> ComposeCfgsyncEnv for T where
    T: ComposeDeployEnv + StaticArtifactRenderer<Deployment = <T as Application>::Deployment>
{
}

fn cfgsync_artifacts_path(config_path: &Path) -> std::path::PathBuf {
    config_path
        .parent()
        .unwrap_or(config_path)
        .join("cfgsync.artifacts.yaml")
}

fn parse_node_container_ports(index: usize, node: &NodeDescriptor) -> Option<NodeContainerPorts> {
    let mut ports = node.container_ports().iter().copied();
    let api = ports.next()?;
    let testing = ports.next()?;

    Some(NodeContainerPorts {
        index,
        api,
        testing,
    })
}

fn readiness_urls(
    host: &str,
    mapping: &HostPortMapping,
    endpoint_path: &str,
) -> Result<Vec<Url>, DynError> {
    let endpoint_path = normalize_endpoint_path(endpoint_path);

    mapping
        .nodes
        .iter()
        .map(|ports| readiness_url(host, ports.api, &endpoint_path))
        .collect::<Result<_, _>>()
}

fn normalize_endpoint_path(endpoint_path: &str) -> String {
    if endpoint_path.starts_with('/') {
        endpoint_path.to_string()
    } else {
        format!("/{endpoint_path}")
    }
}

fn readiness_url(host: &str, api_port: u16, endpoint_path: &str) -> Result<Url, DynError> {
    let url = Url::parse(&format!("http://{host}:{api_port}{endpoint_path}"))?;
    Ok(url)
}

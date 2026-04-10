use std::{fs, path::Path, time::Duration};

use async_trait::async_trait;
use reqwest::Url;
use testing_framework_core::{
    cfgsync::{MaterializedArtifacts, StaticArtifactRenderer},
    scenario::{
        Application, DynError, HttpReadinessRequirement, NodeAccess, NodeClients,
        wait_for_http_ports_with_host_and_requirement, wait_http_readiness,
    },
    topology::DeploymentDescriptor,
};
use tokio::{
    net::TcpStream,
    time::{Instant, sleep},
};

use crate::{
    descriptor::{
        BinaryConfigNodeSpec, ComposeDescriptor, LoopbackNodeRuntimeSpec, NodeDescriptor,
        binary_config_node_runtime_spec,
    },
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposeConfigServerMode {
    Disabled,
    Docker,
}

/// Compose-specific topology surface needed by the runner.
#[async_trait]
pub trait ComposeDeployEnv: Application {
    /// Write per-node config files or other compose-time assets into the stack
    /// workspace before the stack starts.
    fn prepare_compose_configs(
        _path: &Path,
        _topology: &<Self as Application>::Deployment,
        _metrics_otlp_ingest_url: Option<&Url>,
    ) -> Result<(), DynError> {
        Ok(())
    }

    /// File name for a static per-node config rendered into the compose stack.
    fn static_node_config_file_name(index: usize) -> String {
        format!("node-{index}.yaml")
    }

    fn loopback_node_runtime_spec(
        _topology: &<Self as Application>::Deployment,
        _index: usize,
    ) -> Option<LoopbackNodeRuntimeSpec> {
        if let Some(spec) = Self::binary_config_node_spec(_topology, _index) {
            return Some(binary_config_node_runtime_spec(_index, &spec));
        }
        None
    }

    fn binary_config_node_spec(
        _topology: &<Self as Application>::Deployment,
        _index: usize,
    ) -> Option<BinaryConfigNodeSpec> {
        None
    }

    /// Produce the compose descriptor for the given topology.
    fn compose_descriptor(
        topology: &<Self as Application>::Deployment,
        _cfgsync_port: u16,
    ) -> Result<ComposeDescriptor, DynError> {
        let mut nodes = Vec::with_capacity(topology.node_count());
        for index in 0..topology.node_count() {
            let spec = Self::loopback_node_runtime_spec(topology, index).ok_or_else(|| {
                std::io::Error::other("compose_descriptor is not implemented for this app")
            })?;
            nodes.push(NodeDescriptor::with_loopback_ports(
                crate::infrastructure::ports::node_identifier(index),
                spec.image,
                spec.entrypoint,
                spec.volumes,
                spec.extra_hosts,
                spec.container_ports,
                spec.environment,
                spec.platform,
            ));
        }
        Ok(ComposeDescriptor::new(nodes))
    }

    /// Container ports (API/testing) per node, used for docker-compose port
    /// discovery.
    fn node_container_ports(
        topology: &<Self as Application>::Deployment,
    ) -> Result<Vec<NodeContainerPorts>, DynError> {
        let descriptor = Self::compose_descriptor(topology, 0)?;
        Ok(descriptor
            .nodes()
            .iter()
            .enumerate()
            .take(topology.node_count())
            .filter_map(|(index, node)| parse_node_container_ports(index, node))
            .collect())
    }

    /// Hostnames used when rewriting node configs for cfgsync delivery.
    fn cfgsync_hostnames(topology: &<Self as Application>::Deployment) -> Vec<String> {
        (0..topology.node_count())
            .map(crate::infrastructure::ports::node_identifier)
            .collect()
    }

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
        write_static_compose_configs::<Self>(path, topology, metrics_otlp_ingest_url)?;
        write_dummy_cfgsync_config(path, port)?;
        Ok(())
    }

    /// Build the config server container specification.
    fn cfgsync_container_spec(
        _cfgsync_path: &Path,
        _port: u16,
        _network: &str,
    ) -> Result<DockerConfigServerSpec, DynError> {
        Err(std::io::Error::other("cfgsync_container_spec is not implemented for this app").into())
    }

    /// Timeout used when launching the config server container.
    fn cfgsync_start_timeout() -> Duration {
        Duration::from_secs(180)
    }

    fn cfgsync_server_mode() -> ComposeConfigServerMode {
        ComposeConfigServerMode::Disabled
    }

    /// Build node clients from discovered host ports.
    fn node_client_from_ports(
        ports: &NodeHostPorts,
        host: &str,
    ) -> Result<Self::NodeClient, DynError> {
        <Self as Application>::build_node_client(&discovered_node_access(host, ports))
    }

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
    fn node_readiness_path() -> &'static str {
        <Self as Application>::node_readiness_path()
    }

    fn node_readiness_probe() -> ComposeReadinessProbe {
        ComposeReadinessProbe::Http {
            path: <Self as ComposeDeployEnv>::node_readiness_path(),
        }
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
        match <Self as ComposeDeployEnv>::node_readiness_probe() {
            ComposeReadinessProbe::Http { path } => {
                let host = Self::compose_runner_host();
                let urls = readiness_urls(&host, mapping, path)?;
                wait_http_readiness(&urls, requirement).await?;
                Ok(())
            }
            ComposeReadinessProbe::Tcp => wait_for_tcp_readiness(&mapping.nodes, requirement).await,
        }
    }

    /// Wait for HTTP readiness on node ports.
    async fn wait_for_nodes(
        ports: &[u16],
        host: &str,
        requirement: HttpReadinessRequirement,
    ) -> Result<(), DynError> {
        match <Self as ComposeDeployEnv>::node_readiness_probe() {
            ComposeReadinessProbe::Http { path } => {
                wait_for_http_ports_with_host_and_requirement(ports, host, path, requirement)
                    .await?;
                Ok(())
            }
            ComposeReadinessProbe::Tcp => {
                let ports = ports
                    .iter()
                    .copied()
                    .map(|port| NodeHostPorts {
                        api: port,
                        testing: port,
                    })
                    .collect::<Vec<_>>();
                wait_for_tcp_readiness(&ports, requirement).await
            }
        }
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

fn write_static_compose_configs<E>(
    path: &Path,
    topology: &<E as Application>::Deployment,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<(), DynError>
where
    E: ComposeDeployEnv + StaticArtifactRenderer<Deployment = <E as Application>::Deployment>,
{
    E::prepare_compose_configs(path, topology, metrics_otlp_ingest_url)?;

    let hostnames = E::cfgsync_hostnames(topology);
    let configs_dir = stack_configs_dir(path)?;
    fs::create_dir_all(&configs_dir)?;

    for index in 0..topology.node_count() {
        let mut config = E::build_node_config(topology, index)?;
        E::rewrite_for_hostnames(topology, index, &hostnames, &mut config)?;
        let rendered = E::serialize_node_config(&config)?;
        let output_path = configs_dir.join(E::static_node_config_file_name(index));
        fs::write(&output_path, rendered)?;
    }

    Ok(())
}

fn stack_configs_dir(cfgsync_path: &Path) -> Result<std::path::PathBuf, DynError> {
    let stack_dir = cfgsync_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cfgsync path has no parent"))?;
    Ok(stack_dir.join("configs"))
}

fn write_dummy_cfgsync_config(path: &Path, port: u16) -> Result<(), DynError> {
    fs::write(
        path,
        format!(
            "port: {port}\nsource:\n  kind: static\n  artifacts_path: cfgsync.artifacts.yaml\n"
        ),
    )?;
    Ok(())
}

fn parse_node_container_ports(index: usize, node: &NodeDescriptor) -> Option<NodeContainerPorts> {
    let mut ports = node.container_ports().iter().copied();
    let api = ports.next()?;
    let testing = ports.next().unwrap_or(api);

    Some(NodeContainerPorts {
        index,
        api,
        testing,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposeReadinessProbe {
    Http { path: &'static str },
    Tcp,
}

async fn wait_for_tcp_readiness(
    ports: &[NodeHostPorts],
    requirement: HttpReadinessRequirement,
) -> Result<(), DynError> {
    let timeout = Duration::from_secs(60);
    let deadline = Instant::now() + timeout;

    loop {
        let mut ready = 0;
        for node in ports {
            if TcpStream::connect(("127.0.0.1", node.testing))
                .await
                .is_ok()
            {
                ready += 1;
            }
        }

        let total = ports.len();
        let satisfied = match requirement {
            HttpReadinessRequirement::AllNodesReady => ready == total,
            HttpReadinessRequirement::AnyNodeReady => ready >= 1,
            HttpReadinessRequirement::AtLeast(min_ready) => ready >= min_ready,
        };

        if satisfied {
            return Ok(());
        }

        if Instant::now() >= deadline {
            return Err(format!(
                "tcp readiness timed out: ready={ready}, total={total}, requirement={requirement:?}"
            )
            .into());
        }

        sleep(Duration::from_millis(200)).await;
    }
}

pub fn discovered_node_access(host: &str, ports: &NodeHostPorts) -> NodeAccess {
    NodeAccess::new(host, ports.api).with_testing_port(ports.testing)
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

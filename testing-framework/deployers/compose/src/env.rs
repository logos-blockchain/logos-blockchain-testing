use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use async_trait::async_trait;
use reqwest::Url;
use testing_framework_core::{
    cfgsync::{
        CfgsyncOutputPaths, MaterializedArtifacts, RegistrationServerRenderOptions,
        StaticArtifactRenderer, render_and_write_registration_server,
    },
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
        binary_config_node_runtime_spec, build_loopback_node_descriptors,
    },
    docker::config_server::DockerConfigServerSpec,
    infrastructure::ports::{
        HostPortMapping, NodeContainerPorts, NodeHostPorts, compose_runner_host,
    },
};

/// Handle returned by a compose config server (cfgsync or equivalent).
pub trait ConfigServerHandle: Send + Sync {
    /// Stops the config server and releases any local resources it owns.
    fn shutdown(&mut self);
    /// Marks the config server as preserved so runner cleanup should not remove
    /// it.
    fn mark_preserved(&mut self);
    /// Returns the backing container name when the handle is container-backed.
    fn container_name(&self) -> Option<&str> {
        None
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Selects how compose nodes receive config updates.
pub enum ComposeConfigServerMode {
    /// Do not start any config server.
    Disabled,
    /// Start a Docker-backed config server sidecar.
    Docker,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Readiness probe used for compose nodes.
pub enum ComposeReadinessProbe {
    /// Probe a concrete HTTP path on each node.
    Http { path: &'static str },
    /// Probe raw TCP reachability on the testing port.
    Tcp,
}

#[derive(Clone, Copy)]
/// Naming strategy for static compose node config files.
pub enum ComposeNodeConfigFileName {
    /// Use `node-{index}.<extension>`.
    FixedExtension(&'static str),
    /// Build the file name directly from the node index.
    Custom(fn(usize) -> String),
}

impl ComposeNodeConfigFileName {
    /// Resolves the config file name for one node index.
    #[must_use]
    pub fn resolve(&self, index: usize) -> String {
        match self {
            Self::FixedExtension(extension) => format!("node-{index}.{extension}"),
            Self::Custom(build) => build(index),
        }
    }
}

/// Advanced compose deployer integration.
#[async_trait]
pub trait ComposeDeployEnv: Application + Sized {
    /// Prepares compose workspace files before the stack is started.
    fn prepare_compose_configs(
        _path: &Path,
        _topology: &<Self as Application>::Deployment,
        _cfgsync_port: u16,
        _metrics_otlp_ingest_url: Option<&Url>,
    ) -> Result<(), DynError> {
        Ok(())
    }

    /// Returns the static config file name used for one node.
    fn static_node_config_file_name(index: usize) -> String {
        format!("node-{index}.yaml")
    }

    /// Returns the runtime spec for one loopback node when using the standard
    /// one-container-per-node shape.
    fn loopback_node_runtime_spec(
        topology: &<Self as Application>::Deployment,
        index: usize,
    ) -> Result<Option<LoopbackNodeRuntimeSpec>, DynError> {
        if let Some(spec) = Self::binary_config_node_spec(topology, index)? {
            return Ok(Some(binary_config_node_runtime_spec(index, &spec)));
        }
        Ok(None)
    }

    /// Returns the binary+config node spec when the app follows the standard
    /// binary compose path.
    fn binary_config_node_spec(
        _topology: &<Self as Application>::Deployment,
        _index: usize,
    ) -> Result<Option<BinaryConfigNodeSpec>, DynError> {
        Ok(None)
    }

    /// Builds the full compose descriptor for the deployment.
    fn compose_descriptor(
        topology: &<Self as Application>::Deployment,
        _cfgsync_port: u16,
    ) -> Result<ComposeDescriptor, DynError> {
        let nodes = build_loopback_node_descriptors(topology.node_count(), |index| {
            Self::loopback_node_runtime_spec(topology, index)
                .ok()
                .flatten()
                .unwrap_or_else(|| panic!("compose_descriptor is not implemented for this app"))
        });
        Ok(ComposeDescriptor::new(nodes))
    }

    /// Returns the container ports exposed by each compose node.
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

    /// Returns the hostnames advertised to cfgsync-rendered node configs.
    fn cfgsync_hostnames(topology: &<Self as Application>::Deployment) -> Vec<String> {
        (0..topology.node_count())
            .map(crate::infrastructure::ports::node_identifier)
            .collect()
    }

    /// Adds extra artifacts to the cfgsync materialization output.
    fn enrich_cfgsync_artifacts(
        _topology: &<Self as Application>::Deployment,
        _artifacts: &mut MaterializedArtifacts,
    ) -> Result<(), DynError> {
        Ok(())
    }

    /// Selects how compose nodes receive config updates.
    fn cfgsync_server_mode() -> ComposeConfigServerMode {
        ComposeConfigServerMode::Disabled
    }

    /// Builds the config-server container spec when cfgsync is enabled.
    fn cfgsync_container_spec(
        _cfgsync_path: &Path,
        _port: u16,
        _network: &str,
    ) -> Result<DockerConfigServerSpec, DynError> {
        Err(std::io::Error::other("cfgsync_container_spec is not implemented for this app").into())
    }

    /// Returns how long the runner should wait for the config server to start.
    fn cfgsync_start_timeout() -> Duration {
        Duration::from_secs(180)
    }

    /// Builds one node client from mapped compose ports.
    fn node_client_from_ports(
        ports: &NodeHostPorts,
        host: &str,
    ) -> Result<Self::NodeClient, DynError> {
        <Self as Application>::build_node_client(&discovered_node_access(host, ports))
    }

    /// Builds node clients for the full compose deployment.
    fn build_node_clients(
        _topology: &<Self as Application>::Deployment,
        host_ports: &HostPortMapping,
        host: &str,
    ) -> Result<NodeClients<Self>, DynError> {
        let clients = host_ports
            .nodes
            .iter()
            .map(|ports| Self::node_client_from_ports(ports, host))
            .collect::<Result<_, _>>()?;
        Ok(NodeClients::new(clients))
    }

    /// Returns the readiness probe used for compose nodes.
    fn readiness_probe() -> ComposeReadinessProbe {
        ComposeReadinessProbe::Http {
            path: <Self as Application>::node_readiness_path(),
        }
    }

    /// Returns the host that should be used to access forwarded compose ports.
    fn compose_runner_host() -> String {
        compose_runner_host()
    }

    /// Waits for remote readiness using the app's compose-specific probe model.
    async fn wait_remote_readiness(
        _topology: &<Self as Application>::Deployment,
        mapping: &HostPortMapping,
        requirement: HttpReadinessRequirement,
    ) -> Result<(), DynError> {
        match Self::readiness_probe() {
            ComposeReadinessProbe::Http { path } => {
                let host = Self::compose_runner_host();
                let urls = readiness_urls(&host, mapping, path)?;
                wait_http_readiness(&urls, requirement).await?;
                Ok(())
            }
            ComposeReadinessProbe::Tcp => wait_for_tcp_readiness(&mapping.nodes, requirement).await,
        }
    }

    /// Waits for local host ports using the app's compose-specific probe model.
    async fn wait_for_nodes(
        ports: &[u16],
        host: &str,
        requirement: HttpReadinessRequirement,
    ) -> Result<(), DynError> {
        match Self::readiness_probe() {
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

/// Common compose binary-app path.
pub trait ComposeBinaryApp:
    Application + Sized + StaticArtifactRenderer<Deployment = <Self as Application>::Deployment>
{
    /// Returns the binary+config runtime spec shared by all compose nodes.
    fn compose_node_spec() -> BinaryConfigNodeSpec;

    /// Prepares any extra workspace files needed before config rendering.
    fn prepare_compose_workspace(
        _path: &Path,
        _topology: &<Self as Application>::Deployment,
        _metrics_otlp_ingest_url: Option<&Url>,
    ) -> Result<(), DynError> {
        Ok(())
    }

    /// Returns extra non-node services that should be added to the compose
    /// stack.
    fn compose_extra_services(
        _topology: &<Self as Application>::Deployment,
    ) -> Result<Vec<NodeDescriptor>, DynError> {
        Ok(Vec::new())
    }

    /// Returns the static node config file name used for one node.
    fn static_node_config_file_name(index: usize) -> String {
        make_extension_node_config_file_name(&Self::compose_node_spec().config_file_extension)
            .resolve(index)
    }

    /// Builds one node client from mapped compose ports.
    fn node_client_from_ports(
        ports: &NodeHostPorts,
        host: &str,
    ) -> Result<Self::NodeClient, DynError> {
        <Self as Application>::build_node_client(&discovered_node_access(host, ports))
    }

    /// Returns the readiness probe used for compose nodes.
    fn readiness_probe() -> ComposeReadinessProbe {
        ComposeReadinessProbe::Http {
            path: <Self as Application>::node_readiness_path(),
        }
    }

    /// Returns the host that should be used to access forwarded compose ports.
    fn compose_runner_host() -> String {
        compose_runner_host()
    }
}

impl<T> ComposeDeployEnv for T
where
    T: ComposeBinaryApp,
{
    fn prepare_compose_configs(
        path: &Path,
        topology: &<Self as Application>::Deployment,
        _cfgsync_port: u16,
        metrics_otlp_ingest_url: Option<&Url>,
    ) -> Result<(), DynError> {
        T::prepare_compose_workspace(path, topology, metrics_otlp_ingest_url)?;
        write_static_compose_configs::<T>(path, topology)
    }

    fn static_node_config_file_name(index: usize) -> String {
        T::static_node_config_file_name(index)
    }

    fn binary_config_node_spec(
        _topology: &<Self as Application>::Deployment,
        _index: usize,
    ) -> Result<Option<BinaryConfigNodeSpec>, DynError> {
        Ok(Some(T::compose_node_spec()))
    }

    fn compose_descriptor(
        topology: &<Self as Application>::Deployment,
        _cfgsync_port: u16,
    ) -> Result<ComposeDescriptor, DynError> {
        let spec = T::compose_node_spec();
        let mut nodes = (0..topology.node_count())
            .map(|index| {
                let file_name = T::static_node_config_file_name(index);
                let runtime = binary_config_node_runtime_spec(index, &spec);
                NodeDescriptor::with_loopback_ports(
                    crate::infrastructure::ports::node_identifier(index),
                    runtime.image,
                    runtime.entrypoint,
                    vec![format!(
                        "./stack/configs/{file_name}:{}:ro",
                        spec.config_container_path
                    )],
                    runtime.extra_hosts,
                    runtime.container_ports,
                    runtime.environment,
                    runtime.platform,
                )
            })
            .collect::<Vec<_>>();
        nodes.extend(T::compose_extra_services(topology)?);
        Ok(ComposeDescriptor::new(nodes))
    }

    fn node_client_from_ports(
        ports: &NodeHostPorts,
        host: &str,
    ) -> Result<Self::NodeClient, DynError> {
        T::node_client_from_ports(ports, host)
    }

    fn readiness_probe() -> ComposeReadinessProbe {
        T::readiness_probe()
    }

    fn compose_runner_host() -> String {
        T::compose_runner_host()
    }
}

pub(crate) fn prepare_compose_configs<E: ComposeDeployEnv>(
    path: &Path,
    topology: &E::Deployment,
    cfgsync_port: u16,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<(), DynError> {
    E::prepare_compose_configs(path, topology, cfgsync_port, metrics_otlp_ingest_url)
}

pub(crate) fn compose_descriptor<E: ComposeDeployEnv>(
    topology: &E::Deployment,
    cfgsync_port: u16,
) -> Result<ComposeDescriptor, DynError> {
    E::compose_descriptor(topology, cfgsync_port)
}

pub(crate) fn node_container_ports<E: ComposeDeployEnv>(
    topology: &E::Deployment,
) -> Result<Vec<NodeContainerPorts>, DynError> {
    E::node_container_ports(topology)
}

pub(crate) fn cfgsync_container_spec<E: ComposeDeployEnv>(
    cfgsync_path: &Path,
    port: u16,
    network: &str,
) -> Result<DockerConfigServerSpec, DynError> {
    E::cfgsync_container_spec(cfgsync_path, port, network)
}

pub(crate) fn cfgsync_start_timeout<E: ComposeDeployEnv>() -> Duration {
    E::cfgsync_start_timeout()
}

pub(crate) fn cfgsync_server_mode<E: ComposeDeployEnv>() -> ComposeConfigServerMode {
    E::cfgsync_server_mode()
}

pub(crate) fn readiness_http_path<E: ComposeDeployEnv>() -> &'static str {
    match E::readiness_probe() {
        ComposeReadinessProbe::Http { path } => path,
        ComposeReadinessProbe::Tcp => <E as Application>::node_readiness_path(),
    }
}

pub(crate) fn build_node_clients<E: ComposeDeployEnv>(
    topology: &E::Deployment,
    host_ports: &HostPortMapping,
    host: &str,
) -> Result<NodeClients<E>, DynError> {
    E::build_node_clients(topology, host_ports, host)
}

pub(crate) fn wait_remote_readiness<E: ComposeDeployEnv>(
    topology: &E::Deployment,
    mapping: &HostPortMapping,
    requirement: HttpReadinessRequirement,
) -> Result<impl std::future::Future<Output = Result<(), DynError>>, DynError> {
    let topology = topology.clone();
    let mapping = mapping.clone();
    Ok(async move { E::wait_remote_readiness(&topology, &mapping, requirement).await })
}

pub(crate) fn wait_for_nodes<E: ComposeDeployEnv>(
    ports: &[u16],
    host: &str,
    requirement: HttpReadinessRequirement,
) -> Result<impl std::future::Future<Output = Result<(), DynError>>, DynError> {
    let node_ports = ports.to_vec();
    let host = host.to_owned();
    Ok(async move { E::wait_for_nodes(&node_ports, &host, requirement).await })
}

fn write_static_compose_configs<E>(
    path: &Path,
    topology: &<E as Application>::Deployment,
) -> Result<(), DynError>
where
    E: ComposeBinaryApp,
{
    let hostnames = E::cfgsync_hostnames(topology);
    let configs_dir = stack_configs_dir(path)?;
    fs::create_dir_all(&configs_dir)?;

    for index in 0..topology.node_count() {
        let mut config = E::build_node_config(topology, index)?;
        E::rewrite_for_hostnames(topology, index, &hostnames, &mut config)?;
        let rendered = E::serialize_node_config(&config)?;
        fs::write(
            configs_dir.join(E::static_node_config_file_name(index)),
            rendered,
        )?;
    }

    Ok(())
}

/// Materializes cfgsync registration-server config files for a compose stack.
pub fn write_registration_server_compose_configs<E>(
    path: &Path,
    topology: &<E as Application>::Deployment,
    cfgsync_port: u16,
) -> Result<(), DynError>
where
    E: ComposeDeployEnv + StaticArtifactRenderer<Deployment = <E as Application>::Deployment>,
{
    let artifacts_path = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cfgsync path has no parent"))?
        .join("cfgsync.artifacts.yaml");
    let hostnames = E::cfgsync_hostnames(topology);

    render_and_write_registration_server::<E, _>(
        topology,
        &hostnames,
        RegistrationServerRenderOptions {
            port: Some(cfgsync_port),
            artifacts_path: Some("cfgsync.artifacts.yaml".to_owned()),
        },
        CfgsyncOutputPaths {
            config_path: path,
            artifacts_path: &artifacts_path,
        },
        |artifacts| E::enrich_cfgsync_artifacts(topology, artifacts).map_err(Into::into),
    )?;

    Ok(())
}

fn stack_configs_dir(cfgsync_path: &Path) -> Result<PathBuf, DynError> {
    let stack_dir = cfgsync_path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cfgsync path has no parent"))?;
    Ok(stack_dir.join("configs"))
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

/// Converts mapped compose ports into generic node access.
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

fn make_extension_node_config_file_name(extension: &str) -> ComposeNodeConfigFileName {
    match extension {
        "yaml" => ComposeNodeConfigFileName::FixedExtension("yaml"),
        "yml" => ComposeNodeConfigFileName::FixedExtension("yml"),
        "conf" => ComposeNodeConfigFileName::FixedExtension("conf"),
        "nats" => ComposeNodeConfigFileName::FixedExtension("nats"),
        other => {
            let leaked: &'static str = Box::leak(other.to_owned().into_boxed_str());
            ComposeNodeConfigFileName::FixedExtension(leaked)
        }
    }
}

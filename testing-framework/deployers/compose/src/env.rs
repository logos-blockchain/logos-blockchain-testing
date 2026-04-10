use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

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
        build_binary_config_node_descriptors_with_file_name,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ComposeReadinessProbe {
    Http { path: &'static str },
    Tcp,
}

pub type ComposeWorkspacePrep<E> =
    fn(&Path, &<E as Application>::Deployment, Option<&Url>) -> Result<(), DynError>;
pub type ComposeLoopbackSpecBuilder<E> =
    fn(&<E as Application>::Deployment, usize) -> Result<LoopbackNodeRuntimeSpec, DynError>;
pub type ComposeExtraServices<E> =
    fn(&<E as Application>::Deployment) -> Result<Vec<NodeDescriptor>, DynError>;
pub type ComposeDescriptorBuilder<E> =
    fn(&<E as Application>::Deployment, u16) -> Result<ComposeDescriptor, DynError>;
pub type ComposeNodeClientBuilder<E> =
    fn(&NodeHostPorts, &str) -> Result<<E as Application>::NodeClient, DynError>;
pub type ComposeContainerPortsResolver<E> = fn(
    &<E as Application>::Deployment,
    &ComposeDescriptor,
) -> Result<Vec<NodeContainerPorts>, DynError>;
pub type ComposeCfgsyncHostnames<E> = fn(&<E as Application>::Deployment) -> Vec<String>;
pub type ComposeCfgsyncEnricher<E> =
    fn(&<E as Application>::Deployment, &mut MaterializedArtifacts) -> Result<(), DynError>;
pub type ComposeConfigWriter<E> = for<'a> fn(ComposeConfigContext<'a, E>) -> Result<(), DynError>;
pub type ComposeConfigServerSpecBuilder =
    fn(&Path, u16, &str) -> Result<DockerConfigServerSpec, DynError>;
pub type ComposeRunnerHost = fn() -> String;

#[derive(Clone, Copy)]
pub enum ComposeNodeConfigFileName {
    FixedExtension(&'static str),
    Custom(fn(usize) -> String),
}

impl ComposeNodeConfigFileName {
    #[must_use]
    pub fn resolve(&self, index: usize) -> String {
        match self {
            Self::FixedExtension(extension) => format!("node-{index}.{extension}"),
            Self::Custom(build) => build(index),
        }
    }
}

pub trait ComposeDeployEnv: Application + Sized {
    fn compose_runtime() -> ComposeRuntime<Self>;
}

pub struct ComposeRuntime<E: Application> {
    stack: ComposeStack<E>,
    configs: ComposeConfigs<E>,
    access: ComposeAccess<E>,
    cfgsync: ComposeCfgsync<E>,
    node_config_file_name: ComposeNodeConfigFileName,
}

impl<E: Application> ComposeRuntime<E> {
    #[must_use]
    pub fn new(stack: ComposeStack<E>) -> Self {
        Self {
            stack,
            configs: ComposeConfigs::disabled(),
            access: ComposeAccess::default(),
            cfgsync: ComposeCfgsync::default(),
            node_config_file_name: ComposeNodeConfigFileName::FixedExtension("yaml"),
        }
    }

    #[must_use]
    pub fn binary_config(spec: BinaryConfigNodeSpec) -> Self {
        let node_config_file_name =
            make_extension_node_config_file_name(&spec.config_file_extension);
        Self {
            stack: ComposeStack::nodes(ComposeNodes::binary_config(spec)),
            configs: ComposeConfigs::disabled(),
            access: ComposeAccess::default(),
            cfgsync: ComposeCfgsync::default(),
            node_config_file_name,
        }
    }

    #[must_use]
    pub fn loopback(runtime_spec: ComposeLoopbackSpecBuilder<E>) -> Self {
        Self::new(ComposeStack::nodes(ComposeNodes::loopback(runtime_spec)))
    }

    #[must_use]
    pub fn custom_descriptor(build_descriptor: ComposeDescriptorBuilder<E>) -> Self {
        Self::new(ComposeStack::custom(build_descriptor))
    }

    #[must_use]
    pub fn with_node_config_file_name(
        mut self,
        node_config_file_name: ComposeNodeConfigFileName,
    ) -> Self {
        self.node_config_file_name = node_config_file_name;
        self
    }

    #[must_use]
    pub fn with_configs(mut self, configs: ComposeConfigs<E>) -> Self {
        self.configs = configs;
        self
    }

    #[must_use]
    pub fn with_static_configs(mut self) -> Self
    where
        E: ComposeDeployEnv,
        E: StaticArtifactRenderer<Deployment = <E as Application>::Deployment>,
    {
        self.configs = ComposeConfigs::static_node_configs();
        self
    }

    #[must_use]
    pub fn with_registration_server_configs(mut self) -> Self
    where
        E: ComposeDeployEnv,
        E: StaticArtifactRenderer<Deployment = <E as Application>::Deployment>,
    {
        self.configs = ComposeConfigs::registration_server();
        self
    }

    #[must_use]
    pub fn with_access(mut self, access: ComposeAccess<E>) -> Self {
        self.access = access;
        self
    }

    #[must_use]
    pub fn with_cfgsync(mut self, cfgsync: ComposeCfgsync<E>) -> Self {
        self.cfgsync = cfgsync;
        self
    }

    #[must_use]
    pub fn with_prepare_workspace(mut self, prepare_workspace: ComposeWorkspacePrep<E>) -> Self {
        self.stack = self.stack.with_prepare_workspace(prepare_workspace);
        self
    }

    #[must_use]
    pub fn with_extra_services(mut self, extra_services: ComposeExtraServices<E>) -> Self {
        self.stack = self.stack.with_extra_services(extra_services);
        self
    }
}

pub enum ComposeStack<E: Application> {
    Nodes(ComposeNodes<E>),
    Custom {
        build_descriptor: ComposeDescriptorBuilder<E>,
        prepare_workspace: Option<ComposeWorkspacePrep<E>>,
    },
}

impl<E: Application> ComposeStack<E> {
    #[must_use]
    pub fn nodes(nodes: ComposeNodes<E>) -> Self {
        Self::Nodes(nodes)
    }

    #[must_use]
    pub fn custom(build_descriptor: ComposeDescriptorBuilder<E>) -> Self {
        Self::Custom {
            build_descriptor,
            prepare_workspace: None,
        }
    }

    #[must_use]
    pub fn with_prepare_workspace(mut self, prepare_workspace: ComposeWorkspacePrep<E>) -> Self {
        match &mut self {
            Self::Nodes(nodes) => nodes.prepare_workspace = Some(prepare_workspace),
            Self::Custom {
                prepare_workspace: slot,
                ..
            } => *slot = Some(prepare_workspace),
        }
        self
    }

    #[must_use]
    pub fn with_extra_services(mut self, extra_services: ComposeExtraServices<E>) -> Self {
        if let Self::Nodes(nodes) = &mut self {
            nodes.extra_services = Some(extra_services);
        }
        self
    }

    fn prepare_workspace(
        &self,
        path: &Path,
        topology: &E::Deployment,
        metrics_otlp_ingest_url: Option<&Url>,
    ) -> Result<(), DynError> {
        match self {
            Self::Nodes(nodes) => nodes
                .prepare_workspace
                .map(|prepare| prepare(path, topology, metrics_otlp_ingest_url))
                .unwrap_or(Ok(())),
            Self::Custom {
                prepare_workspace, ..
            } => prepare_workspace
                .map(|prepare| prepare(path, topology, metrics_otlp_ingest_url))
                .unwrap_or(Ok(())),
        }
    }

    fn build_descriptor(
        &self,
        topology: &E::Deployment,
        cfgsync_port: u16,
        node_config_file_name: ComposeNodeConfigFileName,
    ) -> Result<ComposeDescriptor, DynError> {
        match self {
            Self::Nodes(nodes) => nodes.build_descriptor(topology, &node_config_file_name),
            Self::Custom {
                build_descriptor, ..
            } => build_descriptor(topology, cfgsync_port),
        }
    }
}

pub struct ComposeNodes<E: Application> {
    runtime: ComposeNodeRuntime<E>,
    extra_services: Option<ComposeExtraServices<E>>,
    prepare_workspace: Option<ComposeWorkspacePrep<E>>,
}

impl<E: Application> ComposeNodes<E> {
    #[must_use]
    pub fn binary_config(spec: BinaryConfigNodeSpec) -> Self {
        Self {
            runtime: ComposeNodeRuntime::BinaryConfig(spec),
            extra_services: None,
            prepare_workspace: None,
        }
    }

    #[must_use]
    pub fn loopback(runtime_spec: ComposeLoopbackSpecBuilder<E>) -> Self {
        Self {
            runtime: ComposeNodeRuntime::Loopback(runtime_spec),
            extra_services: None,
            prepare_workspace: None,
        }
    }

    fn build_descriptor(
        &self,
        topology: &E::Deployment,
        node_config_file_name: &ComposeNodeConfigFileName,
    ) -> Result<ComposeDescriptor, DynError> {
        let mut nodes = match &self.runtime {
            ComposeNodeRuntime::BinaryConfig(spec) => {
                build_binary_config_node_descriptors_with_file_name(
                    topology.node_count(),
                    spec,
                    |index| node_config_file_name.resolve(index),
                )
            }
            ComposeNodeRuntime::Loopback(build_runtime) => (0..topology.node_count())
                .map(|index| {
                    let spec = build_runtime(topology, index)?;
                    Ok(NodeDescriptor::with_loopback_ports(
                        crate::infrastructure::ports::node_identifier(index),
                        spec.image,
                        spec.entrypoint,
                        spec.volumes,
                        spec.extra_hosts,
                        spec.container_ports,
                        spec.environment,
                        spec.platform,
                    ))
                })
                .collect::<Result<Vec<_>, DynError>>()?,
        };

        if let Some(extra_services) = self.extra_services {
            nodes.extend(extra_services(topology)?);
        }

        Ok(ComposeDescriptor::new(nodes))
    }
}

enum ComposeNodeRuntime<E: Application> {
    BinaryConfig(BinaryConfigNodeSpec),
    Loopback(ComposeLoopbackSpecBuilder<E>),
}

pub struct ComposeConfigContext<'a, E: Application> {
    path: &'a Path,
    topology: &'a E::Deployment,
    cfgsync_port: u16,
    metrics_otlp_ingest_url: Option<&'a Url>,
    node_config_file_name: ComposeNodeConfigFileName,
}

impl<'a, E: Application> ComposeConfigContext<'a, E> {
    #[must_use]
    pub fn path(&self) -> &'a Path {
        self.path
    }

    #[must_use]
    pub fn topology(&self) -> &'a E::Deployment {
        self.topology
    }

    #[must_use]
    pub fn cfgsync_port(&self) -> u16 {
        self.cfgsync_port
    }

    #[must_use]
    pub fn metrics_otlp_ingest_url(&self) -> Option<&'a Url> {
        self.metrics_otlp_ingest_url
    }

    pub fn node_config_path(&self, index: usize) -> Result<PathBuf, DynError> {
        Ok(stack_configs_dir(self.path)?.join(self.node_config_file_name.resolve(index)))
    }
}

pub struct ComposeConfigs<E: Application> {
    writer: Option<ComposeConfigWriter<E>>,
}

impl<E: Application> ComposeConfigs<E> {
    #[must_use]
    pub fn disabled() -> Self {
        Self { writer: None }
    }

    #[must_use]
    pub fn custom(writer: ComposeConfigWriter<E>) -> Self {
        Self {
            writer: Some(writer),
        }
    }

    #[must_use]
    pub fn static_node_configs() -> Self
    where
        E: ComposeDeployEnv,
        E: StaticArtifactRenderer<Deployment = <E as Application>::Deployment>,
    {
        Self {
            writer: Some(write_static_compose_configs::<E>),
        }
    }

    #[must_use]
    pub fn registration_server() -> Self
    where
        E: ComposeDeployEnv,
        E: StaticArtifactRenderer<Deployment = <E as Application>::Deployment>,
    {
        Self {
            writer: Some(write_registration_server_compose_configs::<E>),
        }
    }

    fn write(
        &self,
        path: &Path,
        topology: &E::Deployment,
        cfgsync_port: u16,
        metrics_otlp_ingest_url: Option<&Url>,
        node_config_file_name: ComposeNodeConfigFileName,
    ) -> Result<(), DynError> {
        if let Some(writer) = self.writer {
            writer(ComposeConfigContext {
                path,
                topology,
                cfgsync_port,
                metrics_otlp_ingest_url,
                node_config_file_name,
            })?;
        }
        Ok(())
    }
}

pub struct ComposeAccess<E: Application> {
    container_ports: Option<ComposeContainerPortsResolver<E>>,
    node_client_from_ports: Option<ComposeNodeClientBuilder<E>>,
    readiness_probe: ComposeReadinessProbe,
    runner_host: ComposeRunnerHost,
}

impl<E: Application> Default for ComposeAccess<E> {
    fn default() -> Self {
        Self {
            container_ports: None,
            node_client_from_ports: None,
            readiness_probe: ComposeReadinessProbe::Http {
                path: E::node_readiness_path(),
            },
            runner_host: compose_runner_host,
        }
    }
}

impl<E: Application> ComposeAccess<E> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_container_ports(
        mut self,
        container_ports: ComposeContainerPortsResolver<E>,
    ) -> Self {
        self.container_ports = Some(container_ports);
        self
    }

    #[must_use]
    pub fn with_node_client(mut self, node_client_from_ports: ComposeNodeClientBuilder<E>) -> Self {
        self.node_client_from_ports = Some(node_client_from_ports);
        self
    }

    #[must_use]
    pub fn with_readiness_probe(mut self, readiness_probe: ComposeReadinessProbe) -> Self {
        self.readiness_probe = readiness_probe;
        self
    }

    #[must_use]
    pub fn with_runner_host(mut self, runner_host: ComposeRunnerHost) -> Self {
        self.runner_host = runner_host;
        self
    }

    fn node_container_ports(
        &self,
        topology: &E::Deployment,
        descriptor: &ComposeDescriptor,
    ) -> Result<Vec<NodeContainerPorts>, DynError> {
        if let Some(container_ports) = self.container_ports {
            return container_ports(topology, descriptor);
        }

        Ok(descriptor
            .nodes()
            .iter()
            .enumerate()
            .take(topology.node_count())
            .filter_map(|(index, node)| parse_node_container_ports(index, node))
            .collect())
    }

    fn node_client_from_ports(
        &self,
        ports: &NodeHostPorts,
        host: &str,
    ) -> Result<E::NodeClient, DynError> {
        if let Some(node_client_from_ports) = self.node_client_from_ports {
            return node_client_from_ports(ports, host);
        }

        <E as Application>::build_node_client(&discovered_node_access(host, ports))
    }

    fn build_node_clients(
        &self,
        _topology: &E::Deployment,
        host_ports: &HostPortMapping,
        host: &str,
    ) -> Result<NodeClients<E>, DynError> {
        let clients = host_ports
            .nodes
            .iter()
            .map(|ports| self.node_client_from_ports(ports, host))
            .collect::<Result<_, _>>()?;
        Ok(NodeClients::new(clients))
    }

    fn runner_host(&self) -> String {
        (self.runner_host)()
    }

    fn readiness_probe(&self) -> ComposeReadinessProbe {
        self.readiness_probe
    }
}

pub struct ComposeCfgsync<E: Application> {
    server_mode: ComposeConfigServerMode,
    hostnames: ComposeCfgsyncHostnames<E>,
    enrich_artifacts: Option<ComposeCfgsyncEnricher<E>>,
    container_spec: Option<ComposeConfigServerSpecBuilder>,
    start_timeout: Duration,
}

impl<E: Application> Default for ComposeCfgsync<E> {
    fn default() -> Self {
        Self {
            server_mode: ComposeConfigServerMode::Disabled,
            hostnames: default_cfgsync_hostnames::<E>,
            enrich_artifacts: None,
            container_spec: None,
            start_timeout: Duration::from_secs(180),
        }
    }
}

impl<E: Application> ComposeCfgsync<E> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_server_mode(mut self, server_mode: ComposeConfigServerMode) -> Self {
        self.server_mode = server_mode;
        self
    }

    #[must_use]
    pub fn with_hostnames(mut self, hostnames: ComposeCfgsyncHostnames<E>) -> Self {
        self.hostnames = hostnames;
        self
    }

    #[must_use]
    pub fn with_enrich_artifacts(mut self, enrich_artifacts: ComposeCfgsyncEnricher<E>) -> Self {
        self.enrich_artifacts = Some(enrich_artifacts);
        self
    }

    #[must_use]
    pub fn with_container_spec(mut self, container_spec: ComposeConfigServerSpecBuilder) -> Self {
        self.container_spec = Some(container_spec);
        self
    }

    #[must_use]
    pub fn with_start_timeout(mut self, start_timeout: Duration) -> Self {
        self.start_timeout = start_timeout;
        self
    }

    fn server_mode(&self) -> ComposeConfigServerMode {
        self.server_mode
    }

    fn hostnames(&self, topology: &E::Deployment) -> Vec<String> {
        (self.hostnames)(topology)
    }

    fn enrich_artifacts(
        &self,
        topology: &E::Deployment,
        artifacts: &mut MaterializedArtifacts,
    ) -> Result<(), DynError> {
        self.enrich_artifacts
            .map(|enrich| enrich(topology, artifacts))
            .unwrap_or(Ok(()))
    }

    fn container_spec(
        &self,
        cfgsync_path: &Path,
        port: u16,
        network: &str,
    ) -> Result<DockerConfigServerSpec, DynError> {
        let container_spec = self.container_spec.ok_or_else(|| {
            DynError::from(std::io::Error::other(
                "cfgsync container spec is not configured",
            ))
        })?;
        container_spec(cfgsync_path, port, network)
    }

    fn start_timeout(&self) -> Duration {
        self.start_timeout
    }
}

pub(crate) fn runtime_for<E: ComposeDeployEnv>() -> ComposeRuntime<E> {
    E::compose_runtime()
}

pub(crate) fn prepare_compose_configs<E: ComposeDeployEnv>(
    path: &Path,
    topology: &E::Deployment,
    cfgsync_port: u16,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<(), DynError> {
    let runtime = runtime_for::<E>();
    runtime
        .stack
        .prepare_workspace(path, topology, metrics_otlp_ingest_url)?;
    runtime.configs.write(
        path,
        topology,
        cfgsync_port,
        metrics_otlp_ingest_url,
        runtime.node_config_file_name,
    )?;
    Ok(())
}

pub(crate) fn compose_descriptor<E: ComposeDeployEnv>(
    topology: &E::Deployment,
    cfgsync_port: u16,
) -> Result<ComposeDescriptor, DynError> {
    let runtime = runtime_for::<E>();
    runtime
        .stack
        .build_descriptor(topology, cfgsync_port, runtime.node_config_file_name)
}

pub(crate) fn node_container_ports<E: ComposeDeployEnv>(
    topology: &E::Deployment,
) -> Result<Vec<NodeContainerPorts>, DynError> {
    let runtime = runtime_for::<E>();
    let descriptor = runtime
        .stack
        .build_descriptor(topology, 0, runtime.node_config_file_name)?;
    runtime.access.node_container_ports(topology, &descriptor)
}

pub(crate) fn cfgsync_hostnames<E: ComposeDeployEnv>(topology: &E::Deployment) -> Vec<String> {
    runtime_for::<E>().cfgsync.hostnames(topology)
}

pub(crate) fn enrich_cfgsync_artifacts<E: ComposeDeployEnv>(
    topology: &E::Deployment,
    artifacts: &mut MaterializedArtifacts,
) -> Result<(), DynError> {
    runtime_for::<E>()
        .cfgsync
        .enrich_artifacts(topology, artifacts)
}

pub(crate) fn cfgsync_container_spec<E: ComposeDeployEnv>(
    cfgsync_path: &Path,
    port: u16,
    network: &str,
) -> Result<DockerConfigServerSpec, DynError> {
    runtime_for::<E>()
        .cfgsync
        .container_spec(cfgsync_path, port, network)
}

pub(crate) fn cfgsync_start_timeout<E: ComposeDeployEnv>() -> Duration {
    runtime_for::<E>().cfgsync.start_timeout()
}

pub(crate) fn cfgsync_server_mode<E: ComposeDeployEnv>() -> ComposeConfigServerMode {
    runtime_for::<E>().cfgsync.server_mode()
}

pub(crate) fn readiness_http_path<E: ComposeDeployEnv>() -> &'static str {
    match runtime_for::<E>().access.readiness_probe() {
        ComposeReadinessProbe::Http { path } => path,
        ComposeReadinessProbe::Tcp => E::node_readiness_path(),
    }
}

pub(crate) fn build_node_clients<E: ComposeDeployEnv>(
    topology: &E::Deployment,
    host_ports: &HostPortMapping,
    host: &str,
) -> Result<NodeClients<E>, DynError> {
    runtime_for::<E>()
        .access
        .build_node_clients(topology, host_ports, host)
}

pub(crate) fn wait_remote_readiness<E: ComposeDeployEnv>(
    _topology: &E::Deployment,
    mapping: &HostPortMapping,
    requirement: HttpReadinessRequirement,
) -> Result<impl std::future::Future<Output = Result<(), DynError>>, DynError> {
    let runtime = runtime_for::<E>();
    let host = runtime.access.runner_host();
    let probe = runtime.access.readiness_probe();
    Ok(async move {
        match probe {
            ComposeReadinessProbe::Http { path } => {
                let urls = readiness_urls(&host, mapping, path)?;
                wait_http_readiness(&urls, requirement).await?;
                Ok(())
            }
            ComposeReadinessProbe::Tcp => wait_for_tcp_readiness(&mapping.nodes, requirement).await,
        }
    })
}

pub(crate) fn wait_for_nodes<E: ComposeDeployEnv>(
    ports: &[u16],
    host: &str,
    requirement: HttpReadinessRequirement,
) -> Result<impl std::future::Future<Output = Result<(), DynError>>, DynError> {
    let probe = runtime_for::<E>().access.readiness_probe();
    let host = host.to_owned();
    let node_ports = ports.to_vec();
    Ok(async move {
        match probe {
            ComposeReadinessProbe::Http { path } => {
                wait_for_http_ports_with_host_and_requirement(
                    &node_ports,
                    &host,
                    path,
                    requirement,
                )
                .await?;
                Ok(())
            }
            ComposeReadinessProbe::Tcp => {
                let ports = node_ports
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
    })
}

fn write_static_compose_configs<E>(context: ComposeConfigContext<'_, E>) -> Result<(), DynError>
where
    E: ComposeDeployEnv + StaticArtifactRenderer<Deployment = <E as Application>::Deployment>,
{
    let hostnames = cfgsync_hostnames::<E>(context.topology());
    let configs_dir = stack_configs_dir(context.path())?;
    fs::create_dir_all(&configs_dir)?;

    for index in 0..context.topology().node_count() {
        let mut config = E::build_node_config(context.topology(), index)?;
        E::rewrite_for_hostnames(context.topology(), index, &hostnames, &mut config)?;
        let rendered = E::serialize_node_config(&config)?;
        fs::write(context.node_config_path(index)?, rendered)?;
    }

    Ok(())
}

fn write_registration_server_compose_configs<E>(
    context: ComposeConfigContext<'_, E>,
) -> Result<(), DynError>
where
    E: ComposeDeployEnv + StaticArtifactRenderer<Deployment = <E as Application>::Deployment>,
{
    let stack_dir = context
        .path()
        .parent()
        .ok_or_else(|| anyhow::anyhow!("cfgsync path has no parent"))?;
    let artifacts_path = stack_dir.join("cfgsync.artifacts.yaml");
    let hostnames = cfgsync_hostnames::<E>(context.topology());

    render_and_write_registration_server::<E, _>(
        context.topology(),
        &hostnames,
        RegistrationServerRenderOptions {
            port: Some(context.cfgsync_port()),
            artifacts_path: Some("cfgsync.artifacts.yaml".to_owned()),
        },
        CfgsyncOutputPaths {
            config_path: context.path(),
            artifacts_path: &artifacts_path,
        },
        |artifacts| {
            enrich_cfgsync_artifacts::<E>(context.topology(), artifacts).map_err(Into::into)
        },
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

fn default_cfgsync_hostnames<E: Application>(topology: &E::Deployment) -> Vec<String> {
    (0..topology.node_count())
        .map(crate::infrastructure::ports::node_identifier)
        .collect()
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

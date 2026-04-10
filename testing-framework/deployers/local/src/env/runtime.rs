use std::{
    collections::HashMap,
    future::Future,
    path::{Path, PathBuf},
    pin::Pin,
};

use serde::Serialize;
use testing_framework_core::scenario::{
    Application, ClusterNodeConfigApplication, DynError, NodeAccess, StartNodeOptions,
};

use crate::{
    env::{
        BuiltNodeConfig, LocalNodePorts, LocalPeerNode, LocalProcessSpec, Node, NodeConfigEntry,
        NodeEndpoints, build_local_cluster_node_config, discovered_node_access, yaml_node_config,
    },
    process::{LaunchEnvVar, LaunchSpec, ProcessSpawnError},
};

pub struct LocalBuildContext<'a, E: Application> {
    pub topology: &'a E::Deployment,
    pub index: usize,
    pub ports: &'a LocalNodePorts,
    pub peers: &'a [LocalPeerNode],
    pub peer_ports: &'a [u16],
    pub peer_ports_by_name: &'a HashMap<String, u16>,
    pub options: &'a StartNodeOptions<E>,
    pub template_config: Option<&'a E::NodeConfig>,
}

pub type LocalConfigBuilder<E> =
    for<'a> fn(LocalBuildContext<'a, E>) -> Result<<E as Application>::NodeConfig, DynError>;

pub type LocalDynamicNodeBuilder<E> =
    for<'a> fn(
        LocalBuildContext<'a, E>,
    ) -> Result<BuiltNodeConfig<<E as Application>::NodeConfig>, DynError>;

pub type LocalConfigRenderer<E> = fn(&<E as Application>::NodeConfig) -> Result<Vec<u8>, DynError>;

pub type LocalInitialNodesBuilder<E> =
    fn(
        &<E as Application>::Deployment,
    ) -> Result<Vec<NodeConfigEntry<<E as Application>::NodeConfig>>, ProcessSpawnError>;

pub type LocalLaunchSpecBuilder<E> =
    fn(&<E as Application>::NodeConfig, &Path, &str) -> Result<LaunchSpec, DynError>;

pub type LocalApiPort<E> = fn(&<E as Application>::NodeConfig) -> u16;
pub type LocalEndpoints<E> = fn(&<E as Application>::NodeConfig) -> Result<NodeEndpoints, DynError>;
pub type LocalClientBuilder<E> =
    fn(&NodeAccess) -> Result<<E as Application>::NodeClient, DynError>;
pub type LocalPeerPort<E> = fn(&<E as Application>::NodeConfig, &NodeEndpoints) -> u16;
pub type LocalPersistDir<E> = fn(&<E as Application>::Deployment, &str, usize) -> Option<PathBuf>;
pub type LocalSnapshotDir<E> = fn(&<E as Application>::Deployment, &str, usize) -> Option<PathBuf>;
pub type LocalStableReadinessFuture<'a> =
    Pin<Box<dyn Future<Output = Result<(), DynError>> + Send + 'a>>;
pub type LocalStableReadiness<E> = for<'a> fn(&'a [Node<E>]) -> LocalStableReadinessFuture<'a>;

#[derive(Clone)]
enum LocalDynamicNode<E: Application> {
    Standard { build_config: LocalConfigBuilder<E> },
    Custom(LocalDynamicNodeBuilder<E>),
}

impl<E: Application> LocalDynamicNode<E> {
    fn build(
        &self,
        context: LocalBuildContext<'_, E>,
    ) -> Result<BuiltNodeConfig<E::NodeConfig>, DynError> {
        match self {
            Self::Standard { build_config } => {
                let network_port = context.ports.network_port();
                Ok(BuiltNodeConfig {
                    config: build_config(context)?,
                    network_port,
                })
            }
            Self::Custom(build) => build(context),
        }
    }
}

#[derive(Clone)]
enum LocalInitialNodes<E: Application> {
    Generated,
    Custom(LocalInitialNodesBuilder<E>),
}

#[derive(Clone)]
enum LocalLaunch<E: Application> {
    Rendered {
        spec: LocalProcessSpec,
        render_config: LocalConfigRenderer<E>,
    },
    Custom(LocalLaunchSpecBuilder<E>),
}

#[derive(Clone)]
pub struct LocalProcess<E: Application> {
    pub(crate) node_name_prefix: &'static str,
    pub(crate) port_names: &'static [&'static str],
    dynamic_node: LocalDynamicNode<E>,
    initial_nodes: LocalInitialNodes<E>,
    launch: LocalLaunch<E>,
}

impl<E: Application> LocalProcess<E> {
    #[must_use]
    pub fn new(
        binary_env_var: &'static str,
        binary_name: &'static str,
        build_config: LocalConfigBuilder<E>,
        render_config: LocalConfigRenderer<E>,
    ) -> Self {
        Self {
            node_name_prefix: "node",
            port_names: &[],
            dynamic_node: LocalDynamicNode::Standard { build_config },
            initial_nodes: LocalInitialNodes::Generated,
            launch: LocalLaunch::Rendered {
                spec: LocalProcessSpec::new(binary_env_var, binary_name),
                render_config,
            },
        }
    }

    #[must_use]
    pub fn custom(
        build_node: LocalDynamicNodeBuilder<E>,
        build_launch_spec: LocalLaunchSpecBuilder<E>,
    ) -> Self {
        Self {
            node_name_prefix: "node",
            port_names: &[],
            dynamic_node: LocalDynamicNode::Custom(build_node),
            initial_nodes: LocalInitialNodes::Generated,
            launch: LocalLaunch::Custom(build_launch_spec),
        }
    }

    #[must_use]
    pub fn with_node_name_prefix(mut self, value: &'static str) -> Self {
        self.node_name_prefix = value;
        self
    }

    #[must_use]
    pub fn with_port_names(mut self, value: &'static [&'static str]) -> Self {
        self.port_names = value;
        self
    }

    #[must_use]
    pub fn with_initial_nodes(mut self, build_initial_nodes: LocalInitialNodesBuilder<E>) -> Self {
        self.initial_nodes = LocalInitialNodes::Custom(build_initial_nodes);
        self
    }

    #[must_use]
    pub fn with_config_file(mut self, file_name: &str, arg: &str) -> Self {
        if let LocalLaunch::Rendered { spec, .. } = &mut self.launch {
            *spec = spec.clone().with_config_file(file_name, arg);
        }
        self
    }

    #[must_use]
    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        if let LocalLaunch::Rendered { spec, .. } = &mut self.launch {
            *spec = spec.clone().with_env(key, value);
        }
        self
    }

    #[must_use]
    pub fn with_rust_log(mut self, value: &str) -> Self {
        if let LocalLaunch::Rendered { spec, .. } = &mut self.launch {
            *spec = spec.clone().with_rust_log(value);
        }
        self
    }

    #[must_use]
    pub fn with_args(mut self, args: impl IntoIterator<Item = String>) -> Self {
        if let LocalLaunch::Rendered { spec, .. } = &mut self.launch {
            *spec = spec.clone().with_args(args);
        }
        self
    }

    #[must_use]
    pub fn with_launch_env(mut self, vars: impl IntoIterator<Item = LaunchEnvVar>) -> Self {
        if let LocalLaunch::Rendered { spec, .. } = &mut self.launch {
            spec.env.extend(vars);
        }
        self
    }

    pub(crate) fn build_node(
        &self,
        context: LocalBuildContext<'_, E>,
    ) -> Result<BuiltNodeConfig<E::NodeConfig>, DynError> {
        self.dynamic_node.build(context)
    }

    pub(crate) fn build_initial_nodes(
        &self,
        topology: &E::Deployment,
    ) -> Result<Vec<NodeConfigEntry<E::NodeConfig>>, ProcessSpawnError>
    where
        E::NodeConfig: Clone,
    {
        match self.initial_nodes {
            LocalInitialNodes::Generated => super::helpers::build_generated_initial_nodes::<E>(
                topology,
                self.node_name_prefix,
                self.port_names,
                |context| self.build_node(context),
            ),
            LocalInitialNodes::Custom(build) => build(topology),
        }
    }

    pub(crate) fn build_launch_spec(
        &self,
        config: &E::NodeConfig,
        dir: &Path,
        label: &str,
    ) -> Result<LaunchSpec, DynError> {
        match &self.launch {
            LocalLaunch::Rendered {
                spec,
                render_config,
            } => super::helpers::rendered_config_launch_spec(render_config(config)?, spec),
            LocalLaunch::Custom(build) => build(config, dir, label),
        }
    }
}

impl<E> LocalProcess<E>
where
    E: Application,
    E::NodeConfig: Serialize,
{
    #[must_use]
    pub fn yaml(
        binary_env_var: &'static str,
        binary_name: &'static str,
        build_config: LocalConfigBuilder<E>,
    ) -> Self {
        Self::new(
            binary_env_var,
            binary_name,
            build_config,
            yaml_node_config::<E::NodeConfig>,
        )
    }
}

#[derive(Clone)]
pub struct LocalAccess<E: Application> {
    api_port: Option<LocalApiPort<E>>,
    endpoints: Option<LocalEndpoints<E>>,
    client: Option<LocalClientBuilder<E>>,
    peer_port: Option<LocalPeerPort<E>>,
    readiness_path: &'static str,
}

impl<E: Application> LocalAccess<E> {
    #[must_use]
    pub fn http(api_port: LocalApiPort<E>) -> Self {
        Self {
            api_port: Some(api_port),
            endpoints: None,
            client: None,
            peer_port: None,
            readiness_path: E::node_readiness_path(),
        }
    }

    #[must_use]
    pub fn custom(endpoints: LocalEndpoints<E>) -> Self {
        Self {
            api_port: None,
            endpoints: Some(endpoints),
            client: None,
            peer_port: None,
            readiness_path: E::node_readiness_path(),
        }
    }

    #[must_use]
    pub fn with_client(mut self, client: LocalClientBuilder<E>) -> Self {
        self.client = Some(client);
        self
    }

    #[must_use]
    pub fn with_peer_port(mut self, peer_port: LocalPeerPort<E>) -> Self {
        self.peer_port = Some(peer_port);
        self
    }

    #[must_use]
    pub fn with_readiness_path(mut self, readiness_path: &'static str) -> Self {
        self.readiness_path = readiness_path;
        self
    }

    pub(crate) fn node_endpoints(&self, config: &E::NodeConfig) -> Result<NodeEndpoints, DynError> {
        if let Some(endpoints) = self.endpoints {
            return endpoints(config);
        }

        if let Some(api_port) = self.api_port {
            return Ok(NodeEndpoints::from_api_port(api_port(config)));
        }

        Err(std::io::Error::other("node endpoints are not configured").into())
    }

    pub(crate) fn node_client(&self, endpoints: &NodeEndpoints) -> Result<E::NodeClient, DynError> {
        if let Some(client) = self.client {
            return client(&discovered_node_access(endpoints));
        }

        E::build_node_client(&discovered_node_access(endpoints))
    }

    pub(crate) fn node_peer_port(&self, config: &E::NodeConfig, endpoints: &NodeEndpoints) -> u16 {
        self.peer_port
            .map(|peer_port| peer_port(config, endpoints))
            .unwrap_or_else(|| endpoints.api.port())
    }

    pub(crate) fn readiness_path(&self) -> &'static str {
        self.readiness_path
    }
}

#[derive(Clone)]
pub struct LocalLifecycle<E: Application> {
    initial_persist_dir: Option<LocalPersistDir<E>>,
    initial_snapshot_dir: Option<LocalSnapshotDir<E>>,
    stable_readiness: Option<LocalStableReadiness<E>>,
}

impl<E: Application> LocalLifecycle<E> {
    #[must_use]
    pub fn new() -> Self {
        Self {
            initial_persist_dir: None,
            initial_snapshot_dir: None,
            stable_readiness: None,
        }
    }

    #[must_use]
    pub fn with_initial_persist_dir(mut self, persist_dir: LocalPersistDir<E>) -> Self {
        self.initial_persist_dir = Some(persist_dir);
        self
    }

    #[must_use]
    pub fn with_initial_snapshot_dir(mut self, snapshot_dir: LocalSnapshotDir<E>) -> Self {
        self.initial_snapshot_dir = Some(snapshot_dir);
        self
    }

    #[must_use]
    pub fn with_stable_readiness(mut self, stable_readiness: LocalStableReadiness<E>) -> Self {
        self.stable_readiness = Some(stable_readiness);
        self
    }

    pub(crate) fn initial_persist_dir(
        &self,
        topology: &E::Deployment,
        node_name: &str,
        index: usize,
    ) -> Option<PathBuf> {
        self.initial_persist_dir
            .and_then(|persist_dir| persist_dir(topology, node_name, index))
    }

    pub(crate) fn initial_snapshot_dir(
        &self,
        topology: &E::Deployment,
        node_name: &str,
        index: usize,
    ) -> Option<PathBuf> {
        self.initial_snapshot_dir
            .and_then(|snapshot_dir| snapshot_dir(topology, node_name, index))
    }

    pub(crate) async fn wait_stable(&self, nodes: &[Node<E>]) -> Result<(), DynError> {
        if let Some(stable_readiness) = self.stable_readiness {
            return stable_readiness(nodes).await;
        }

        Ok(())
    }
}

#[derive(Clone)]
pub struct LocalRuntime<E: Application> {
    pub(crate) process: LocalProcess<E>,
    pub(crate) access: LocalAccess<E>,
    pub(crate) lifecycle: LocalLifecycle<E>,
}

impl<E: Application> LocalRuntime<E> {
    #[must_use]
    pub fn new(process: LocalProcess<E>, access: LocalAccess<E>) -> Self {
        Self {
            process,
            access,
            lifecycle: LocalLifecycle::new(),
        }
    }

    #[must_use]
    pub fn with_lifecycle(mut self, lifecycle: LocalLifecycle<E>) -> Self {
        self.lifecycle = lifecycle;
        self
    }
}

pub fn cluster_node_config_from_context<E>(
    context: LocalBuildContext<'_, E>,
) -> Result<<E as Application>::NodeConfig, DynError>
where
    E: Application + ClusterNodeConfigApplication,
{
    build_local_cluster_node_config::<E>(context.index, context.ports, context.peers)
}

use std::{
    collections::HashMap,
    net::{Ipv4Addr, SocketAddr},
};

use serde::Serialize;
use testing_framework_core::scenario::{
    Application, ClusterNodeConfigApplication, DynError, NodeAccess, StartNodeOptions,
};

use crate::{
    env::{
        LocalNodePorts, LocalPeerNode, LocalProcessSpec, NodeEndpoints, discovered_node_access,
        yaml_node_config,
    },
    process::LaunchEnvVar,
};

pub struct LocalBuildContext<'a, E: Application> {
    pub topology: &'a E::Deployment,
    pub index: usize,
    pub ports: &'a LocalNodePorts,
    pub peers: &'a [LocalPeerNode],
    pub peer_ports_by_name: &'a HashMap<String, u16>,
    pub options: &'a StartNodeOptions<E>,
    pub template_config: Option<&'a E::NodeConfig>,
}

pub type LocalConfigBuilder<E> =
    for<'a> fn(LocalBuildContext<'a, E>) -> Result<<E as Application>::NodeConfig, DynError>;

pub type LocalConfigRenderer<E> = fn(&<E as Application>::NodeConfig) -> Result<Vec<u8>, DynError>;

pub type LocalApiPort<E> = fn(&<E as Application>::NodeConfig) -> u16;

pub type LocalEndpoints<E> = fn(&<E as Application>::NodeConfig) -> Result<NodeEndpoints, DynError>;

pub type LocalClientBuilder<E> =
    fn(&NodeAccess) -> Result<<E as Application>::NodeClient, DynError>;

pub type LocalPeerPort<E> = fn(&<E as Application>::NodeConfig, &NodeEndpoints) -> u16;

#[derive(Clone)]
pub struct LocalProcess<E: Application> {
    pub(crate) spec: LocalProcessSpec,
    pub(crate) build_config: LocalConfigBuilder<E>,
    pub(crate) render_config: LocalConfigRenderer<E>,
    pub(crate) node_name_prefix: &'static str,
    pub(crate) port_names: &'static [&'static str],
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
            spec: LocalProcessSpec::new(binary_env_var, binary_name),
            build_config,
            render_config,
            node_name_prefix: "node",
            port_names: &[],
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
    pub fn with_config_file(mut self, file_name: &str, arg: &str) -> Self {
        self.spec = self.spec.with_config_file(file_name, arg);
        self
    }

    #[must_use]
    pub fn with_env(mut self, key: &str, value: &str) -> Self {
        self.spec = self.spec.with_env(key, value);
        self
    }

    #[must_use]
    pub fn with_rust_log(mut self, value: &str) -> Self {
        self.spec = self.spec.with_rust_log(value);
        self
    }

    #[must_use]
    pub fn with_args(mut self, args: impl IntoIterator<Item = String>) -> Self {
        self.spec = self.spec.with_args(args);
        self
    }

    #[must_use]
    pub fn with_launch_env(mut self, vars: impl IntoIterator<Item = LaunchEnvVar>) -> Self {
        self.spec.env.extend(vars);
        self
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
    pub(crate) api_port: Option<LocalApiPort<E>>,
    pub(crate) endpoints: Option<LocalEndpoints<E>>,
    pub(crate) client: Option<LocalClientBuilder<E>>,
    pub(crate) peer_port: Option<LocalPeerPort<E>>,
    pub(crate) readiness_path: &'static str,
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
            return Ok(NodeEndpoints {
                api: SocketAddr::from((Ipv4Addr::LOCALHOST, api_port(config))),
                extra_ports: HashMap::new(),
            });
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
}

#[derive(Clone)]
pub struct LocalRuntime<E: Application> {
    pub(crate) process: LocalProcess<E>,
    pub(crate) access: LocalAccess<E>,
}

impl<E: Application> LocalRuntime<E> {
    #[must_use]
    pub fn new(process: LocalProcess<E>, access: LocalAccess<E>) -> Self {
        Self { process, access }
    }
}

pub fn cluster_node_config_from_context<E>(
    context: LocalBuildContext<'_, E>,
) -> Result<<E as Application>::NodeConfig, DynError>
where
    E: Application + ClusterNodeConfigApplication,
{
    crate::env::build_local_cluster_node_config::<E>(context.index, context.ports, context.peers)
}

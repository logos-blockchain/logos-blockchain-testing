use std::{collections::HashMap, error::Error};

use super::ScenarioApplication;
use crate::{cfgsync::StaticNodeConfigProvider, topology::DeploymentDescriptor};

#[derive(Clone, Debug)]
pub struct ClusterPeerView {
    index: usize,
    host: String,
    network_port: u16,
}

impl ClusterPeerView {
    #[must_use]
    pub fn new(index: usize, host: impl Into<String>, network_port: u16) -> Self {
        Self {
            index,
            host: host.into(),
            network_port,
        }
    }

    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub fn network_port(&self) -> u16 {
        self.network_port
    }

    #[must_use]
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.network_port)
    }
}

#[derive(Clone, Debug)]
pub struct ClusterNodeView {
    index: usize,
    host: String,
    network_port: u16,
    named_ports: HashMap<&'static str, u16>,
}

impl ClusterNodeView {
    #[must_use]
    pub fn new(index: usize, host: impl Into<String>, network_port: u16) -> Self {
        Self {
            index,
            host: host.into(),
            network_port,
            named_ports: HashMap::new(),
        }
    }

    #[must_use]
    pub fn with_named_port(mut self, name: &'static str, port: u16) -> Self {
        self.named_ports.insert(name, port);
        self
    }

    #[must_use]
    pub fn index(&self) -> usize {
        self.index
    }

    #[must_use]
    pub fn host(&self) -> &str {
        &self.host
    }

    #[must_use]
    pub fn network_port(&self) -> u16 {
        self.network_port
    }

    pub fn require_named_port(&self, name: &str) -> Result<u16, std::io::Error> {
        self.named_ports
            .get(name)
            .copied()
            .ok_or_else(|| std::io::Error::other(format!("missing node port '{name}'")))
    }

    #[must_use]
    pub fn authority(&self) -> String {
        format!("{}:{}", self.host, self.network_port)
    }
}

pub trait ClusterNodeConfigApplication: ScenarioApplication {
    type ConfigError: Error + Send + Sync + 'static;

    fn static_network_port() -> u16;

    fn static_named_ports() -> &'static [(&'static str, u16)] {
        &[]
    }

    fn build_cluster_node_config(
        node: &ClusterNodeView,
        peers: &[ClusterPeerView],
    ) -> Result<Self::NodeConfig, Self::ConfigError>;

    fn serialize_cluster_node_config(
        config: &Self::NodeConfig,
    ) -> Result<String, Self::ConfigError>;
}

impl<T> StaticNodeConfigProvider for T
where
    T: ClusterNodeConfigApplication,
    T::Deployment: DeploymentDescriptor,
{
    type Error = T::ConfigError;

    fn build_node_config(
        deployment: &Self::Deployment,
        node_index: usize,
    ) -> Result<Self::NodeConfig, Self::Error> {
        build_static_cluster_node_config::<T>(deployment, node_index, None)
    }

    fn rewrite_for_hostnames(
        deployment: &Self::Deployment,
        node_index: usize,
        hostnames: &[String],
        config: &mut Self::NodeConfig,
    ) -> Result<(), Self::Error> {
        *config = build_static_cluster_node_config::<T>(deployment, node_index, Some(hostnames))?;
        Ok(())
    }

    fn serialize_node_config(config: &Self::NodeConfig) -> Result<String, Self::Error> {
        T::serialize_cluster_node_config(config)
    }
}

fn build_static_cluster_node_config<T>(
    deployment: &T::Deployment,
    node_index: usize,
    hostnames: Option<&[String]>,
) -> Result<T::NodeConfig, T::ConfigError>
where
    T: ClusterNodeConfigApplication,
    T::Deployment: DeploymentDescriptor,
{
    let node = static_node_view::<T>(node_index, hostnames);
    let peers = (0..deployment.node_count())
        .filter(|&i| i != node_index)
        .map(|i| static_peer_view::<T>(i, hostnames))
        .collect::<Vec<_>>();

    T::build_cluster_node_config(&node, &peers)
}

fn static_node_view<T>(node_index: usize, hostnames: Option<&[String]>) -> ClusterNodeView
where
    T: ClusterNodeConfigApplication,
{
    let host = hostnames
        .and_then(|names| names.get(node_index).cloned())
        .unwrap_or_else(|| format!("node-{node_index}"));
    let mut node = ClusterNodeView::new(node_index, host, T::static_network_port());
    for (name, port) in T::static_named_ports() {
        node = node.with_named_port(name, *port);
    }
    node
}

fn static_peer_view<T>(node_index: usize, hostnames: Option<&[String]>) -> ClusterPeerView
where
    T: ClusterNodeConfigApplication,
{
    let host = hostnames
        .and_then(|names| names.get(node_index).cloned())
        .unwrap_or_else(|| format!("node-{node_index}"));
    ClusterPeerView::new(node_index, host, T::static_network_port())
}

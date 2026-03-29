use std::{collections::HashMap, error::Error, io};

use cfgsync_artifacts::{ArtifactFile, ArtifactSet};

use crate::{
    cfgsync::StaticNodeConfigProvider,
    env::Application,
    scenario::{PeerSelection, StartNodeOptions},
    topology::DeploymentDescriptor,
};

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

pub trait ClusterNodeConfigApplication: Application {
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
    T::ConfigError: From<io::Error>,
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

    fn build_node_artifacts_for_options(
        deployment: &Self::Deployment,
        node_index: usize,
        hostnames: &[String],
        options: &StartNodeOptions<Self>,
    ) -> Result<Option<ArtifactSet>, Self::Error> {
        match &options.peers {
            PeerSelection::DefaultLayout => Ok(None),
            PeerSelection::None => {
                let config =
                    build_cluster_node_config_for_indices::<T>(node_index, hostnames, &[])?;
                let yaml = T::serialize_cluster_node_config(&config)?;
                Ok(Some(single_config_artifact(yaml)))
            }
            PeerSelection::Named(names) => {
                let indices = resolve_named_peer_indices::<T>(deployment, node_index, names)?;
                let config =
                    build_cluster_node_config_for_indices::<T>(node_index, hostnames, &indices)?;
                let yaml = T::serialize_cluster_node_config(&config)?;
                Ok(Some(single_config_artifact(yaml)))
            }
        }
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
    T::ConfigError: From<io::Error>,
{
    let node = static_node_view::<T>(node_index, hostnames);
    let peers = (0..deployment.node_count())
        .filter(|&i| i != node_index)
        .map(|i| static_peer_view::<T>(i, hostnames))
        .collect::<Vec<_>>();

    T::build_cluster_node_config(&node, &peers)
}

fn build_cluster_node_config_for_indices<T>(
    node_index: usize,
    hostnames: &[String],
    peer_indices: &[usize],
) -> Result<T::NodeConfig, T::ConfigError>
where
    T: ClusterNodeConfigApplication,
    T::ConfigError: From<io::Error>,
{
    let node = static_node_view::<T>(node_index, Some(hostnames));
    let peers = peer_indices
        .iter()
        .copied()
        .map(|index| static_peer_view::<T>(index, Some(hostnames)))
        .collect::<Vec<_>>();

    T::build_cluster_node_config(&node, &peers)
}

fn resolve_named_peer_indices<T>(
    deployment: &T::Deployment,
    node_index: usize,
    names: &[String],
) -> Result<Vec<usize>, T::ConfigError>
where
    T: ClusterNodeConfigApplication,
    T::Deployment: DeploymentDescriptor,
    T::ConfigError: From<io::Error>,
{
    let mut indices = Vec::with_capacity(names.len());

    for name in names {
        let Some(index) = parse_node_index(name) else {
            return Err(io::Error::other(format!("unknown peer name '{name}'")).into());
        };
        if index >= deployment.node_count() {
            return Err(io::Error::other(format!("peer index out of range for '{name}'")).into());
        }
        if index != node_index {
            indices.push(index);
        }
    }

    Ok(indices)
}

fn parse_node_index(name: &str) -> Option<usize> {
    name.strip_prefix("node-")?.parse().ok()
}

fn single_config_artifact(config_yaml: String) -> ArtifactSet {
    ArtifactSet::new(vec![ArtifactFile::new(
        "/config.yaml".to_string(),
        config_yaml,
    )])
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::{Application, DefaultFeed, DefaultFeedRuntime, NodeAccess, NodeClients};

    struct DummyClusterApp;

    #[async_trait::async_trait]
    impl Application for DummyClusterApp {
        type Deployment = crate::topology::ClusterTopology;
        type NodeClient = String;
        type NodeConfig = String;
        type FeedRuntime = DefaultFeedRuntime;

        fn build_node_client(
            _access: &NodeAccess,
        ) -> Result<Self::NodeClient, crate::scenario::DynError> {
            Ok("client".to_owned())
        }

        async fn prepare_feed(
            _node_clients: NodeClients<Self>,
        ) -> Result<(DefaultFeed, Self::FeedRuntime), crate::scenario::DynError> {
            crate::scenario::default_feed_result()
        }
    }

    impl ClusterNodeConfigApplication for DummyClusterApp {
        type ConfigError = io::Error;

        fn static_network_port() -> u16 {
            9000
        }

        fn build_cluster_node_config(
            node: &ClusterNodeView,
            peers: &[ClusterPeerView],
        ) -> Result<Self::NodeConfig, Self::ConfigError> {
            Ok(format!(
                "node={};peers={}",
                node.authority(),
                peers
                    .iter()
                    .map(ClusterPeerView::authority)
                    .collect::<Vec<_>>()
                    .join(",")
            ))
        }

        fn serialize_cluster_node_config(
            config: &Self::NodeConfig,
        ) -> Result<String, Self::ConfigError> {
            Ok(config.clone())
        }
    }

    #[test]
    fn cluster_app_builds_named_peer_override_artifacts() {
        let deployment = crate::topology::ClusterTopology::new(3);
        let hostnames = vec![
            "node-0.svc".to_owned(),
            "node-1.svc".to_owned(),
            "node-2.svc".to_owned(),
        ];
        let options =
            StartNodeOptions::<DummyClusterApp>::default().with_peers(PeerSelection::Named(vec![
                "node-0".to_owned(),
                "node-2".to_owned(),
            ]));

        let artifacts =
            DummyClusterApp::build_node_artifacts_for_options(&deployment, 1, &hostnames, &options)
                .expect("override artifacts")
                .expect("expected override");

        assert_eq!(artifacts.files.len(), 1);
        assert_eq!(artifacts.files[0].path, "/config.yaml");
        assert_eq!(
            artifacts.files[0].content,
            "node=node-1.svc:9000;peers=node-0.svc:9000,node-2.svc:9000"
        );
    }

    #[test]
    fn cluster_app_builds_empty_peer_override_artifacts() {
        let deployment = crate::topology::ClusterTopology::new(2);
        let hostnames = vec!["node-0.svc".to_owned(), "node-1.svc".to_owned()];
        let options =
            StartNodeOptions::<DummyClusterApp>::default().with_peers(PeerSelection::None);

        let artifacts =
            DummyClusterApp::build_node_artifacts_for_options(&deployment, 1, &hostnames, &options)
                .expect("override artifacts")
                .expect("expected override");

        assert_eq!(artifacts.files[0].content, "node=node-1.svc:9000;peers=");
    }
}

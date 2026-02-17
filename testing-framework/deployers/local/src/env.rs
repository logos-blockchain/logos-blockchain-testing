use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use testing_framework_core::scenario::{
    Application, DynError, HttpReadinessRequirement, ReadinessError, StartNodeOptions,
    wait_for_http_ports_with_requirement,
};

use crate::process::{LaunchSpec, NodeEndpoints, ProcessNode, ProcessSpawnError};

pub type Node<E> = ProcessNode<<E as Application>::NodeConfig, <E as Application>::NodeClient>;

pub struct BuiltNodeConfig<Config> {
    pub config: Config,
    pub network_port: u16,
}

pub struct NodeConfigEntry<NodeConfigValue> {
    pub name: String,
    pub config: NodeConfigValue,
}

#[async_trait::async_trait]
pub trait LocalDeployerEnv: Application + Sized
where
    <Self as Application>::NodeConfig: Clone + Send + Sync + 'static,
{
    fn build_node_config(
        topology: &Self::Deployment,
        index: usize,
        peer_ports_by_name: &HashMap<String, u16>,
        options: &StartNodeOptions<Self>,
        peer_ports: &[u16],
    ) -> Result<BuiltNodeConfig<<Self as Application>::NodeConfig>, DynError>;

    fn build_initial_node_configs(
        topology: &Self::Deployment,
    ) -> Result<Vec<NodeConfigEntry<<Self as Application>::NodeConfig>>, ProcessSpawnError>;

    fn initial_persist_dir(
        _topology: &Self::Deployment,
        _node_name: &str,
        _index: usize,
    ) -> Option<PathBuf> {
        None
    }

    fn build_launch_spec(
        config: &<Self as Application>::NodeConfig,
        dir: &Path,
        label: &str,
    ) -> Result<LaunchSpec, DynError>;

    fn node_endpoints(config: &<Self as Application>::NodeConfig) -> NodeEndpoints;

    fn node_peer_port(node: &Node<Self>) -> u16 {
        node.endpoints().api.port()
    }

    fn node_client(endpoints: &NodeEndpoints) -> Self::NodeClient;

    fn readiness_endpoint_path() -> &'static str {
        "/"
    }

    async fn wait_readiness_stable(_nodes: &[Node<Self>]) -> Result<(), DynError> {
        Ok(())
    }
}

pub async fn wait_local_http_readiness<E: LocalDeployerEnv>(
    nodes: &[Node<E>],
    requirement: HttpReadinessRequirement,
) -> Result<(), ReadinessError> {
    let ports: Vec<_> = nodes
        .iter()
        .map(|node| node.endpoints().api.port())
        .collect();
    wait_for_http_ports_with_requirement(&ports, E::readiness_endpoint_path(), requirement).await?;

    E::wait_readiness_stable(nodes)
        .await
        .map_err(|source| ReadinessError::ClusterStable { source })
}

pub async fn spawn_node_from_config<E: LocalDeployerEnv>(
    label: String,
    config: <E as Application>::NodeConfig,
    keep_tempdir: bool,
    persist_dir: Option<&std::path::Path>,
) -> Result<Node<E>, ProcessSpawnError> {
    ProcessNode::spawn(
        &label,
        config,
        E::build_launch_spec,
        E::node_endpoints,
        keep_tempdir,
        persist_dir,
        E::node_client,
    )
    .await
}

#[cfg(test)]
mod tests {
    use std::{
        path::Path,
        sync::atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use testing_framework_core::{
        scenario::{Application, DynError, Feed, FeedRuntime},
        topology::DeploymentDescriptor,
    };

    use super::*;

    static STABLE_CALLS: AtomicUsize = AtomicUsize::new(0);

    #[derive(Clone)]
    struct DummyFeed;

    impl Feed for DummyFeed {
        type Subscription = ();

        fn subscribe(&self) -> Self::Subscription {}
    }

    struct DummyFeedRuntime;

    #[async_trait]
    impl FeedRuntime for DummyFeedRuntime {
        type Feed = DummyFeed;

        async fn run(self: Box<Self>) {}
    }

    #[derive(Clone)]
    struct DummyConfig;

    #[derive(Clone)]
    struct DummyTopology;

    impl DeploymentDescriptor for DummyTopology {
        fn node_count(&self) -> usize {
            0
        }
    }

    struct DummyEnv;

    #[async_trait]
    impl Application for DummyEnv {
        type Deployment = DummyTopology;
        type NodeClient = ();
        type NodeConfig = DummyConfig;
        type FeedRuntime = DummyFeedRuntime;

        async fn prepare_feed(
            _client: Self::NodeClient,
        ) -> Result<(<Self::FeedRuntime as FeedRuntime>::Feed, Self::FeedRuntime), DynError>
        {
            Ok((DummyFeed, DummyFeedRuntime))
        }
    }

    #[async_trait]
    impl LocalDeployerEnv for DummyEnv {
        fn build_node_config(
            _topology: &Self::Deployment,
            _index: usize,
            _peer_ports_by_name: &HashMap<String, u16>,
            _options: &StartNodeOptions<Self>,
            _peer_ports: &[u16],
        ) -> Result<BuiltNodeConfig<<Self as Application>::NodeConfig>, DynError> {
            unreachable!("not used in this test")
        }

        fn build_initial_node_configs(
            _topology: &Self::Deployment,
        ) -> Result<Vec<NodeConfigEntry<<Self as Application>::NodeConfig>>, ProcessSpawnError>
        {
            unreachable!("not used in this test")
        }

        fn build_launch_spec(
            _config: &<Self as Application>::NodeConfig,
            _dir: &Path,
            _label: &str,
        ) -> Result<LaunchSpec, DynError> {
            Ok(LaunchSpec::default())
        }

        fn node_endpoints(_config: &<Self as Application>::NodeConfig) -> NodeEndpoints {
            NodeEndpoints::default()
        }

        fn node_client(_endpoints: &NodeEndpoints) -> Self::NodeClient {}

        async fn wait_readiness_stable(_nodes: &[Node<Self>]) -> Result<(), DynError> {
            STABLE_CALLS.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[tokio::test]
    async fn empty_cluster_still_runs_stability_hook() {
        STABLE_CALLS.store(0, Ordering::SeqCst);
        let nodes: Vec<Node<DummyEnv>> = Vec::new();
        wait_local_http_readiness::<DummyEnv>(&nodes, HttpReadinessRequirement::AllNodesReady)
            .await
            .expect("empty cluster should be considered ready");
        assert_eq!(STABLE_CALLS.load(Ordering::SeqCst), 1);
    }
}

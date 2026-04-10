use std::{
    path::Path,
    sync::atomic::{AtomicUsize, Ordering},
};

use async_trait::async_trait;
use testing_framework_core::{
    scenario::{Application, DynError, Feed, FeedRuntime, HttpReadinessRequirement, NodeClients},
    topology::DeploymentDescriptor,
};

use super::*;

static STABLE_CALLS: AtomicUsize = AtomicUsize::new(0);

#[derive(Clone, Default)]
struct DummyFeed;

impl Feed for DummyFeed {
    type Subscription = ();

    fn subscribe(&self) -> Self::Subscription {}
}

#[derive(Default)]
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
        _node_clients: NodeClients<Self>,
    ) -> Result<(<Self::FeedRuntime as FeedRuntime>::Feed, Self::FeedRuntime), DynError> {
        Ok((DummyFeed, DummyFeedRuntime))
    }
}

#[async_trait]
impl LocalDeployerEnv for DummyEnv {
    fn build_node_config(
        _topology: &Self::Deployment,
        _index: usize,
        _peer_ports_by_name: &std::collections::HashMap<String, u16>,
        _options: &StartNodeOptions<Self>,
        _peer_ports: &[u16],
    ) -> Result<BuiltNodeConfig<<Self as Application>::NodeConfig>, DynError> {
        unreachable!("not used in this test")
    }

    fn build_initial_node_configs(
        _topology: &Self::Deployment,
    ) -> Result<Vec<NodeConfigEntry<<Self as Application>::NodeConfig>>, ProcessSpawnError> {
        unreachable!("not used in this test")
    }

    fn build_launch_spec(
        _config: &<Self as Application>::NodeConfig,
        _dir: &Path,
        _label: &str,
    ) -> Result<crate::process::LaunchSpec, DynError> {
        Ok(crate::process::LaunchSpec::default())
    }

    fn node_endpoints(
        _config: &<Self as Application>::NodeConfig,
    ) -> Result<NodeEndpoints, DynError> {
        Ok(NodeEndpoints::default())
    }

    fn node_client(_endpoints: &NodeEndpoints) -> Result<Self::NodeClient, DynError> {
        Ok(())
    }

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

use std::sync::atomic::{AtomicUsize, Ordering};

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

#[async_trait::async_trait]
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

#[async_trait::async_trait]
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

impl LocalDeployerEnv for DummyEnv {
    fn local_runtime() -> LocalRuntime<Self> {
        LocalRuntime::new(
            LocalProcess::custom(build_dummy_node, build_dummy_launch_spec)
                .with_initial_nodes(build_dummy_initial_nodes),
            LocalAccess::custom(dummy_endpoints).with_client(|_| Ok(())),
        )
        .with_lifecycle(LocalLifecycle::new().with_stable_readiness(dummy_wait_stable))
    }
}

fn build_dummy_node(
    _context: LocalBuildContext<'_, DummyEnv>,
) -> Result<BuiltNodeConfig<DummyConfig>, DynError> {
    unreachable!("not used in this test")
}

fn build_dummy_initial_nodes(
    _topology: &DummyTopology,
) -> Result<Vec<NodeConfigEntry<DummyConfig>>, crate::process::ProcessSpawnError> {
    unreachable!("not used in this test")
}

fn build_dummy_launch_spec(
    _config: &DummyConfig,
    _dir: &std::path::Path,
    _label: &str,
) -> Result<crate::process::LaunchSpec, DynError> {
    Ok(crate::process::LaunchSpec::default())
}

fn dummy_endpoints(_config: &DummyConfig) -> Result<NodeEndpoints, DynError> {
    Ok(NodeEndpoints::default())
}

fn dummy_wait_stable<'a>(_nodes: &'a [Node<DummyEnv>]) -> runtime::LocalStableReadinessFuture<'a> {
    Box::pin(async {
        STABLE_CALLS.fetch_add(1, Ordering::SeqCst);
        Ok(())
    })
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

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use async_trait::async_trait;
pub use lb_framework::*;
use reqwest::Url;
pub use scenario::{
    CoreBuilderExt, ObservabilityBuilderExt, ScenarioBuilder, ScenarioBuilderExt,
    ScenarioBuilderWith,
};
use testing_framework_core::scenario::{
    Application, DynError, ExternalNodeSource, FeedRuntime, NodeClients, RunContext,
    StartNodeOptions,
};
use testing_framework_runner_local::{
    BuiltNodeConfig, LocalDeployerEnv, NodeConfigEntry,
    process::{LaunchSpec, NodeEndpoints, ProcessSpawnError},
};
use tokio::sync::broadcast;
use workloads::{LbcBlockFeedEnv, LbcScenarioEnv};

pub mod cfgsync;
mod compose_env;
pub mod constants;
mod k8s_env;
pub mod scenario;

pub type LbcComposeDeployer = testing_framework_runner_compose::ComposeDeployer<LbcExtEnv>;
pub type LbcK8sDeployer = testing_framework_runner_k8s::K8sDeployer<LbcExtEnv>;

pub struct LbcExtEnv;

#[async_trait]
impl Application for LbcExtEnv {
    type Deployment = <LbcEnv as Application>::Deployment;

    type NodeClient = <LbcEnv as Application>::NodeClient;

    type NodeConfig = <LbcEnv as Application>::NodeConfig;

    type FeedRuntime = <LbcEnv as Application>::FeedRuntime;

    fn external_node_client(source: &ExternalNodeSource) -> Result<Self::NodeClient, DynError> {
        let base_url = Url::parse(&source.endpoint)?;
        Ok(NodeHttpClient::from_urls(base_url, None))
    }

    async fn prepare_feed(
        node_clients: NodeClients<Self>,
    ) -> Result<(<Self::FeedRuntime as FeedRuntime>::Feed, Self::FeedRuntime), DynError> {
        let clients = node_clients.snapshot();
        let upstream_clients = NodeClients::<lb_framework::LbcEnv>::new(clients);

        <LbcEnv as Application>::prepare_feed(upstream_clients).await
    }
}

impl LbcScenarioEnv for LbcExtEnv {}

impl LbcBlockFeedEnv for LbcExtEnv {
    fn block_feed_subscription(ctx: &RunContext<Self>) -> broadcast::Receiver<Arc<BlockRecord>> {
        ctx.feed().subscribe()
    }

    fn block_feed(ctx: &RunContext<Self>) -> BlockFeed {
        ctx.feed()
    }
}

#[async_trait]
impl LocalDeployerEnv for LbcExtEnv {
    fn build_node_config(
        topology: &Self::Deployment,
        index: usize,
        peer_ports_by_name: &HashMap<String, u16>,
        options: &StartNodeOptions<Self>,
        peer_ports: &[u16],
    ) -> Result<BuiltNodeConfig<<Self as Application>::NodeConfig>, DynError> {
        let mapped_options = map_start_options(options)?;
        <LbcEnv as LocalDeployerEnv>::build_node_config(
            topology,
            index,
            peer_ports_by_name,
            &mapped_options,
            peer_ports,
        )
    }

    fn build_initial_node_configs(
        topology: &Self::Deployment,
    ) -> Result<Vec<NodeConfigEntry<<Self as Application>::NodeConfig>>, ProcessSpawnError> {
        <LbcEnv as LocalDeployerEnv>::build_initial_node_configs(topology)
    }

    fn initial_persist_dir(
        topology: &Self::Deployment,
        node_name: &str,
        index: usize,
    ) -> Option<PathBuf> {
        <LbcEnv as LocalDeployerEnv>::initial_persist_dir(topology, node_name, index)
    }

    fn build_launch_spec(
        config: &<Self as Application>::NodeConfig,
        dir: &Path,
        label: &str,
    ) -> Result<LaunchSpec, DynError> {
        <LbcEnv as LocalDeployerEnv>::build_launch_spec(config, dir, label)
    }

    fn node_endpoints(config: &<Self as Application>::NodeConfig) -> NodeEndpoints {
        <LbcEnv as LocalDeployerEnv>::node_endpoints(config)
    }

    fn node_client(endpoints: &NodeEndpoints) -> Self::NodeClient {
        <LbcEnv as LocalDeployerEnv>::node_client(endpoints)
    }

    fn readiness_endpoint_path() -> &'static str {
        <LbcEnv as LocalDeployerEnv>::readiness_endpoint_path()
    }
}

fn map_start_options(
    options: &StartNodeOptions<LbcExtEnv>,
) -> Result<StartNodeOptions<LbcEnv>, DynError> {
    if options.config_patch.is_some() {
        return Err("LbcExtEnv local deployer bridge does not support config_patch yet".into());
    }

    let mut mapped = StartNodeOptions::<LbcEnv>::default();
    mapped.peers = options.peers.clone();
    mapped.config_override = options.config_override.clone();
    mapped.persist_dir = options.persist_dir.clone();

    Ok(mapped)
}

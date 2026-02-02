use std::sync::Arc;

use async_trait::async_trait;
pub use lb_framework::*;
use testing_framework_core::scenario::{Application, DynError, FeedRuntime, RunContext};
use tokio::sync::broadcast;

pub mod cfgsync;
mod compose_env;
pub mod constants;
mod k8s_env;
pub mod scenario;

pub struct LbcExtEnv;

#[async_trait]
impl Application for LbcExtEnv {
    type Deployment = <lb_framework::LbcEnv as Application>::Deployment;
    type NodeClient = <lb_framework::LbcEnv as Application>::NodeClient;
    type NodeConfig = <lb_framework::LbcEnv as Application>::NodeConfig;
    type FeedRuntime = <lb_framework::LbcEnv as Application>::FeedRuntime;

    async fn prepare_feed(
        client: Self::NodeClient,
    ) -> Result<(<Self::FeedRuntime as FeedRuntime>::Feed, Self::FeedRuntime), DynError> {
        <lb_framework::LbcEnv as Application>::prepare_feed(client).await
    }
}

pub use scenario::{
    CoreBuilderExt, ObservabilityBuilderExt, ScenarioBuilder, ScenarioBuilderExt,
    ScenarioBuilderWith,
};

pub type LbcComposeDeployer = testing_framework_runner_compose::ComposeDeployer<LbcExtEnv>;
pub type LbcK8sDeployer = testing_framework_runner_k8s::K8sDeployer<LbcExtEnv>;

impl lb_framework::workloads::LbcScenarioEnv for LbcExtEnv {}

impl lb_framework::workloads::LbcBlockFeedEnv for LbcExtEnv {
    fn block_feed_subscription(
        ctx: &RunContext<Self>,
    ) -> broadcast::Receiver<Arc<lb_framework::BlockRecord>> {
        ctx.feed().subscribe()
    }
}

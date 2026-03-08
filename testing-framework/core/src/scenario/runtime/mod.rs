pub mod context;
mod deployer;
mod inventory;
pub mod metrics;
mod node_clients;
pub mod orchestration;
pub mod providers;
pub mod readiness;
mod runner;

use async_trait::async_trait;
pub use context::{CleanupGuard, RunContext, RunHandle, RunMetrics};
pub use deployer::{Deployer, ScenarioError};
pub use node_clients::NodeClients;
#[doc(hidden)]
pub use orchestration::{
    ManagedSource, SourceOrchestrationPlan, build_source_orchestration_plan, orchestrate_sources,
    orchestrate_sources_with_providers, resolve_sources,
};
#[doc(hidden)]
pub use providers::{
    ApplicationExternalProvider, AttachProvider, AttachProviderError, AttachedNode,
    SourceProviders, StaticManagedProvider,
};
pub use readiness::{
    HttpReadinessRequirement, ReadinessError, StabilizationConfig, wait_for_http_ports,
    wait_for_http_ports_with_host, wait_for_http_ports_with_host_and_requirement,
    wait_for_http_ports_with_requirement, wait_http_readiness, wait_until_stable,
};
pub use runner::Runner;
use tokio::task::JoinHandle;

use crate::{env::Application, scenario::DynError};

/// Cloneable feed handle exposed to workloads and expectations.
pub trait Feed: Clone + Send + Sync + 'static {
    type Subscription: Send + 'static;

    fn subscribe(&self) -> Self::Subscription;
}

/// Background worker driving a cluster feed.
#[async_trait]
pub trait FeedRuntime: Send + 'static {
    type Feed: Feed;

    async fn run(self: Box<Self>);
}

/// Cleanup guard for a spawned feed worker.
pub struct FeedHandle {
    handle: JoinHandle<()>,
}

impl FeedHandle {
    pub const fn new(handle: JoinHandle<()>) -> Self {
        Self { handle }
    }
}

impl CleanupGuard for FeedHandle {
    fn cleanup(self: Box<Self>) {
        self.handle.abort();
    }
}

/// Spawn a background task that drives the environment-provided feed.
pub async fn spawn_feed<E: Application>(
    node_clients: NodeClients<E>,
) -> Result<(<E::FeedRuntime as FeedRuntime>::Feed, FeedHandle), DynError> {
    let (feed, worker) = E::prepare_feed(node_clients).await?;

    let handle = tokio::spawn(async move {
        Box::new(worker).run().await;
    });

    Ok((feed, FeedHandle::new(handle)))
}

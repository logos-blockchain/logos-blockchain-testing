use std::{sync::Arc, time::Duration};

use super::{metrics::Metrics, node_clients::ClusterClient};
use crate::scenario::{Application, ClusterWaitHandle, DynError, NodeClients, NodeControlHandle};

#[derive(Debug, thiserror::Error)]
enum RunContextCapabilityError {
    #[error("wait_network_ready is not available for this runner")]
    MissingClusterWait,
}

/// Shared runtime context available to workloads and expectations.
pub struct RunContext<E: Application> {
    descriptors: E::Deployment,
    node_clients: NodeClients<E>,
    metrics: RunMetrics,
    expectation_cooldown: Duration,
    telemetry: Metrics,
    feed: <E::FeedRuntime as super::FeedRuntime>::Feed,
    node_control: Option<Arc<dyn NodeControlHandle<E>>>,
    cluster_wait: Option<Arc<dyn ClusterWaitHandle<E>>>,
}

/// Low-level runtime assembly input used by deployers to build a runnable
/// cluster context.
pub struct RuntimeAssembly<E: Application> {
    descriptors: E::Deployment,
    node_clients: NodeClients<E>,
    run_duration: Duration,
    expectation_cooldown: Duration,
    telemetry: Metrics,
    feed: <E::FeedRuntime as super::FeedRuntime>::Feed,
    node_control: Option<Arc<dyn NodeControlHandle<E>>>,
    cluster_wait: Option<Arc<dyn ClusterWaitHandle<E>>>,
}

impl<E: Application> RunContext<E> {
    /// Builds a run context from prepared deployment/runtime artifacts.
    #[must_use]
    pub(crate) fn new(
        descriptors: E::Deployment,
        node_clients: NodeClients<E>,
        run_duration: Duration,
        expectation_cooldown: Duration,
        telemetry: Metrics,
        feed: <E::FeedRuntime as super::FeedRuntime>::Feed,
        node_control: Option<Arc<dyn NodeControlHandle<E>>>,
    ) -> Self {
        let metrics = RunMetrics::new(run_duration);

        Self {
            descriptors,
            node_clients,
            metrics,
            expectation_cooldown,
            telemetry,
            feed,
            node_control,
            cluster_wait: None,
        }
    }

    #[must_use]
    pub(crate) fn with_cluster_wait(mut self, cluster_wait: Arc<dyn ClusterWaitHandle<E>>) -> Self {
        self.cluster_wait = Some(cluster_wait);
        self
    }

    #[must_use]
    pub fn descriptors(&self) -> &E::Deployment {
        &self.descriptors
    }

    #[must_use]
    pub const fn node_clients(&self) -> &NodeClients<E> {
        &self.node_clients
    }

    #[must_use]
    pub fn random_node_client(&self) -> Option<E::NodeClient> {
        self.node_clients.random_client()
    }

    #[must_use]
    pub fn feed(&self) -> <E::FeedRuntime as super::FeedRuntime>::Feed {
        self.feed.clone()
    }

    #[must_use]
    pub const fn telemetry(&self) -> &Metrics {
        &self.telemetry
    }

    #[must_use]
    pub const fn run_duration(&self) -> Duration {
        self.metrics.run_duration()
    }

    #[must_use]
    pub(crate) const fn expectation_cooldown(&self) -> Duration {
        self.expectation_cooldown
    }

    #[must_use]
    pub fn node_control(&self) -> Option<Arc<dyn NodeControlHandle<E>>> {
        self.node_control.clone()
    }

    #[must_use]
    pub(crate) const fn controls_nodes(&self) -> bool {
        self.node_control.is_some()
    }

    pub(crate) async fn wait_network_ready(&self) -> Result<(), DynError> {
        self.require_cluster_wait()?.wait_network_ready().await
    }

    #[must_use]
    pub const fn cluster_client(&self) -> ClusterClient<'_, E> {
        self.node_clients.cluster_client()
    }

    fn require_cluster_wait(&self) -> Result<Arc<dyn ClusterWaitHandle<E>>, DynError> {
        self.cluster_wait
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(|| RunContextCapabilityError::MissingClusterWait.into())
    }
}

impl<E: Application> RuntimeAssembly<E> {
    #[must_use]
    pub fn new(
        descriptors: E::Deployment,
        node_clients: NodeClients<E>,
        run_duration: Duration,
        expectation_cooldown: Duration,
        telemetry: Metrics,
        feed: <E::FeedRuntime as super::FeedRuntime>::Feed,
    ) -> Self {
        Self {
            descriptors,
            node_clients,
            run_duration,
            expectation_cooldown,
            telemetry,
            feed,
            node_control: None,
            cluster_wait: None,
        }
    }

    #[must_use]
    pub fn with_node_control(mut self, node_control: Arc<dyn NodeControlHandle<E>>) -> Self {
        self.node_control = Some(node_control);
        self
    }

    #[must_use]
    pub fn with_cluster_wait(mut self, cluster_wait: Arc<dyn ClusterWaitHandle<E>>) -> Self {
        self.cluster_wait = Some(cluster_wait);
        self
    }

    #[must_use]
    pub fn build_context(self) -> RunContext<E> {
        let context = RunContext::new(
            self.descriptors,
            self.node_clients,
            self.run_duration,
            self.expectation_cooldown,
            self.telemetry,
            self.feed,
            self.node_control,
        );

        match self.cluster_wait {
            Some(cluster_wait) => context.with_cluster_wait(cluster_wait),
            None => context,
        }
    }

    #[must_use]
    pub fn build_runner(self, cleanup_guard: Option<Box<dyn CleanupGuard>>) -> super::Runner<E> {
        super::Runner::new(self.build_context(), cleanup_guard)
    }
}

impl<E: Application> From<RunContext<E>> for RuntimeAssembly<E> {
    fn from(context: RunContext<E>) -> Self {
        Self {
            descriptors: context.descriptors,
            node_clients: context.node_clients,
            run_duration: context.metrics.run_duration(),
            expectation_cooldown: context.expectation_cooldown,
            telemetry: context.telemetry,
            feed: context.feed,
            node_control: context.node_control,
            cluster_wait: context.cluster_wait,
        }
    }
}

/// Handle returned by the runner to control the lifecycle of the run.
pub struct RunHandle<E: Application> {
    run_context: Arc<RunContext<E>>,
    cleanup_guard: Option<Box<dyn CleanupGuard>>,
}

impl<E: Application> Drop for RunHandle<E> {
    fn drop(&mut self) {
        if let Some(guard) = self.cleanup_guard.take() {
            guard.cleanup();
        }
    }
}

impl<E: Application> RunHandle<E> {
    #[must_use]
    /// Build a handle from a shared context reference.
    pub(crate) fn from_shared(
        context: Arc<RunContext<E>>,
        cleanup_guard: Option<Box<dyn CleanupGuard>>,
    ) -> Self {
        Self {
            run_context: context,
            cleanup_guard,
        }
    }

    #[must_use]
    /// Access the shared run context.
    pub fn context(&self) -> &RunContext<E> {
        &self.run_context
    }
}

/// Derived metrics about the current run timing.
#[derive(Clone, Copy)]
pub struct RunMetrics {
    run_duration: Duration,
}

impl RunMetrics {
    #[must_use]
    pub const fn new(run_duration: Duration) -> Self {
        Self { run_duration }
    }

    #[must_use]
    pub const fn run_duration(&self) -> Duration {
        self.run_duration
    }
}

pub trait CleanupGuard: Send {
    fn cleanup(self: Box<Self>);
}

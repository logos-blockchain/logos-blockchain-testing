use std::time::Duration;

use queue_runtime_ext::{QueueEnv, QueueTopology};
use testing_framework_app::{
    AppHost, AppHostDeployer, AppHostEnv, AppHostScenarioBuilder, AppScenarioBuilderExt as _,
    ClusterApp, ClusterRestartChaos,
};
use testing_framework_core::scenario::{
    DynError,
    internal::{CoreBuilder, CoreBuilderAccess},
};

use crate::{QueueConverges, QueueProduceWorkload};

/// Entry point for the queue verb DSL.
pub struct QueueScenario;

impl QueueScenario {
    #[must_use]
    pub fn nodes(count: usize) -> QueueScenarioBuilder {
        QueueScenarioBuilder {
            inner: AppHost::scenario(),
            nodes: count,
        }
    }
}

/// App-host scenario builder that remembers the requested queue node count.
///
/// The queue cluster itself is attached as a [`ClusterApp`] by the
/// [`QueueRunExt::run_secs`] finisher.
pub struct QueueScenarioBuilder {
    inner: AppHostScenarioBuilder,
    nodes: usize,
}

impl CoreBuilderAccess for QueueScenarioBuilder {
    type Env = AppHostEnv;
    type Caps = ();

    fn map_core_builder(
        mut self,
        f: impl FnOnce(CoreBuilder<AppHostEnv>) -> CoreBuilder<AppHostEnv>,
    ) -> Self {
        self.inner = self.inner.map_core_builder(f);
        self
    }

    fn core_builder_ref(&self) -> &CoreBuilder<AppHostEnv> {
        self.inner.core_builder_ref()
    }

    fn core_builder_mut(&mut self) -> &mut CoreBuilder<AppHostEnv> {
        self.inner.core_builder_mut()
    }
}

/// Queue domain verbs available on every scenario builder over
/// [`AppHostEnv`].
///
/// Verbs only expand: each sub-builder lowers to `with_workload` /
/// `with_expectation` calls with the corresponding noun object.
pub trait QueueDslExt: CoreBuilderAccess<Env = AppHostEnv> + Sized {
    /// Enqueue `operations` payloads through the first node.
    #[must_use]
    fn produce(self, operations: usize) -> QueueProduceBuilder<Self> {
        QueueProduceBuilder {
            builder: self,
            workload: QueueProduceWorkload::new().operations(operations),
        }
    }

    /// Expect all nodes to agree on a queue of at least `min_queue_len`.
    #[must_use]
    fn expect_converged(self, min_queue_len: usize) -> QueueConvergedBuilder<Self> {
        QueueConvergedBuilder {
            builder: self,
            expectation: QueueConverges::new(min_queue_len),
        }
    }

    /// Randomly restart queue cluster nodes throughout the run.
    #[must_use]
    fn restart_nodes_randomly(self) -> QueueRestartChaosBuilder<Self> {
        QueueRestartChaosBuilder {
            builder: self,
            chaos: ClusterRestartChaos::new(),
        }
    }
}

impl<B: CoreBuilderAccess<Env = AppHostEnv>> QueueDslExt for B {}

pub struct QueueProduceBuilder<B: CoreBuilderAccess<Env = AppHostEnv>> {
    builder: B,
    workload: QueueProduceWorkload,
}

impl<B: CoreBuilderAccess<Env = AppHostEnv>> QueueProduceBuilder<B> {
    #[must_use]
    pub fn rate_per_sec(mut self, value: usize) -> Self {
        self.workload = self.workload.rate_per_sec(value);
        self
    }

    #[must_use]
    pub fn payload_prefix(mut self, value: impl Into<String>) -> Self {
        self.workload = self.workload.payload_prefix(value);
        self
    }

    #[must_use]
    pub fn done(self) -> B {
        let Self { builder, workload } = self;
        builder.map_core_builder(|inner| inner.with_workload(workload))
    }
}

pub struct QueueConvergedBuilder<B: CoreBuilderAccess<Env = AppHostEnv>> {
    builder: B,
    expectation: QueueConverges,
}

impl<B: CoreBuilderAccess<Env = AppHostEnv>> QueueConvergedBuilder<B> {
    #[must_use]
    pub fn within_secs(self, secs: u64) -> B {
        self.within(Duration::from_secs(secs))
    }

    #[must_use]
    pub fn within(self, timeout: Duration) -> B {
        let Self {
            builder,
            expectation,
        } = self;
        builder.map_core_builder(|inner| inner.with_expectation(expectation.timeout(timeout)))
    }
}

pub struct QueueRestartChaosBuilder<B: CoreBuilderAccess<Env = AppHostEnv>> {
    builder: B,
    chaos: ClusterRestartChaos<QueueEnv>,
}

impl<B: CoreBuilderAccess<Env = AppHostEnv>> QueueRestartChaosBuilder<B> {
    #[must_use]
    pub fn every_secs(mut self, min: u64, max: u64) -> Self {
        self.chaos = self.chaos.every_secs(min, max);
        self
    }

    #[must_use]
    pub fn every(mut self, min: Duration, max: Duration) -> Self {
        self.chaos = self.chaos.every(min, max);
        self
    }

    #[must_use]
    pub fn cooldown_secs(mut self, secs: u64) -> Self {
        self.chaos = self.chaos.cooldown_secs(secs);
        self
    }

    #[must_use]
    pub fn cooldown(mut self, cooldown: Duration) -> Self {
        self.chaos = self.chaos.cooldown(cooldown);
        self
    }

    #[must_use]
    pub fn excluding_nodes(mut self, nodes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.chaos = self.chaos.excluding_nodes(nodes);
        self
    }

    #[must_use]
    pub fn done(self) -> B {
        let Self { builder, chaos } = self;
        builder.map_core_builder(|inner| inner.with_workload(chaos))
    }
}

/// Finisher: attach the queue cluster app, set the run duration, build the
/// scenario, and run it through the backend-neutral app-host deployer.
pub trait QueueRunExt: Sized {
    fn run_secs(self, secs: u64) -> impl Future<Output = Result<(), DynError>> + Send;
}

impl QueueRunExt for QueueScenarioBuilder {
    async fn run_secs(self, secs: u64) -> Result<(), DynError> {
        let Self { inner, nodes } = self;
        let mut scenario = inner
            .with_app(ClusterApp::<QueueEnv>::new(QueueTopology::new(nodes)))
            .with_run_duration(Duration::from_secs(secs))
            .build()
            .map_err(DynError::from)?;

        let runner = AppHostDeployer.deploy(&scenario).await?;
        runner.run(&mut scenario).await?;
        Ok(())
    }
}

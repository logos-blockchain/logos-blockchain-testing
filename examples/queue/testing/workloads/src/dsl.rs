use std::time::Duration;

use queue_runtime_ext::{QueueEnv, QueueLocalDeployer, QueueScenarioBuilder, QueueTopology};
use testing_framework_core::scenario::{
    Deployer, DynError,
    internal::{CoreBuilderAccess, NodeControlScenarioBuilder},
};

use crate::{QueueConverges, QueueProduceWorkload};

/// Entry point for the queue verb DSL.
pub struct QueueScenario;

impl QueueScenario {
    #[must_use]
    pub fn nodes(count: usize) -> QueueScenarioBuilder {
        QueueScenarioBuilder::with_deployment(QueueTopology::new(count))
    }
}

/// Queue domain verbs available on every scenario builder over [`QueueEnv`].
///
/// Verbs only expand: each sub-builder lowers to `with_workload` /
/// `with_expectation` calls with the corresponding noun object.
pub trait QueueDslExt: CoreBuilderAccess<Env = QueueEnv> + Sized {
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
}

impl<B: CoreBuilderAccess<Env = QueueEnv>> QueueDslExt for B {}

pub struct QueueProduceBuilder<B: CoreBuilderAccess<Env = QueueEnv>> {
    builder: B,
    workload: QueueProduceWorkload,
}

impl<B: CoreBuilderAccess<Env = QueueEnv>> QueueProduceBuilder<B> {
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

pub struct QueueConvergedBuilder<B: CoreBuilderAccess<Env = QueueEnv>> {
    builder: B,
    expectation: QueueConverges,
}

impl<B: CoreBuilderAccess<Env = QueueEnv>> QueueConvergedBuilder<B> {
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

/// Finisher: set the run duration, build the scenario, and run it against the
/// local process deployer.
pub trait QueueRunExt: Sized {
    fn run_secs(self, secs: u64) -> impl Future<Output = Result<(), DynError>> + Send;
}

impl QueueRunExt for QueueScenarioBuilder {
    async fn run_secs(self, secs: u64) -> Result<(), DynError> {
        let scenario = self
            .with_run_duration(Duration::from_secs(secs))
            .build()
            .map_err(DynError::from)?;
        run_local(scenario).await
    }
}

impl QueueRunExt for NodeControlScenarioBuilder<QueueEnv> {
    async fn run_secs(self, secs: u64) -> Result<(), DynError> {
        let scenario = self
            .with_run_duration(Duration::from_secs(secs))
            .build()
            .map_err(DynError::from)?;
        run_local(scenario).await
    }
}

async fn run_local<Caps>(
    mut scenario: testing_framework_core::scenario::Scenario<QueueEnv, Caps>,
) -> Result<(), DynError>
where
    Caps: Send + Sync,
    QueueLocalDeployer: Deployer<QueueEnv, Caps>,
    <QueueLocalDeployer as Deployer<QueueEnv, Caps>>::Error: Into<DynError>,
{
    let deployer = QueueLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await.map_err(Into::into)?;
    runner.run(&mut scenario).await?;
    Ok(())
}

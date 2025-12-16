use std::{any::Any, panic::AssertUnwindSafe, sync::Arc, time::Duration};

use futures::FutureExt as _;
use tokio::{
    task::JoinSet,
    time::{sleep, timeout},
};

use super::deployer::ScenarioError;
use crate::scenario::{
    DynError, Expectation, Scenario,
    runtime::context::{CleanupGuard, RunContext, RunHandle},
};

type WorkloadOutcome = Result<(), DynError>;

const COOLDOWN_BLOCK_INTERVAL_MULTIPLIER: f64 = 5.0;
const MIN_NODE_CONTROL_COOLDOWN: Duration = Duration::from_secs(30);
const DEFAULT_BLOCK_FEED_SETTLE_WAIT: Duration = Duration::from_secs(1);
const MIN_BLOCK_FEED_SETTLE_WAIT: Duration = Duration::from_secs(2);

/// Represents a fully prepared environment capable of executing a scenario.
pub struct Runner {
    context: Arc<RunContext>,
    cleanup_guard: Option<Box<dyn CleanupGuard>>,
}

impl Runner {
    /// Construct a runner from the run context and optional cleanup guard.
    #[must_use]
    pub fn new(context: RunContext, cleanup_guard: Option<Box<dyn CleanupGuard>>) -> Self {
        Self {
            context: Arc::new(context),
            cleanup_guard,
        }
    }

    /// Access the underlying run context.
    #[must_use]
    pub fn context(&self) -> Arc<RunContext> {
        Arc::clone(&self.context)
    }

    pub(crate) fn cleanup(&mut self) {
        if let Some(guard) = self.cleanup_guard.take() {
            guard.cleanup();
        }
    }

    pub(crate) fn into_run_handle(mut self) -> RunHandle {
        RunHandle::from_shared(Arc::clone(&self.context), self.cleanup_guard.take())
    }

    /// Executes the scenario by driving workloads first and then evaluating all
    /// expectations. On any failure it cleans up resources and propagates the
    /// error to the caller.
    pub async fn run<Caps>(
        mut self,
        scenario: &mut Scenario<Caps>,
    ) -> Result<RunHandle, ScenarioError>
    where
        Caps: Send + Sync,
    {
        let context = self.context();
        if let Err(error) =
            Self::prepare_expectations(scenario.expectations_mut(), context.as_ref()).await
        {
            self.cleanup();
            return Err(error);
        }

        if let Err(error) = Self::run_workloads(&context, scenario).await {
            self.cleanup();
            return Err(error);
        }

        Self::settle_before_expectations(&context).await;

        if let Err(error) =
            Self::run_expectations(scenario.expectations_mut(), context.as_ref()).await
        {
            self.cleanup();
            return Err(error);
        }

        Ok(self.into_run_handle())
    }

    async fn prepare_expectations(
        expectations: &mut [Box<dyn Expectation>],
        context: &RunContext,
    ) -> Result<(), ScenarioError> {
        for expectation in expectations {
            if let Err(source) = expectation.start_capture(context).await {
                return Err(ScenarioError::ExpectationCapture(source));
            }
        }
        Ok(())
    }

    /// Spawns every workload, waits until the configured duration elapses (or a
    /// workload fails), and then aborts the remaining tasks.
    async fn run_workloads<Caps>(
        context: &Arc<RunContext>,
        scenario: &Scenario<Caps>,
    ) -> Result<(), ScenarioError>
    where
        Caps: Send + Sync,
    {
        let mut workloads = Self::spawn_workloads(scenario, context);
        let _ = Self::drive_until_timer(&mut workloads, scenario.duration()).await?;

        // Keep workloads running during the cooldown window so that late
        // inclusions (especially DA parent-linked ops) still have a chance to
        // land before expectations evaluate. We still abort everything at the
        // end of cooldown to prevent leaking tasks across runs.
        if let Some(cooldown) = Self::cooldown_duration(context.as_ref()) {
            if !cooldown.is_zero() {
                if workloads.is_empty() {
                    sleep(cooldown).await;
                } else {
                    let _ = Self::drive_until_timer(&mut workloads, cooldown).await?;
                }
            }
        }

        Self::drain_workloads(&mut workloads).await
    }

    async fn settle_before_expectations(context: &Arc<RunContext>) {
        // `BlockFeed` polls node storage on an interval. After we abort workloads
        // we give the feed a moment to catch up with the last blocks that might
        // include workload operations so expectations evaluate on a more stable
        // snapshot.
        let has_node_control = context.node_control().is_some();
        let hint = context.run_metrics().block_interval_hint();
        if !has_node_control && hint.is_none() {
            return;
        }

        let mut wait = hint.unwrap_or(DEFAULT_BLOCK_FEED_SETTLE_WAIT);
        wait = wait.max(MIN_BLOCK_FEED_SETTLE_WAIT);
        sleep(wait).await;
    }

    /// Evaluates every registered expectation, aggregating failures so callers
    /// can see all missing conditions in a single report.
    async fn run_expectations(
        expectations: &mut [Box<dyn Expectation>],
        context: &RunContext,
    ) -> Result<(), ScenarioError> {
        let mut failures: Vec<(String, DynError)> = Vec::new();
        for expectation in expectations {
            if let Err(source) = expectation.evaluate(context).await {
                failures.push((expectation.name().to_owned(), source));
            }
        }

        if failures.is_empty() {
            return Ok(());
        }

        let summary = failures
            .into_iter()
            .map(|(name, source)| format!("{name}: {source}"))
            .collect::<Vec<_>>()
            .join("\n");

        Err(ScenarioError::Expectations(summary.into()))
    }

    fn cooldown_duration(context: &RunContext) -> Option<Duration> {
        let metrics = context.run_metrics();
        let needs_stabilization = context.node_control().is_some();
        if let Some(interval) = metrics.block_interval_hint() {
            if interval.is_zero() {
                return None;
            }
            let mut wait = interval.mul_f64(COOLDOWN_BLOCK_INTERVAL_MULTIPLIER);
            // Expectations observe blocks via `BlockFeed`, which ultimately
            // follows the chain information returned by `consensus_info`.
            // When the consensus uses a security parameter (finality depth),
            // newly included operations can take ~k blocks to show up in the
            // observable chain. Short smoke runs otherwise end up evaluating
            // before finality catches up, systematically failing inclusion
            // expectations (especially for DA, where ops are parent-linked).
            let security_param = context
                .descriptors()
                .config()
                .consensus_params
                .security_param;
            wait = wait.max(interval.mul_f64(security_param.get() as f64));
            if needs_stabilization {
                wait = wait.max(MIN_NODE_CONTROL_COOLDOWN);
            }
            Some(wait)
        } else if needs_stabilization {
            Some(MIN_NODE_CONTROL_COOLDOWN)
        } else {
            None
        }
    }

    /// Spawns each workload inside its own task and returns the join set for
    /// cooperative management.
    fn spawn_workloads<Caps>(
        scenario: &Scenario<Caps>,
        context: &Arc<RunContext>,
    ) -> JoinSet<WorkloadOutcome>
    where
        Caps: Send + Sync,
    {
        let mut workloads = JoinSet::new();
        for workload in scenario.workloads() {
            let workload = Arc::clone(workload);
            let ctx = Arc::clone(context);

            workloads.spawn(async move {
                let outcome = AssertUnwindSafe(async { workload.start(ctx.as_ref()).await })
                    .catch_unwind()
                    .await;

                outcome.unwrap_or_else(|panic| {
                    Err(format!("workload panicked: {}", panic_message(panic)).into())
                })
            });
        }

        workloads
    }

    /// Polls workload tasks until the timeout fires or one reports an error.
    async fn drive_until_timer(
        workloads: &mut JoinSet<WorkloadOutcome>,
        duration: Duration,
    ) -> Result<bool, ScenarioError> {
        let run_future = async {
            while let Some(result) = workloads.join_next().await {
                Self::map_join_result(result)?;
            }
            Ok(())
        };

        timeout(duration, run_future)
            .await
            .map_or(Ok(true), |result| {
                result?;
                Ok(false)
            })
    }

    /// Aborts and drains any remaining workload tasks so we do not leak work
    /// across scenario runs.
    async fn drain_workloads(
        workloads: &mut JoinSet<WorkloadOutcome>,
    ) -> Result<(), ScenarioError> {
        workloads.abort_all();

        while let Some(result) = workloads.join_next().await {
            Self::map_join_result(result)?;
        }

        Ok(())
    }

    /// Converts the outcome of a workload task into the canonical scenario
    /// error, tolerating cancellation when the runner aborts unfinished tasks.
    fn map_join_result(
        result: Result<WorkloadOutcome, tokio::task::JoinError>,
    ) -> Result<(), ScenarioError> {
        match result {
            Ok(outcome) => outcome.map_err(ScenarioError::Workload),
            Err(join_err) if join_err.is_cancelled() => Ok(()),
            Err(join_err) => Err(ScenarioError::Workload(
                format!("workload task failed: {join_err}").into(),
            )),
        }
    }
}

/// Attempts to turn a panic payload into a readable string for diagnostics.
fn panic_message(panic: Box<dyn Any + Send>) -> String {
    panic.downcast::<String>().map_or_else(
        |panic| {
            panic.downcast::<&'static str>().map_or_else(
                |_| "unknown panic".to_owned(),
                |message| (*message).to_owned(),
            )
        },
        |message| *message,
    )
}

impl Drop for Runner {
    fn drop(&mut self) {
        self.cleanup();
    }
}

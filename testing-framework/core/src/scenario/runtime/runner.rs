use std::{any::Any, future::Future, panic::AssertUnwindSafe, sync::Arc, time::Duration};

use futures::FutureExt as _;
use tokio::{
    task::{JoinError, JoinSet},
    time::{Interval, interval, sleep},
};
use tracing::{debug, info, warn};

use super::deployer::ScenarioError;
use crate::scenario::{
    Application, DynError, Expectation, Scenario, Workload,
    runtime::context::{CleanupGuard, RunContext, RunHandle},
};

type WorkloadOutcome = Result<(), DynError>;

const MIN_NODE_CONTROL_COOLDOWN: Duration = Duration::from_secs(30);
const DEFAULT_BLOCK_FEED_SETTLE_WAIT: Duration = Duration::from_secs(1);
const MIN_BLOCK_FEED_SETTLE_WAIT: Duration = Duration::from_secs(2);
const EXPECTATION_CAPTURE_CHECK_INTERVAL: Duration = Duration::from_secs(1);
const UNKNOWN_PANIC: &str = "<unknown panic>";

/// Represents a fully prepared environment capable of executing a scenario.
pub struct Runner<E: Application> {
    context: Arc<RunContext<E>>,
    cleanup_guard: Option<Box<dyn CleanupGuard>>,
}

impl<E: Application> Drop for Runner<E> {
    fn drop(&mut self) {
        self.cleanup();
    }
}

impl<E: Application> Runner<E> {
    #[must_use]
    pub(crate) fn new(
        context: RunContext<E>,
        cleanup_guard: Option<Box<dyn CleanupGuard>>,
    ) -> Self {
        Self {
            context: Arc::new(context),
            cleanup_guard,
        }
    }

    /// Access the underlying run context.
    #[must_use]
    pub fn context(&self) -> &RunContext<E> {
        self.context.as_ref()
    }

    pub async fn wait_network_ready(&self) -> Result<(), DynError> {
        self.context.wait_network_ready().await
    }

    pub(crate) fn cleanup(&mut self) {
        if let Some(guard) = self.cleanup_guard.take() {
            guard.cleanup();
        }
    }

    pub(crate) fn into_run_handle(mut self) -> RunHandle<E> {
        RunHandle::from_shared(Arc::clone(&self.context), self.cleanup_guard.take())
    }

    /// Execute workloads and evaluate expectations.
    pub async fn run<Caps>(
        mut self,
        scenario: &mut Scenario<E, Caps>,
    ) -> Result<RunHandle<E>, ScenarioError>
    where
        Caps: Send + Sync,
    {
        let context = Arc::clone(&self.context);
        let run_duration = scenario.duration();
        let workloads = scenario.workloads().to_vec();
        let expectation_count = scenario.expectations().len();

        info!(
            run_secs = run_duration.as_secs(),
            workloads = workloads.len(),
            expectations = expectation_count,
            "runner starting scenario execution"
        );

        self.run_step(Self::prepare_expectations(
            scenario.expectations_mut(),
            context.as_ref(),
        ))
        .await?;

        self.run_step(Self::run_workload_phase(
            Arc::clone(&context),
            &workloads,
            run_duration,
            scenario.expectations_mut(),
        ))
        .await?;

        Self::settle_before_expectations(context.as_ref()).await;

        self.run_step(Self::run_expectations(
            scenario.expectations_mut(),
            context.as_ref(),
        ))
        .await?;

        info!("runner finished scenario execution");

        Ok(self.into_run_handle())
    }

    async fn run_step(
        &mut self,
        step: impl Future<Output = Result<(), ScenarioError>>,
    ) -> Result<(), ScenarioError> {
        match step.await {
            Ok(()) => Ok(()),
            Err(error) => {
                self.cleanup();
                Err(error)
            }
        }
    }

    async fn prepare_expectations(
        expectations: &mut [Box<dyn Expectation<E>>],
        context: &RunContext<E>,
    ) -> Result<(), ScenarioError> {
        for expectation in expectations {
            expectation
                .start_capture(context)
                .await
                .map_err(ScenarioError::ExpectationCapture)?;
        }

        Ok(())
    }

    async fn run_workload_phase(
        context: Arc<RunContext<E>>,
        workloads: &[Arc<dyn Workload<E>>],
        duration: Duration,
        expectations: &mut [Box<dyn Expectation<E>>],
    ) -> Result<(), ScenarioError> {
        info!(
            workloads = workloads.len(),
            run_secs = duration.as_secs(),
            "runner workload phase started"
        );

        if workloads.is_empty() {
            Self::run_idle_window_with_capture_checks(duration, expectations, context.as_ref())
                .await?;

            info!("runner workload phase completed (idle)");

            return Ok(());
        }

        let mut running = Self::spawn_workloads(workloads, Arc::clone(&context));

        Self::run_window_until_timeout(&mut running, duration, expectations, context.as_ref())
            .await?;

        if let Some(cooldown) = nonzero_cooldown(Self::cooldown_duration(context.as_ref())) {
            info!(
                cooldown_secs = cooldown.as_secs(),
                "runner cooldown window started"
            );

            Self::run_window_until_timeout(&mut running, cooldown, expectations, context.as_ref())
                .await?;
        }

        Self::drain_workloads(&mut running).await?;

        info!("runner workload phase completed");

        Ok(())
    }

    async fn settle_before_expectations(context: &RunContext<E>) {
        // Give the feed a short catch-up window before evaluating expectations.
        let Some(wait) = Self::settle_wait_duration(context) else {
            return;
        };

        sleep(wait).await;
    }

    fn settle_wait_duration(context: &RunContext<E>) -> Option<Duration> {
        let control_profile = context.cluster_control_profile();
        let configured_wait = context.expectation_cooldown();

        if configured_wait.is_zero() && !control_profile.supports_node_control() {
            return None;
        }

        let wait = if configured_wait.is_zero() {
            DEFAULT_BLOCK_FEED_SETTLE_WAIT
        } else {
            configured_wait
        };

        Some(wait.max(MIN_BLOCK_FEED_SETTLE_WAIT))
    }

    /// Evaluates every registered expectation, aggregating failures so callers
    /// can see all missing conditions in a single report.
    async fn run_expectations(
        expectations: &mut [Box<dyn Expectation<E>>],
        context: &RunContext<E>,
    ) -> Result<(), ScenarioError> {
        let mut failures = Vec::new();
        for expectation in expectations {
            if let Err(source) = expectation.evaluate(context).await {
                failures.push((expectation.name().to_owned(), source));
            }
        }

        if failures.is_empty() {
            return Ok(());
        }

        Err(ScenarioError::Expectations(
            expectation_failure_summary(failures).into(),
        ))
    }

    fn cooldown_duration(context: &RunContext<E>) -> Option<Duration> {
        // Managed environments need a minimum cooldown so feed and expectations
        // observe stabilized state.
        let needs_stabilization = context.cluster_control_profile().framework_owns_lifecycle();

        let mut wait = context.expectation_cooldown();

        if wait.is_zero() {
            return needs_stabilization.then_some(MIN_NODE_CONTROL_COOLDOWN);
        }

        if needs_stabilization {
            wait = wait.max(MIN_NODE_CONTROL_COOLDOWN);
        }
        Some(wait)
    }

    /// Spawn each workload in its own task.
    fn spawn_workloads(
        workloads: &[Arc<dyn Workload<E>>],
        context: Arc<RunContext<E>>,
    ) -> JoinSet<WorkloadOutcome> {
        let mut running = JoinSet::new();
        for workload in workloads {
            let workload = Arc::clone(workload);
            let ctx = Arc::clone(&context);

            running.spawn(async move {
                // Convert panics into workload errors so the runner can report
                // them instead of aborting the process.
                let outcome = AssertUnwindSafe(async { workload.start(ctx.as_ref()).await })
                    .catch_unwind()
                    .await;

                outcome.unwrap_or_else(|panic| {
                    Err(format!("workload panicked: {}", panic_message(panic)).into())
                })
            });
        }

        running
    }

    /// Drive workload tasks until timeout or failure.
    async fn run_window_until_timeout(
        workloads: &mut JoinSet<WorkloadOutcome>,
        duration: Duration,
        expectations: &mut [Box<dyn Expectation<E>>],
        context: &RunContext<E>,
    ) -> Result<(), ScenarioError> {
        if duration.is_zero() {
            return Ok(());
        }

        let timer = sleep(duration);
        tokio::pin!(timer);
        let mut capture_tick = capture_check_interval();

        loop {
            tokio::select! {
                _ = &mut timer => return Ok(()),
                _ = capture_tick.tick() => {
                    Self::run_capture_checks(expectations, context).await?;
                }
                result = workloads.join_next(), if !workloads.is_empty() => {
                    let Some(result) = result else {
                        return Ok(());
                    };

                    Self::map_join_result(result)?;

                    if workloads.is_empty() {
                        return Ok(());
                    }
                }
            }
        }
    }

    async fn run_capture_checks(
        expectations: &mut [Box<dyn Expectation<E>>],
        context: &RunContext<E>,
    ) -> Result<(), ScenarioError> {
        let expectation_count = expectations.len();

        for expectation in expectations {
            if let Err(source) = expectation.check_during_capture(context).await {
                warn!(expectation = expectation.name(), %source, "expectation failed during capture");

                return Err(capture_check_failure(expectation.name(), source));
            }
        }

        debug!(
            expectations = expectation_count,
            "expectation capture check pass"
        );

        Ok(())
    }

    async fn run_idle_window_with_capture_checks(
        duration: Duration,
        expectations: &mut [Box<dyn Expectation<E>>],
        context: &RunContext<E>,
    ) -> Result<(), ScenarioError> {
        if duration.is_zero() {
            return Ok(());
        }

        let timer = sleep(duration);
        tokio::pin!(timer);
        let mut capture_tick = capture_check_interval();

        loop {
            tokio::select! {
                _ = &mut timer => return Ok(()),
                _ = capture_tick.tick() => {
                    Self::run_capture_checks(expectations, context).await?;
                }
            }
        }
    }

    fn map_join_result(result: Result<WorkloadOutcome, JoinError>) -> Result<(), ScenarioError> {
        match result {
            Ok(Ok(())) => Ok(()),
            Ok(Err(err)) => Err(ScenarioError::Workload(err)),
            Err(join_err) => Err(ScenarioError::Workload(join_err.into())),
        }
    }

    /// Wait for all workloads to exit.
    async fn drain_workloads(
        workloads: &mut JoinSet<WorkloadOutcome>,
    ) -> Result<(), ScenarioError> {
        while let Some(result) = workloads.join_next().await {
            Self::map_join_result(result)?;
        }

        Ok(())
    }
}

fn nonzero_cooldown(cooldown: Option<Duration>) -> Option<Duration> {
    cooldown.filter(|duration| !duration.is_zero())
}

fn capture_check_interval() -> Interval {
    interval(EXPECTATION_CAPTURE_CHECK_INTERVAL)
}

fn panic_message(panic: Box<dyn Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|message| (*message).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| UNKNOWN_PANIC.to_owned())
}

fn capture_check_failure(expectation: &str, source: DynError) -> ScenarioError {
    ScenarioError::ExpectationFailedDuringCapture(format!("{expectation}: {source}").into())
}

fn expectation_failure_summary(failures: Vec<(String, DynError)>) -> String {
    failures
        .into_iter()
        .map(|(name, source)| format!("{name}: {source}"))
        .collect::<Vec<String>>()
        .join("\n")
}

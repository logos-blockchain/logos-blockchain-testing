use std::{any::Any, future::Future, panic::AssertUnwindSafe, sync::Arc, time::Duration};

use futures::{FutureExt as _, future};
use tokio::{
    task::{JoinError, JoinSet},
    time::{sleep, timeout},
};

use super::deployer::ScenarioError;
use crate::scenario::{
    Application, DynError, Expectation, Scenario,
    runtime::context::{CleanupGuard, RunContext, RunHandle},
};

type WorkloadOutcome = Result<(), DynError>;

const MIN_NODE_CONTROL_COOLDOWN: Duration = Duration::from_secs(30);
const DEFAULT_BLOCK_FEED_SETTLE_WAIT: Duration = Duration::from_secs(1);
const MIN_BLOCK_FEED_SETTLE_WAIT: Duration = Duration::from_secs(2);
const UNKNOWN_PANIC: &str = "<unknown panic>";

/// Represents a fully prepared environment capable of executing a scenario.
pub struct Runner<E: Application> {
    context: Arc<RunContext<E>>,
    cleanup_guard: Option<Box<dyn CleanupGuard>>,
}

impl<E: Application> Runner<E> {
    /// Construct a runner from the run context and optional cleanup guard.
    #[must_use]
    pub fn new(context: RunContext<E>, cleanup_guard: Option<Box<dyn CleanupGuard>>) -> Self {
        Self {
            context: Arc::new(context),
            cleanup_guard,
        }
    }

    /// Access the underlying run context.
    #[must_use]
    pub fn context(&self) -> Arc<RunContext<E>> {
        Arc::clone(&self.context)
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
        let context = self.context();

        self.run_step(Self::prepare_expectations(
            scenario.expectations_mut(),
            context.as_ref(),
        ))
        .await?;

        self.run_step(Self::run_workloads(Arc::clone(&context), scenario))
            .await?;

        Self::settle_before_expectations(context.as_ref()).await;

        self.run_step(Self::run_expectations(
            scenario.expectations_mut(),
            context.as_ref(),
        ))
        .await?;

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

    async fn run_workloads<Caps>(
        context: Arc<RunContext<E>>,
        scenario: &Scenario<E, Caps>,
    ) -> Result<(), ScenarioError>
    where
        Caps: Send + Sync,
    {
        if scenario.workloads().is_empty() {
            return idle_until_duration(scenario.duration()).await;
        }

        let mut workloads = Self::spawn_workloads(scenario, Arc::clone(&context));
        Self::run_workload_window(&mut workloads, scenario.duration()).await?;

        if let Some(cooldown) = nonzero_cooldown(Self::cooldown_duration(context.as_ref())) {
            Self::run_workload_window(&mut workloads, cooldown).await?;
        }

        Self::drain_workloads(&mut workloads).await
    }

    async fn run_workload_window(
        workloads: &mut JoinSet<WorkloadOutcome>,
        duration: Duration,
    ) -> Result<(), ScenarioError> {
        let _completed = Self::drive_until_timer(workloads, duration).await?;
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
        let has_node_control = context.controls_nodes();
        let configured_wait = context.expectation_cooldown();

        if configured_wait.is_zero() && !has_node_control {
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
        let needs_stabilization = context.controls_nodes();

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
    fn spawn_workloads<Caps>(
        scenario: &Scenario<E, Caps>,
        context: Arc<RunContext<E>>,
    ) -> JoinSet<WorkloadOutcome>
    where
        Caps: Send + Sync,
    {
        let mut workloads = JoinSet::new();
        for workload in scenario.workloads() {
            let workload = Arc::clone(workload);
            let ctx = Arc::clone(&context);

            workloads.spawn(async move {
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

        workloads
    }

    /// Drive workload tasks until timeout or failure.
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

        match timeout(duration, run_future).await {
            Ok(result) => {
                result?;
                Ok(true)
            }

            Err(_) => Ok(false),
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

async fn idle_until_duration(duration: Duration) -> Result<(), ScenarioError> {
    if duration.is_zero() {
        return Ok(());
    }

    let _ = timeout(duration, async { future::pending::<()>().await }).await;
    Ok(())
}

fn nonzero_cooldown(cooldown: Option<Duration>) -> Option<Duration> {
    cooldown.filter(|duration| !duration.is_zero())
}

fn panic_message(panic: Box<dyn Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|message| (*message).to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| UNKNOWN_PANIC.to_owned())
}

fn expectation_failure_summary(failures: Vec<(String, DynError)>) -> String {
    failures
        .into_iter()
        .map(|(name, source)| format!("{name}: {source}"))
        .collect::<Vec<String>>()
        .join("\n")
}

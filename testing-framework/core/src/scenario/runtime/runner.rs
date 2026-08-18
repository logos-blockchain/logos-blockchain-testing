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
const DEFAULT_POST_WORKLOAD_SETTLE_WAIT: Duration = Duration::from_secs(1);
const MIN_POST_WORKLOAD_SETTLE_WAIT: Duration = Duration::from_secs(2);
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
        // Give runtime extensions a short catch-up window before evaluating
        // expectations.
        let Some(wait) = Self::settle_wait_duration(context) else {
            return;
        };

        sleep(wait).await;
    }

    fn settle_wait_duration(context: &RunContext<E>) -> Option<Duration> {
        let has_node_control = context.node_control().is_some();
        let configured_wait = context.expectation_cooldown();

        if configured_wait.is_zero() && !has_node_control {
            return None;
        }

        let wait = if configured_wait.is_zero() {
            DEFAULT_POST_WORKLOAD_SETTLE_WAIT
        } else {
            configured_wait
        };

        Some(wait.max(MIN_POST_WORKLOAD_SETTLE_WAIT))
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
        // Managed environments need a minimum cooldown so runtime extensions and
        // expectations observe stabilized state.
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

#[cfg(test)]
mod tests {
    use std::{
        io,
        sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    use async_trait::async_trait;

    use super::Runner;
    use crate::{
        scenario::{
            Application, ClusterControlProfile, DynError, Expectation, Metrics, NodeClients,
            RunContext, ScenarioBuilder, ScenarioError, Workload, internal::CleanupGuard,
            runtime::RuntimeAssembly,
        },
        topology::NodeCountTopology,
    };

    struct TestApp;

    #[async_trait]
    impl Application for TestApp {
        type Deployment = NodeCountTopology;
        type NodeClient = u8;
        type NodeConfig = ();
    }

    struct CountingCleanup(Arc<AtomicUsize>);

    impl CleanupGuard for CountingCleanup {
        fn cleanup(self: Box<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    enum WorkloadOutcome {
        Pass,
        Fail,
        Panic,
    }

    struct TestWorkload {
        events: Arc<Mutex<Vec<&'static str>>>,
        outcome: WorkloadOutcome,
    }

    #[async_trait]
    impl Workload<TestApp> for TestWorkload {
        fn name(&self) -> &str {
            "test_workload"
        }

        async fn start(&self, _ctx: &RunContext<TestApp>) -> Result<(), DynError> {
            self.events.lock().expect("events lock").push("workload");
            match self.outcome {
                WorkloadOutcome::Pass => Ok(()),
                WorkloadOutcome::Fail => Err(io::Error::other("workload failed").into()),
                WorkloadOutcome::Panic => panic!("workload panic"),
            }
        }
    }

    struct TestExpectation {
        name: &'static str,
        events: Arc<Mutex<Vec<&'static str>>>,
        fail_capture: bool,
        fail_evaluation: bool,
    }

    #[async_trait]
    impl Expectation<TestApp> for TestExpectation {
        fn name(&self) -> &str {
            self.name
        }

        async fn start_capture(&mut self, _ctx: &RunContext<TestApp>) -> Result<(), DynError> {
            self.events.lock().expect("events lock").push("capture");
            if self.fail_capture {
                return Err(io::Error::other("capture failed").into());
            }
            Ok(())
        }

        async fn evaluate(&mut self, _ctx: &RunContext<TestApp>) -> Result<(), DynError> {
            self.events.lock().expect("events lock").push("evaluate");
            if self.fail_evaluation {
                return Err(io::Error::other(format!("{} failed", self.name)).into());
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn runner_executes_capture_workload_and_evaluation_then_transfers_cleanup() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let cleanup_calls = Arc::new(AtomicUsize::new(0));
        let mut scenario = scenario(
            TestWorkload {
                events: Arc::clone(&events),
                outcome: WorkloadOutcome::Pass,
            },
            vec![expectation("passes", &events, false, false)],
        );

        let handle = runner(&cleanup_calls)
            .run(&mut scenario)
            .await
            .expect("scenario should pass");

        assert_eq!(
            *events.lock().expect("events lock"),
            vec!["capture", "workload", "evaluate"]
        );
        assert_eq!(cleanup_calls.load(Ordering::SeqCst), 0);

        drop(handle);
        assert_eq!(cleanup_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn workload_failure_triggers_cleanup_immediately() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let cleanup_calls = Arc::new(AtomicUsize::new(0));
        let mut scenario = scenario(
            TestWorkload {
                events: Arc::clone(&events),
                outcome: WorkloadOutcome::Fail,
            },
            Vec::new(),
        );

        let error = runner(&cleanup_calls)
            .run(&mut scenario)
            .await
            .err()
            .expect("workload failure must fail the scenario");

        assert!(matches!(error, ScenarioError::Workload(_)));
        assert!(error.to_string().contains("workload failed"));
        assert_eq!(cleanup_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn workload_panic_is_reported_as_a_workload_error() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let cleanup_calls = Arc::new(AtomicUsize::new(0));
        let mut scenario = scenario(
            TestWorkload {
                events,
                outcome: WorkloadOutcome::Panic,
            },
            Vec::new(),
        );

        let error = runner(&cleanup_calls)
            .run(&mut scenario)
            .await
            .err()
            .expect("workload panic must fail the scenario");

        assert!(
            error
                .to_string()
                .contains("workload panicked: workload panic")
        );
        assert_eq!(cleanup_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn expectation_failures_are_aggregated_and_cleaned_up() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let cleanup_calls = Arc::new(AtomicUsize::new(0));
        let mut scenario = scenario(
            TestWorkload {
                events: Arc::clone(&events),
                outcome: WorkloadOutcome::Pass,
            },
            vec![
                expectation("first", &events, false, true),
                expectation("second", &events, false, true),
            ],
        );

        let error = runner(&cleanup_calls)
            .run(&mut scenario)
            .await
            .err()
            .expect("failed expectations must fail the scenario");
        let message = error.to_string();

        assert!(matches!(error, ScenarioError::Expectations(_)));
        assert!(message.contains("first: first failed"));
        assert!(message.contains("second: second failed"));
        assert_eq!(cleanup_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn capture_failure_prevents_workload_start_and_cleans_up() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let cleanup_calls = Arc::new(AtomicUsize::new(0));
        let mut scenario = scenario(
            TestWorkload {
                events: Arc::clone(&events),
                outcome: WorkloadOutcome::Pass,
            },
            vec![expectation("capture", &events, true, false)],
        );

        let error = runner(&cleanup_calls)
            .run(&mut scenario)
            .await
            .err()
            .expect("capture failure must fail the scenario");

        assert!(matches!(error, ScenarioError::ExpectationCapture(_)));
        assert_eq!(*events.lock().expect("events lock"), vec!["capture"]);
        assert_eq!(cleanup_calls.load(Ordering::SeqCst), 1);
    }

    fn scenario(
        workload: TestWorkload,
        expectations: Vec<TestExpectation>,
    ) -> crate::scenario::Scenario<TestApp> {
        let mut builder = ScenarioBuilder::<TestApp>::with_deployment(NodeCountTopology::new(1))
            .with_run_duration(Duration::ZERO)
            .with_expectation_cooldown(Duration::ZERO)
            .with_workload(workload);
        for expectation in expectations {
            builder = builder.with_expectation(expectation);
        }
        builder.build().expect("test scenario should build")
    }

    fn runner(cleanup_calls: &Arc<AtomicUsize>) -> Runner<TestApp> {
        RuntimeAssembly::new(
            NodeCountTopology::new(1),
            NodeClients::new(vec![1]),
            Duration::from_secs(10),
            Duration::ZERO,
            ClusterControlProfile::ExistingClusterAttached,
            Metrics::empty(),
        )
        .build_runner(Some(Box::new(CountingCleanup(Arc::clone(cleanup_calls)))))
    }

    fn expectation(
        name: &'static str,
        events: &Arc<Mutex<Vec<&'static str>>>,
        fail_capture: bool,
        fail_evaluation: bool,
    ) -> TestExpectation {
        TestExpectation {
            name,
            events: Arc::clone(events),
            fail_capture,
            fail_evaluation,
        }
    }
}

//! Generic continuous observation runtime.
//!
//! This module provides the reusable runtime needed by both TF scenarios and
//! manual-cluster consumers such as Cucumber worlds. It does not know any app
//! semantics. Apps provide source types, observation logic, materialized state,
//! snapshots, and delta events.

mod factory;

use std::{
    any::type_name,
    collections::VecDeque,
    sync::Arc,
    time::{Duration, SystemTime},
};

use async_trait::async_trait;
pub use factory::{
    BoxedSourceProvider, ObservationExtensionFactory, SourceProviderFactory, StaticSourceProvider,
};
use parking_lot::Mutex;
use tokio::{
    sync::broadcast,
    task::JoinHandle,
    time::{MissedTickBehavior, interval},
};
use tracing::{debug, info, warn};

use crate::scenario::DynError;

/// Configuration for a background observation runtime.
#[derive(Clone, Debug)]
pub struct ObservationConfig {
    /// Time between observation cycles.
    pub interval: Duration,
    /// Maximum number of non-empty event batches retained in memory.
    pub history_limit: usize,
}

impl Default for ObservationConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(1),
            history_limit: 64,
        }
    }
}

/// One named observation source.
#[derive(Clone, Debug)]
pub struct ObservedSource<S> {
    /// Human-readable source name used in logs and app-level reporting.
    pub name: String,
    /// App-owned source handle.
    pub source: S,
}

impl<S> ObservedSource<S> {
    /// Builds one named observation source.
    #[must_use]
    pub fn new(name: &str, source: S) -> Self {
        Self {
            name: name.to_owned(),
            source,
        }
    }
}

/// Supplies the current observation source set.
#[async_trait]
pub trait SourceProvider<S>: Send + Sync + 'static {
    /// Returns the current source set for the next observation cycle.
    async fn sources(&self) -> Result<Vec<ObservedSource<S>>, DynError>;
}

/// App-owned observation logic.
#[async_trait]
pub trait Observer: Send + Sync + 'static {
    /// App-owned source type.
    type Source: Clone + Send + Sync + 'static;
    /// App-owned retained materialized state.
    type State: Send + Sync + 'static;
    /// App-owned current snapshot view.
    type Snapshot: Clone + Send + Sync + 'static;
    /// App-owned delta event type emitted per cycle.
    type Event: Clone + Send + Sync + 'static;

    /// Builds the initial retained state from the current source set.
    async fn init(&self, sources: &[ObservedSource<Self::Source>])
    -> Result<Self::State, DynError>;

    /// Advances retained state by one cycle and returns any new delta events.
    async fn poll(
        &self,
        sources: &[ObservedSource<Self::Source>],
        state: &mut Self::State,
    ) -> Result<Vec<Self::Event>, DynError>;

    /// Builds the current snapshot view from retained state.
    fn snapshot(&self, state: &Self::State) -> Self::Snapshot;
}

/// One materialized snapshot emitted by the runtime.
#[derive(Clone, Debug)]
pub struct ObservationSnapshot<S> {
    /// Monotonic cycle number.
    pub cycle: u64,
    /// Capture timestamp.
    pub observed_at: SystemTime,
    /// Number of sources used for this snapshot.
    pub source_count: usize,
    /// App-owned snapshot payload.
    pub value: S,
}

/// One delta batch emitted by a successful observation cycle.
#[derive(Clone, Debug)]
pub struct ObservationBatch<E> {
    /// Monotonic cycle number.
    pub cycle: u64,
    /// Capture timestamp.
    pub observed_at: SystemTime,
    /// Number of sources used for this batch.
    pub source_count: usize,
    /// App-owned delta events discovered in this cycle.
    pub events: Vec<E>,
}

/// Observation runtime failure stage.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationFailureStage {
    /// Source refresh failed before a poll could run.
    SourceRefresh,
    /// Observer poll failed after sources were refreshed.
    Poll,
}

/// Last failed observation cycle.
#[derive(Clone, Debug)]
pub struct ObservationFailure {
    /// Monotonic cycle number.
    pub cycle: u64,
    /// Failure timestamp.
    pub observed_at: SystemTime,
    /// Number of sources involved in the failed cycle.
    pub source_count: usize,
    /// Runtime stage that failed.
    pub stage: ObservationFailureStage,
    /// Human-readable failure message.
    pub message: String,
}

/// Errors returned while starting an observation runtime.
#[derive(Debug, thiserror::Error)]
pub enum ObservationRuntimeError {
    /// The configured interval is invalid.
    #[error("observation interval must be greater than zero")]
    InvalidInterval,
    /// Source discovery failed during runtime startup.
    #[error("failed to refresh observation sources during startup: {source}")]
    SourceRefresh {
        #[source]
        source: DynError,
    },
    /// Observer state initialization failed during runtime startup.
    #[error("failed to initialize observation state: {source}")]
    ObserverInit {
        #[source]
        source: DynError,
    },
}

/// Read-side handle for one running observer.
pub struct ObservationHandle<O: Observer> {
    shared: Arc<Mutex<SharedObservationState<O>>>,
    batches: broadcast::Sender<Arc<ObservationBatch<O::Event>>>,
}

impl<O: Observer> Clone for ObservationHandle<O> {
    fn clone(&self) -> Self {
        Self {
            shared: Arc::clone(&self.shared),
            batches: self.batches.clone(),
        }
    }
}

impl<O: Observer> ObservationHandle<O> {
    /// Returns the latest successful snapshot, if one has been produced.
    #[must_use]
    pub fn latest_snapshot(&self) -> Option<ObservationSnapshot<O::Snapshot>> {
        self.shared.lock().latest_snapshot.clone()
    }

    /// Returns retained non-empty event batches.
    #[must_use]
    pub fn history(&self) -> Vec<Arc<ObservationBatch<O::Event>>> {
        self.shared.lock().history.iter().cloned().collect()
    }

    /// Returns the most recent cycle failure, if any.
    #[must_use]
    pub fn last_error(&self) -> Option<ObservationFailure> {
        self.shared.lock().last_error.clone()
    }

    /// Subscribes to future non-empty event batches.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<ObservationBatch<O::Event>>> {
        self.batches.subscribe()
    }
}

/// Lifecycle owner for one background observation runtime.
pub struct ObservationRuntime<O: Observer> {
    handle: ObservationHandle<O>,
    task: Option<JoinHandle<()>>,
}

impl<O: Observer> ObservationRuntime<O> {
    /// Starts one background observation runtime.
    pub async fn start<P>(
        provider: P,
        observer: O,
        config: ObservationConfig,
    ) -> Result<Self, ObservationRuntimeError>
    where
        P: SourceProvider<O::Source>,
    {
        ensure_positive_interval(config.interval)?;

        let sources = provider
            .sources()
            .await
            .map_err(|source| ObservationRuntimeError::SourceRefresh { source })?;

        let source_count = sources.len();
        let state = observer
            .init(&sources)
            .await
            .map_err(|source| ObservationRuntimeError::ObserverInit { source })?;

        let snapshot = build_snapshot(0, source_count, &observer, &state);
        let batches = broadcast::channel(config.history_limit.max(1)).0;
        let shared = Arc::new(Mutex::new(SharedObservationState::new(snapshot)));
        let handle = ObservationHandle {
            shared: Arc::clone(&shared),
            batches,
        };

        info!(
            observer = type_name::<O>(),
            interval_ms = config.interval.as_millis(),
            history_limit = config.history_limit,
            source_count,
            "starting observation runtime"
        );

        let runtime_handle = handle.clone();
        let task = tokio::spawn(run_observation_loop(
            provider,
            observer,
            config,
            shared,
            runtime_handle.batches.clone(),
            state,
        ));

        Ok(Self {
            handle: runtime_handle,
            task: Some(task),
        })
    }

    /// Returns a read-side handle for the running observer.
    #[must_use]
    pub fn handle(&self) -> ObservationHandle<O> {
        self.handle.clone()
    }

    /// Splits the runtime into its handle and background task.
    #[must_use]
    pub fn into_parts(mut self) -> (ObservationHandle<O>, JoinHandle<()>) {
        let task = self
            .task
            .take()
            .expect("observation runtime task is always present before into_parts");

        (self.handle.clone(), task)
    }

    /// Aborts the background task.
    pub fn abort(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl<O: Observer> Drop for ObservationRuntime<O> {
    fn drop(&mut self) {
        self.abort();
    }
}

struct SharedObservationState<O: Observer> {
    latest_snapshot: Option<ObservationSnapshot<O::Snapshot>>,
    history: VecDeque<Arc<ObservationBatch<O::Event>>>,
    last_error: Option<ObservationFailure>,
}

impl<O: Observer> SharedObservationState<O> {
    fn new(snapshot: ObservationSnapshot<O::Snapshot>) -> Self {
        Self {
            latest_snapshot: Some(snapshot),
            history: VecDeque::new(),
            last_error: None,
        }
    }
}

async fn run_observation_loop<O, P>(
    provider: P,
    observer: O,
    config: ObservationConfig,
    shared: Arc<Mutex<SharedObservationState<O>>>,
    batches: broadcast::Sender<Arc<ObservationBatch<O::Event>>>,
    mut state: O::State,
) where
    O: Observer,
    P: SourceProvider<O::Source>,
{
    let mut ticker = build_interval(config.interval);
    let mut cycle = 1u64;

    ticker.tick().await;

    loop {
        ticker.tick().await;

        let cycle_outcome = observe_cycle(&provider, &observer, cycle, &mut state).await;

        match cycle_outcome {
            Ok(success) => record_cycle_success(&shared, &batches, &config, success),
            Err(failure) => record_cycle_failure(&shared, failure),
        }

        cycle += 1;
    }
}

struct CycleSuccess<O: Observer> {
    snapshot: ObservationSnapshot<O::Snapshot>,
    batch: Option<Arc<ObservationBatch<O::Event>>>,
}

async fn observe_cycle<O, P>(
    provider: &P,
    observer: &O,
    cycle: u64,
    state: &mut O::State,
) -> Result<CycleSuccess<O>, ObservationFailure>
where
    O: Observer,
    P: SourceProvider<O::Source>,
{
    let sources = provider.sources().await.map_err(|source| {
        build_failure(cycle, 0, ObservationFailureStage::SourceRefresh, source)
    })?;

    let source_count = sources.len();
    let events = observer.poll(&sources, state).await.map_err(|source| {
        build_failure(cycle, source_count, ObservationFailureStage::Poll, source)
    })?;

    let snapshot = build_snapshot(cycle, source_count, observer, state);
    let batch = build_batch(cycle, source_count, events);

    Ok(CycleSuccess { snapshot, batch })
}

fn record_cycle_success<O: Observer>(
    shared: &Arc<Mutex<SharedObservationState<O>>>,
    batches: &broadcast::Sender<Arc<ObservationBatch<O::Event>>>,
    config: &ObservationConfig,
    success: CycleSuccess<O>,
) {
    debug!(
        observer = type_name::<O>(),
        cycle = success.snapshot.cycle,
        source_count = success.snapshot.source_count,
        event_count = success.batch.as_ref().map_or(0, |batch| batch.events.len()),
        "observation cycle completed"
    );

    let mut state = shared.lock();
    state.latest_snapshot = Some(success.snapshot);
    state.last_error = None;

    let Some(batch) = success.batch else {
        return;
    };

    push_history(&mut state.history, Arc::clone(&batch), config.history_limit);
    drop(state);

    let _ = batches.send(batch);
}

fn record_cycle_failure<O: Observer>(
    shared: &Arc<Mutex<SharedObservationState<O>>>,
    failure: ObservationFailure,
) {
    warn!(
        observer = type_name::<O>(),
        cycle = failure.cycle,
        source_count = failure.source_count,
        stage = ?failure.stage,
        message = %failure.message,
        "observation cycle failed"
    );

    shared.lock().last_error = Some(failure);
}

fn ensure_positive_interval(interval: Duration) -> Result<(), ObservationRuntimeError> {
    if interval.is_zero() {
        return Err(ObservationRuntimeError::InvalidInterval);
    }

    Ok(())
}

fn build_interval(period: Duration) -> tokio::time::Interval {
    let mut ticker = interval(period);
    ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
    ticker
}

fn build_snapshot<O: Observer>(
    cycle: u64,
    source_count: usize,
    observer: &O,
    state: &O::State,
) -> ObservationSnapshot<O::Snapshot> {
    ObservationSnapshot {
        cycle,
        observed_at: SystemTime::now(),
        source_count,
        value: observer.snapshot(state),
    }
}

fn build_batch<E>(
    cycle: u64,
    source_count: usize,
    events: Vec<E>,
) -> Option<Arc<ObservationBatch<E>>> {
    if events.is_empty() {
        return None;
    }

    Some(Arc::new(ObservationBatch {
        cycle,
        observed_at: SystemTime::now(),
        source_count,
        events,
    }))
}

fn build_failure(
    cycle: u64,
    source_count: usize,
    stage: ObservationFailureStage,
    source: DynError,
) -> ObservationFailure {
    ObservationFailure {
        cycle,
        observed_at: SystemTime::now(),
        source_count,
        stage,
        message: source.to_string(),
    }
}

fn push_history<E>(
    history: &mut VecDeque<Arc<ObservationBatch<E>>>,
    batch: Arc<ObservationBatch<E>>,
    history_limit: usize,
) {
    if history_limit == 0 {
        return;
    }

    history.push_back(batch);

    while history.len() > history_limit {
        history.pop_front();
    }
}

#[cfg(test)]
mod tests;

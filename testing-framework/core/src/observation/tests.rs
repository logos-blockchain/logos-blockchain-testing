use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use parking_lot::Mutex;
use tokio::time::{Instant, sleep};

use super::{
    ObservationConfig, ObservationFailureStage, ObservationRuntime, ObservedSource, Observer,
    SourceProvider,
};
use crate::scenario::DynError;

#[derive(Clone)]
struct TestSourceProvider {
    sources: Arc<Mutex<Vec<ObservedSource<u64>>>>,
    fail_refreshes: Arc<AtomicUsize>,
}

impl TestSourceProvider {
    fn new(sources: Vec<ObservedSource<u64>>) -> Self {
        Self {
            sources: Arc::new(Mutex::new(sources)),
            fail_refreshes: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn replace_sources(&self, sources: Vec<ObservedSource<u64>>) {
        *self.sources.lock() = sources;
    }

    fn fail_next_refresh(&self) {
        self.fail_refreshes.store(1, Ordering::SeqCst);
    }
}

#[async_trait]
impl SourceProvider<u64> for TestSourceProvider {
    async fn sources(&self) -> Result<Vec<ObservedSource<u64>>, DynError> {
        if self.fail_refreshes.swap(0, Ordering::SeqCst) == 1 {
            return Err("refresh failed".into());
        }

        Ok(self.sources.lock().clone())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TestSnapshot {
    total_sources_seen: u64,
    last_source_count: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct TestEvent {
    total_sources_seen: u64,
}

#[derive(Default)]
struct TestState {
    total_sources_seen: u64,
    last_source_count: usize,
}

struct CountingObserver;

#[async_trait]
impl Observer for CountingObserver {
    type Source = u64;
    type State = TestState;
    type Snapshot = TestSnapshot;
    type Event = TestEvent;

    async fn init(
        &self,
        sources: &[ObservedSource<Self::Source>],
    ) -> Result<Self::State, DynError> {
        Ok(TestState {
            total_sources_seen: sources.iter().map(|source| source.source).sum(),
            last_source_count: sources.len(),
        })
    }

    async fn poll(
        &self,
        sources: &[ObservedSource<Self::Source>],
        state: &mut Self::State,
    ) -> Result<Vec<Self::Event>, DynError> {
        state.total_sources_seen += sources.iter().map(|source| source.source).sum::<u64>();
        state.last_source_count = sources.len();

        Ok(vec![TestEvent {
            total_sources_seen: state.total_sources_seen,
        }])
    }

    fn snapshot(&self, state: &Self::State) -> Self::Snapshot {
        TestSnapshot {
            total_sources_seen: state.total_sources_seen,
            last_source_count: state.last_source_count,
        }
    }
}

#[tokio::test]
async fn runtime_updates_snapshot_and_history() {
    let provider = TestSourceProvider::new(vec![ObservedSource::new("node-0", 2)]);
    let runtime = ObservationRuntime::start(
        provider,
        CountingObserver,
        ObservationConfig {
            interval: Duration::from_millis(25),
            history_limit: 2,
        },
    )
    .await
    .expect("runtime should start");

    let handle = runtime.handle();
    wait_for_cycle(&handle, 2).await;

    let snapshot = handle.latest_snapshot().expect("snapshot should exist");
    assert!(snapshot.cycle >= 2);
    assert_eq!(snapshot.source_count, 1);
    assert_eq!(snapshot.value.last_source_count, 1);
    assert!(snapshot.value.total_sources_seen >= 6);

    let history = handle.history();
    assert_eq!(history.len(), 2);
    assert!(history.iter().all(|batch| !batch.events.is_empty()));
}

#[tokio::test]
async fn runtime_refreshes_sources_each_cycle() {
    let provider = TestSourceProvider::new(vec![ObservedSource::new("node-0", 1)]);
    let runtime = ObservationRuntime::start(
        provider.clone(),
        CountingObserver,
        ObservationConfig {
            interval: Duration::from_millis(25),
            history_limit: 4,
        },
    )
    .await
    .expect("runtime should start");

    let handle = runtime.handle();
    wait_for_cycle(&handle, 1).await;

    provider.replace_sources(vec![
        ObservedSource::new("node-0", 1),
        ObservedSource::new("node-1", 3),
    ]);

    wait_for_snapshot_source_count(&handle, 2).await;

    let snapshot = handle.latest_snapshot().expect("snapshot should exist");
    assert_eq!(snapshot.source_count, 2);
    assert_eq!(snapshot.value.last_source_count, 2);
}

#[tokio::test]
async fn runtime_records_cycle_failures() {
    let provider = TestSourceProvider::new(vec![ObservedSource::new("node-0", 1)]);
    let runtime = ObservationRuntime::start(
        provider.clone(),
        CountingObserver,
        ObservationConfig {
            interval: Duration::from_millis(25),
            history_limit: 2,
        },
    )
    .await
    .expect("runtime should start");

    let handle = runtime.handle();
    provider.fail_next_refresh();

    wait_for_failure(&handle).await;

    let failure = handle.last_error().expect("failure should exist");
    assert_eq!(failure.stage, ObservationFailureStage::SourceRefresh);
    assert_eq!(failure.message, "refresh failed");
}

async fn wait_for_cycle(handle: &super::ObservationHandle<CountingObserver>, cycle: u64) {
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        let Some(snapshot) = handle.latest_snapshot() else {
            sleep(Duration::from_millis(10)).await;
            continue;
        };

        if snapshot.cycle >= cycle {
            return;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for cycle {cycle}"
        );

        sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_snapshot_source_count(
    handle: &super::ObservationHandle<CountingObserver>,
    source_count: usize,
) {
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        let Some(snapshot) = handle.latest_snapshot() else {
            sleep(Duration::from_millis(10)).await;
            continue;
        };

        if snapshot.source_count == source_count {
            return;
        }

        assert!(
            Instant::now() < deadline,
            "timed out waiting for source_count {source_count}"
        );

        sleep(Duration::from_millis(10)).await;
    }
}

async fn wait_for_failure(handle: &super::ObservationHandle<CountingObserver>) {
    let deadline = Instant::now() + Duration::from_secs(2);

    loop {
        if handle.last_error().is_some() {
            return;
        }

        assert!(Instant::now() < deadline, "timed out waiting for failure");

        sleep(Duration::from_millis(10)).await;
    }
}

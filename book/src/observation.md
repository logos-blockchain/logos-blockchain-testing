# Continuous Observation

The observation runtime polls application state in the background and stores snapshots, histories, and event streams for workloads and expectations. It provides typed state inside the test process rather than external telemetry.

---

## Shared Polling Runtime

Chaos and convergence tests repeatedly query state such as the current leader, whether every node has seen a key, or what changed after a restart. The observation runtime (`testing-framework/core/src/observation/`) runs one background polling task with shared error and staleness tracking. Workloads and expectations read the stored state.

[Telemetry](telemetry.md) exports metrics, logs, and traces to external endpoints. Observation instead keeps typed application state inside the test and makes it synchronously queryable during the run.

---

## The Observer Trait

An application defines how to poll and interpret its state; the runtime schedules the polling:

```rust,ignore
#[async_trait]
pub trait Observer: Send + Sync + 'static {
    type Source: Clone + Send + Sync + 'static;   // app-owned source handle
    type State: Send + Sync + 'static;            // retained materialized state
    type Snapshot: Clone + Send + Sync + 'static; // current view
    type Event: Clone + Send + Sync + 'static;    // delta emitted per cycle

    async fn init(&self, sources: &[ObservedSource<Self::Source>]) -> Result<Self::State, DynError>;

    async fn poll(
        &self,
        sources: &[ObservedSource<Self::Source>],
        state: &mut Self::State,
    ) -> Result<Vec<Self::Event>, DynError>;

    fn snapshot(&self, state: &Self::State) -> Self::Snapshot;
}
```

Each cycle the runtime refreshes the source set, calls `poll` to advance `State` and collect delta `Event`s, then derives a `Snapshot` from the state. `ObservedSource<S>` is just a `name` plus the app-owned source value (`ObservedSource::new(name, source)`), typically a node client.

**Sources are re-queried every cycle** through `SourceProvider<S>`:

```rust,ignore
#[async_trait]
pub trait SourceProvider<S>: Send + Sync + 'static {
    async fn sources(&self) -> Result<Vec<ObservedSource<S>>, DynError>;
}
```

`StaticSourceProvider::new(sources)` covers the common fixed-cluster case. A custom provider makes sources *dynamic*: it can return a different set each cycle, which lets observation stay correct across node restarts. `SourceProviderFactory<E, S>` builds the provider once node clients exist; any closure `Fn(&E::Deployment, NodeClients<E>) -> Result<BoxedSourceProvider<S>, DynError>` qualifies.

---

## Plugging Into a Scenario

`ObservationExtensionFactory<E, O>` is a [runtime extension factory](runtime-extensions.md): at prepare time it builds the source provider, starts the runtime, and stores the read handle in the `RunContext` (background task registered for abort-on-teardown via `PreparedRuntimeExtension::from_task`). The builder has convenience methods for it (`CoreBuilderExt`):

```rust,ignore
// Clonable observer:
builder.with_observer(MyObserver, my_source_provider_fn, ObservationConfig::default())
// Observer built lazily per run:
builder.with_observer_factory(|| MyObserver::new(), my_source_provider_fn, config)
```

`ObservationConfig` has two fields: `interval` (time between cycles, default 1 s, must be non-zero) and `history_limit` (retained non-empty event batches, default 64).

Outside scenarios, for example around a `ManualCluster`, start it directly: `ObservationRuntime::start(provider, observer, config)`, then `handle()`, `into_parts()` (handle + `JoinHandle`), or `abort()`. Dropping the runtime aborts the task.

---

## Reading: the ObservationHandle

Workloads and expectations retrieve the handle by type and read four things:

| Method | Returns |
|--------|---------|
| `latest_snapshot()` | `Option<ObservationSnapshot<O::Snapshot>>` — most recent successful view |
| `history()` | Retained non-empty `ObservationBatch<O::Event>`s, oldest first, bounded by `history_limit` |
| `last_error()` | `Option<ObservationFailure>` — the most recent failed cycle |
| `subscribe()` | `broadcast::Receiver` of future non-empty batches |

**Snapshots vs batches vs events:** a *snapshot* is the whole current view (`cycle`, `observed_at`, `source_count`, `value`); an *event* is one delta discovered during a cycle; a *batch* groups the events of one cycle. Cycles that produce no events produce no batch. `history()` and `subscribe()` only ever see non-empty batches, while `latest_snapshot()` is refreshed on every successful cycle.

**Freshness and failures.** On a failed cycle, the runtime records an `ObservationFailure` (with `stage: SourceRefresh` if source discovery failed, `stage: Poll` if the observer failed) and retains the last successful snapshot. The next successful cycle clears `last_error`. To check staleness, compare `snapshot.cycle` or `observed_at` across reads, and inspect `last_error()` when a wait times out; it usually names the source that stopped answering.

---

## Worked Example: the OpenRaft Cluster Observer

`examples/openraft_kv/testing/integration/src/observation.rs` observes a Raft cluster. State and snapshot are the same type (the latest per-node states plus any per-source failures), and no delta events are emitted (`Event = ()`):

```rust,ignore
#[derive(Clone, Debug, Default)]
pub struct OpenRaftClusterObserver;

#[async_trait]
impl Observer for OpenRaftClusterObserver {
    type Source = OpenRaftKvClient;
    type State = OpenRaftClusterSnapshot;
    type Snapshot = OpenRaftClusterSnapshot;
    type Event = ();

    async fn init(&self, sources: &[ObservedSource<Self::Source>]) -> Result<Self::State, DynError> {
        Ok(capture_cluster_snapshot(sources).await)
    }

    async fn poll(
        &self,
        sources: &[ObservedSource<Self::Source>],
        state: &mut Self::State,
    ) -> Result<Vec<Self::Event>, DynError> {
        *state = capture_cluster_snapshot(sources).await;
        Ok(Vec::new())
    }

    fn snapshot(&self, state: &Self::State) -> Self::Snapshot {
        state.clone()
    }
}
```

`capture_cluster_snapshot` queries each source's `/state` endpoint and records per-node errors as `OpenRaftSourceFailure` values instead of failing the cycle. A node restarting therefore appears as a named failure inside the snapshot. The snapshot type provides `agreed_leader(different_from)`, `all_voters_match(...)`, `all_kv_match(...)`, and `summary()` for timeout messages.

Two source providers accompany it:

```rust,ignore
// Fixed: scenario runs, sources from the run's node clients.
pub fn openraft_cluster_source_provider(
    _deployment: &<OpenRaftKvEnv as Application>::Deployment,
    node_clients: NodeClients<OpenRaftKvEnv>,
) -> Result<BoxedSourceProvider<OpenRaftKvClient>, DynError> {
    Ok(Box::new(StaticSourceProvider::new(named_sources(node_clients.snapshot()))))
}
```

and `OpenRaftManualClusterSourceProvider`, a dynamic provider that re-resolves clients from a `ManualCluster` on every cycle so observation follows manual restarts. The scenario builder wires the fixed one in via `with_observer(OpenRaftClusterObserver, openraft_cluster_source_provider, OpenRaftClusterObserver::config())`.

The [failover scenario](chaos.md) waits on this observed state:

```rust,ignore
let observer = ctx.require_extension::<ObservationHandle<OpenRaftClusterObserver>>()?;
let leader = wait_for_observed_leader(&observer, timeout, None).await?;
```

> **External example:** logos-blockchain's `BlockFeed` is an adopter-side analog of this pattern: an observer in its own repository materializes block records, per-node head snapshots, and transaction statistics, using the same `Observer`/`ObservationHandle` mechanism.

---

## See Also

- [Runtime Extensions](runtime-extensions.md) — the mechanism observation plugs into
- [Chaos and Controlled Failure](chaos.md) — observation-driven recovery waits
- [Expectations and Evaluation](expectations.md) — snapshot-based verdicts
- [Telemetry and External Observability](telemetry.md) — the external counterpart

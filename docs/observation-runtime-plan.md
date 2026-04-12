# Observation Runtime Plan

## Why this work exists

TF is good at deployment plumbing. It is weak at continuous observation.

Today, the same problems are solved repeatedly with custom loops:
- TF block feed logic in Logos
- Cucumber manual-cluster polling loops
- ad hoc catch-up scans for wallet and chain state
- app-local state polling in expectations

That is the gap this work should close.

The goal is not a generic "distributed systems DSL".
The goal is one reusable observation runtime that:
- continuously collects data from dynamic sources
- keeps typed materialized state
- exposes both current snapshot and delta/history views
- fits naturally in TF scenarios and Cucumber manual-cluster code

## Constraints

### TF constraints
- TF abstractions must stay universal and simple.
- TF must not know app semantics like blocks, wallets, leaders, jobs, or topics.
- TF must remain useful for simple apps such as `openraft_kv`, not only Logos.

### App constraints
- Apps must be able to build richer abstractions on top of TF.
- Logos must be able to support:
  - current block-feed replacement
  - fork-aware chain state
  - public-peer sync targets
  - multi-wallet UTXO tracking
- Apps must be able to adopt this incrementally.

### Migration constraints
- We do not want a flag-day rewrite.
- Existing loops can coexist with the new runtime until replacements are proven.

## Non-goals

This work should not:
- put feed back onto the base `Application` trait
- build app-specific semantics into TF core
- replace filesystem blockchain snapshots used for startup/restore
- force every app to use continuous observation
- introduce a large public abstraction stack that nobody can explain

## Core idea

Introduce one TF-level observation runtime.

That runtime owns:
- source refresh
- scheduling
- polling/ingestion
- bounded history
- latest snapshot caching
- delta publication
- freshness/error tracking
- lifecycle hooks for TF and Cucumber

Apps own:
- source types
- raw observation logic
- materialized state
- snapshot shape
- delta/event shape
- higher-level projections such as wallet state

## Public TF surface

The TF public surface should stay small.

### `ObservedSource<S>`
A named source instance.

Used for:
- local node clients
- public peer endpoints
- any other app-owned source type

### `SourceProvider<S>`
Returns the current source set.

This must support dynamic source lists because:
- manual cluster nodes come and go
- Cucumber worlds may attach public peers
- node control may restart or replace sources during a run

### `Observer`
App-owned observation logic.

It defines:
- `Source`
- `State`
- `Snapshot`
- `Event`

And it implements:
- `init(...)`
- `poll(...)`
- `snapshot(...)`

The important boundary is:
- TF owns the runtime
- app code owns materialization

### `ObservationRuntime`
The engine that:
- starts the loop
- refreshes sources
- calls `poll(...)`
- stores history
- publishes deltas
- updates latest snapshot
- tracks last error and freshness

### `ObservationHandle`
The read-side interface for workloads, expectations, and Cucumber steps.

It should expose at least:
- latest snapshot
- delta subscription
- bounded history
- last error

## Intended shape

```rust
pub struct ObservedSource<S> {
    pub name: String,
    pub source: S,
}

#[async_trait]
pub trait SourceProvider<S>: Send + Sync + 'static {
    async fn sources(&self) -> Vec<ObservedSource<S>>;
}

#[async_trait]
pub trait Observer: Send + Sync + 'static {
    type Source: Clone + Send + Sync + 'static;
    type State: Send + Sync + 'static;
    type Snapshot: Clone + Send + Sync + 'static;
    type Event: Clone + Send + Sync + 'static;

    async fn init(
        &self,
        sources: &[ObservedSource<Self::Source>],
    ) -> Result<Self::State, DynError>;

    async fn poll(
        &self,
        sources: &[ObservedSource<Self::Source>],
        state: &mut Self::State,
    ) -> Result<Vec<Self::Event>, DynError>;

    fn snapshot(&self, state: &Self::State) -> Self::Snapshot;
}
```

This is enough.

If more helper layers are needed, they should stay internal first.

## How current use cases fit

### `openraft_kv`
Use one simple observer.

- sources: node clients
- state: latest per-node Raft state
- snapshot: sorted node-state view
- events: optional deltas, possibly empty at first

This is the simplest proving case.
It validates the runtime without dragging in Logos complexity.

### Logos block feed replacement
Use one shared chain observer.

- sources: local node clients
- state:
  - node heads
  - block graph
  - heights
  - seen headers
  - recent history
- snapshot:
  - current head/lib/graph summary
- events:
  - newly discovered blocks

This covers both existing Logos feed use cases:
- current snapshot consumers
- delta/subscription consumers

### Cucumber manual-cluster sync
Use the same observer runtime with a different source set.

- sources:
  - local manual-cluster node clients
  - public peer endpoints
- state:
  - local consensus views
  - public consensus views
  - derived majority public target
- snapshot:
  - current local and public sync picture

This removes custom poll/sleep loops from steps.

### Multi-wallet fork-aware tracking
This should not be a TF concept.

It should be a Logos projection built on top of the shared chain observer.

- input: chain observer state
- output: per-header wallet state cache keyed by block header
- property: naturally fork-aware because it follows actual ancestry

That replaces repeated backward scans from tip with continuous maintained state.

## Logos layering

Logos should not put every concern into one giant impl.

Recommended layering:

1. **Chain source adapter**
   - local node reads
   - public peer reads

2. **Shared chain observer**
   - catch-up
   - continuous ingestion
   - graph/history materialization

3. **Logos projections**
   - head view
   - public sync target
   - fork graph queries
   - wallet state
   - tx inclusion helpers

TF provides the runtime.
Logos provides the domain model built on top.

## Adoption plan

### Phase 1: add TF observation runtime
- add `ObservedSource`, `SourceProvider`, `Observer`, `ObservationRuntime`, `ObservationHandle`
- keep the public API small
- no app migrations yet

### Phase 2: prove it on `openraft_kv`
- add one simple observer over `/state`
- migrate one expectation to use the observation handle
- validate local, compose, and k8s

### Phase 3: add Logos shared chain observer
- implement it alongside current feed/loops
- do not remove existing consumers yet
- prove snapshot and delta outputs are useful

### Phase 4: migrate one Logos consumer at a time
Suggested order:
1. fork/head snapshot consumer
2. tx inclusion consumer
3. Cucumber sync-to-public-chain logic
4. wallet/UTXO tracking

### Phase 5: delete old loops and feed paths
- only after the new runtime has replaced real consumers cleanly

## Validation gates

Each phase should have clear checks.

### Runtime-level
- crate-level `cargo check`
- targeted tests for runtime lifecycle and history retention
- explicit tests for dynamic source refresh

### App-level
- `openraft_kv`:
  - local failover
  - compose failover
  - k8s failover
- Logos:
  - one snapshot consumer migrated
  - one delta consumer migrated
- Cucumber:
  - one manual-cluster sync path migrated

## Open questions

These should stay open until implementation forces a decision:
- whether `ObservationHandle` should expose full history directly or only cursor/subscription access
- how much error/freshness metadata belongs in the generic runtime vs app snapshot types
- whether multiple observers should share one scheduler/runtime instance or simply run independently first

## Design guardrails

When implementing this work:
- keep TF public abstractions minimal
- keep app semantics out of TF core
- do not chase a generic testing DSL
- build from reusable blocks, not one-off mega impls
- keep migration incremental
- prefer simple, explainable runtime behavior over clever abstraction

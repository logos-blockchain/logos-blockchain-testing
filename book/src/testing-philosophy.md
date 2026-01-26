# Testing Philosophy

This framework embodies specific principles that shape how you author and run
scenarios. Understanding these principles helps you write effective tests and
interpret results correctly.

## Declarative over Imperative

Describe **what** you want to test, not **how** to orchestrate it:

```rust,ignore
use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn declarative_over_imperative() {
    // Good: declarative
    let _plan = ScenarioBuilder::topology_with(|t| t.network_star().nodes(2))
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
        })
        .expect_consensus_liveness()
        .build();

    // Bad: imperative (framework doesn't work this way)
    // spawn_node();
    // loop { submit_tx(); check_block(); }
}
```

**Why it matters:** The framework handles deployment, readiness, and cleanup.
You focus on test intent, not infrastructure orchestration.

**Exception:** For advanced network scenarios (split-brain, late joins, network healing) that can't be expressed declaratively, see [Manual Clusters](manual-cluster.md) for imperative control.

## Protocol Time, Not Wall Time

Reason in **blocks** and **consensus intervals**, not wall-clock seconds.

**Consensus defaults:**
- Slot duration: 2 seconds (NTP-synchronized, configurable via `CONSENSUS_SLOT_TIME`)
- Active slot coefficient: 0.9 (90% block probability per slot, configurable via `CONSENSUS_ACTIVE_SLOT_COEFF`)
- Expected rate: ~27 blocks per minute

```rust,ignore
use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn protocol_time_not_wall_time() {
    // Good: protocol-oriented thinking
    let _plan = ScenarioBuilder::topology_with(|t| t.network_star().nodes(2))
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
        })
        .with_run_duration(Duration::from_secs(60)) // Let framework calculate expected blocks
        .expect_consensus_liveness() // "Did we produce the expected blocks?"
        .build();

    // Bad: wall-clock assumptions
    // "I expect exactly 30 blocks in 60 seconds"
    // This breaks on slow CI where slot timing might drift
}
```

**Why it matters:** Slot timing is fixed (2s by default, NTP-synchronized), so the
expected number of blocks is predictable: ~27 blocks in 60s with the default
0.9 active slot coefficient. The framework calculates expected blocks from slot
duration and run window, making assertions protocol-based rather than tied to
specific wall-clock expectations. Assert on "blocks produced relative to slots"
not "blocks produced in exact wall-clock seconds".

## Determinism First, Chaos When Needed

**Default scenarios are repeatable:**
- Fixed topology
- Predictable traffic rates
- Deterministic checks

**Chaos is opt-in:**
```rust,ignore
use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::{ChaosBuilderExt, ScenarioBuilderExt};

pub fn determinism_first() {
    // Separate: functional test (deterministic)
    let _plan = ScenarioBuilder::topology_with(|t| t.network_star().nodes(2))
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
        })
        .expect_consensus_liveness()
        .build();

    // Separate: chaos test (introduces randomness)
    let _chaos_plan =
        ScenarioBuilder::topology_with(|t| t.network_star().nodes(3))
            .enable_node_control()
            .chaos_with(|c| {
                c.restart()
                    .min_delay(Duration::from_secs(30))
                    .max_delay(Duration::from_secs(60))
                    .target_cooldown(Duration::from_secs(45))
                    .apply()
            })
            .transactions_with(|txs| {
                txs.rate(5) // 5 transactions per block
            })
            .expect_consensus_liveness()
            .build();
}
```

**Why it matters:** Mixing determinism with chaos creates noisy, hard-to-debug
failures. Separate concerns make failures actionable.

## Observable Health Signals

Prefer **user-facing signals** over internal state:

**Good checks:**
- Blocks progressing at expected rate (liveness)
- Transactions included within N blocks (inclusion)
- Transactions included within N blocks (inclusion)

**Avoid internal checks:**
- Memory pool size
- Internal service state
- Cache hit rates

**Why it matters:** User-facing signals reflect actual system health.
Internal state can be "healthy" while the system is broken from a user
perspective.

## Minimum Run Windows

Always run long enough for **meaningful block production**:

```rust,ignore
use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn minimum_run_windows() {
    // Bad: too short (~2 blocks with default 2s slots, 0.9 coeff)
    let _too_short = ScenarioBuilder::with_node_counts(1)
        .with_run_duration(Duration::from_secs(5))
        .expect_consensus_liveness()
        .build();

    // Good: enough blocks for assertions (~27 blocks with default 2s slots, 0.9
    // coeff)
    let _good = ScenarioBuilder::with_node_counts(1)
        .with_run_duration(Duration::from_secs(60))
        .expect_consensus_liveness()
        .build();
}
```

**Note:** Block counts assume default consensus parameters:
- Slot duration: 2 seconds (configurable via `CONSENSUS_SLOT_TIME`)
- Active slot coefficient: 0.9 (90% block probability per slot, configurable via `CONSENSUS_ACTIVE_SLOT_COEFF`)
- Formula: `blocks ≈ (duration / slot_duration) × active_slot_coeff`

If upstream changes these parameters, adjust your duration expectations accordingly.

The framework enforces minimum durations (at least 2× slot duration), but be explicit. Very short runs risk false confidence—one lucky block doesn't prove liveness.

## Summary

These principles keep scenarios:
- **Portable** across environments (protocol time, declarative)
- **Debuggable** (determinism, separation of concerns)
- **Meaningful** (observable signals, sufficient duration)

When authoring scenarios, ask: "Does this test the protocol behavior or
my local environment quirks?"

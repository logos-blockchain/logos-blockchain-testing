# API Levels: Builder DSL vs. Direct Instantiation

The framework supports two styles for constructing scenarios:

1. **High-level Builder DSL** (recommended): fluent helper methods (e.g. `.transactions_with(...)`)
2. **Low-level direct instantiation**: construct workload/expectation types explicitly, then attach them

Both styles produce the same runtime behavior because they ultimately call the same core builder APIs.

## High-Level Builder DSL (Recommended)

The DSL is implemented as extension traits (primarily `testing_framework_workflows::ScenarioBuilderExt`) on the core scenario builder.

```rust
use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

let plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(3).executors(2))
    .wallets(5)
    .transactions_with(|txs| txs.rate(5).users(3))
    .da_with(|da| da.channel_rate(1).blob_rate(1).headroom_percent(20))
    .expect_consensus_liveness()
    .with_run_duration(Duration::from_secs(60))
    .build();
```

**When to use:**
- Most test code (smoke, regression, CI)
- When you want sensible defaults and minimal boilerplate

## Low-Level Direct Instantiation

Direct instantiation gives you explicit control over the concrete types you attach:

```rust
use std::{
    num::{NonZeroU64, NonZeroUsize},
    time::Duration,
};

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::{
    expectations::ConsensusLiveness,
    workloads::{da, transaction},
};

let tx_workload = transaction::Workload::with_rate(5)
    .expect("transaction rate must be non-zero")
    .with_user_limit(NonZeroUsize::new(3));

let da_workload = da::Workload::with_rate(
    NonZeroU64::new(1).unwrap(),  // blob rate per block
    NonZeroU64::new(1).unwrap(),  // channel rate per block
    da::Workload::default_headroom_percent(),
);

let plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(3).executors(2))
    .wallets(5)
    .with_workload(tx_workload)
    .with_workload(da_workload)
    .with_expectation(ConsensusLiveness::default())
    .with_run_duration(Duration::from_secs(60))
    .build();
```

**When to use:**
- Custom workload/expectation implementations
- Reusing preconfigured workload instances across multiple scenarios
- Debugging / exploring the underlying workload types

## Method Correspondence

| High-Level DSL | Low-Level Direct |
|----------------|------------------|
| `.transactions_with(\|txs\| txs.rate(5).users(3))` | `.with_workload(transaction::Workload::with_rate(5).expect(...).with_user_limit(...))` |
| `.da_with(\|da\| da.blob_rate(1).channel_rate(1))` | `.with_workload(da::Workload::with_rate(...))` |
| `.expect_consensus_liveness()` | `.with_expectation(ConsensusLiveness::default())` |

## Bundled Expectations (Important)

Workloads can bundle expectations by implementing `Workload::expectations()`.

These bundled expectations are attached automatically whenever you call `.with_workload(...)` (including when you use the DSL), because the core builder expands workload expectations during attachment.

## Mixing Both Styles

Mixing is common: use the DSL for built-ins, and direct instantiation for custom pieces.

```rust
use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

let custom_workload = MyCustomWorkload::new(config);

let plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(3).executors(2))
    .transactions_with(|txs| txs.rate(5).users(3)) // DSL
    .with_workload(custom_workload)                // direct
    .expect_consensus_liveness()                   // DSL
    .with_run_duration(Duration::from_secs(60))
    .build();
```

## Implementation Detail (How the DSL Works)

The DSL methods are thin wrappers. For example:

```rust
builder.transactions_with(|txs| txs.rate(5).users(3))
```

is roughly equivalent to:

```rust
builder.transactions().rate(5).users(3).apply()
```

## Troubleshooting

**DSL method not found**
- Ensure the extension traits are in scope, e.g. `use testing_framework_workflows::ScenarioBuilderExt;`
- Cross-check method names in [Builder API Quick Reference](dsl-cheat-sheet.md)

## See Also

- [Builder API Quick Reference](dsl-cheat-sheet.md)
- [Example: New Workload & Expectation (Rust)](custom-workload-example.md)
- [Extending the Framework](extending.md)

# Chaos Workloads

> **When should I read this?** You don't need chaos testing to be productive with the framework. Focus on basic scenarios first—chaos is for resilience validation and operational readiness drills once your core tests are stable.

Chaos in the framework uses node control to introduce failures and validate
recovery. The built-in restart workload lives in
`testing_framework_workflows::workloads::chaos::RandomRestartWorkload`.

## How it works
- Requires `NodeControlCapability` (`enable_node_control()` in the scenario
  builder) and a runner that provides a `NodeControlHandle`.
- Randomly selects nodes (validators, executors) to restart based on your
  include/exclude flags.
- Respects min/max delay between restarts and a target cooldown to avoid
  flapping the same node too frequently.
- Runs alongside other workloads; expectations should account for the added
  disruption.
- Support varies by runner: node control is not provided by the local runner
  and is not yet implemented for the k8s runner. Use a runner that advertises
  `NodeControlHandle` support (e.g., compose) for chaos workloads.

## Usage
```rust
use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::{ScenarioBuilderExt, workloads::chaos::RandomRestartWorkload};

pub fn random_restart_plan() -> testing_framework_core::scenario::Scenario<
    testing_framework_core::scenario::NodeControlCapability,
> {
    ScenarioBuilder::topology_with(|t| t.network_star().validators(2).executors(1))
        .enable_node_control()
        .with_workload(RandomRestartWorkload::new(
            Duration::from_secs(45),  // min delay
            Duration::from_secs(75),  // max delay
            Duration::from_secs(120), // target cooldown
            true,                     // include validators
            true,                     // include executors
        ))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(150))
        .build()
}
```

## Expectations to pair
- **Consensus liveness**: ensure blocks keep progressing despite restarts.
- **Height convergence**: optionally check all nodes converge after the chaos
  window.
- Any workload-specific inclusion checks if you’re also driving tx/DA traffic.

## Best practices
- Keep delays/cooldowns realistic; avoid back-to-back restarts that would never
  happen in production.
- Limit chaos scope: toggle validators vs executors based on what you want to
  test.
- Combine with observability: monitor metrics/logs to explain failures.

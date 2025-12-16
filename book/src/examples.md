# Examples

Concrete scenario shapes that illustrate how to combine topologies, workloads,
and expectations.

**Runnable examples:** The repo includes complete binaries in `examples/src/bin/`:
- `local_runner.rs` — Host processes (local)
- `compose_runner.rs` — Docker Compose (requires image built)
- `k8s_runner.rs` — Kubernetes (requires cluster access and image loaded)

**Recommended:** Use `scripts/run-examples.sh -t <duration> -v <validators> -e <executors> <mode>` where mode is `host`, `compose`, or `k8s`.

**Alternative:** Direct cargo run: `POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin <name>`

**All runners require `POL_PROOF_DEV_MODE=true`** to avoid expensive proof generation.

**Code patterns** below show how to build scenarios. Wrap these in `#[tokio::test]` functions for integration tests, or `#[tokio::main]` for binaries.

## Simple consensus liveness

Minimal test that validates basic block production:

```rust
use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn simple_consensus() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(3).executors(0))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(30))
        .build();

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}
```

**When to use**: smoke tests for consensus on minimal hardware.

## Transaction workload

Test consensus under transaction load:

```rust
use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn transaction_workload() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(2).executors(0))
        .wallets(20)
        .transactions_with(|txs| txs.rate(5).users(10))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(60))
        .build();

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}
```

**When to use**: validate transaction submission and inclusion.

## DA + transaction workload

Combined test stressing both transaction and DA layers:

```rust
use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn da_and_transactions() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(3).executors(2))
        .wallets(30)
        .transactions_with(|txs| txs.rate(5).users(15))
        .da_with(|da| da.channel_rate(2).blob_rate(2))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(90))
        .build();

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}
```

**When to use**: end-to-end coverage of transaction and DA layers.

## Chaos resilience

Test system resilience under node restarts:

```rust
use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_compose::ComposeDeployer;
use testing_framework_workflows::{ChaosBuilderExt, ScenarioBuilderExt};

pub async fn chaos_resilience() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(4).executors(2))
        .enable_node_control()
        .wallets(20)
        .transactions_with(|txs| txs.rate(3).users(10))
        .chaos_with(|c| {
            c.restart()
                .min_delay(Duration::from_secs(20))
                .max_delay(Duration::from_secs(40))
                .target_cooldown(Duration::from_secs(30))
                .apply()
        })
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(120))
        .build();

    let deployer = ComposeDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}
```

**When to use**: resilience validation and operational readiness drills.

**Note**: Chaos tests require `ComposeDeployer` or another runner with node control support.

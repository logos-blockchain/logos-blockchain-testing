# Builder API Quick Reference

Quick reference for the scenario builder DSL. All methods are chainable.

## Imports

```rust,ignore
use std::time::Duration;

use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_compose::ComposeDeployer;
use testing_framework_runner_k8s::K8sDeployer;
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::{ChaosBuilderExt, ScenarioBuilderExt};
```

## Topology

```rust,ignore
use testing_framework_core::scenario::{Builder, ScenarioBuilder};

pub fn topology() -> Builder<()> {
    ScenarioBuilder::topology_with(|t| {
        t.network_star() // Star topology (all connect to seed node)
            .nodes(3) // Number of nodes
    })
}
```

## Wallets

```rust,ignore
use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn wallets_plan() -> testing_framework_core::scenario::Scenario<()> {
    ScenarioBuilder::topology_with(|t| t.network_star().nodes(1))
        .wallets(50) // Seed 50 funded wallet accounts
        .build()
}
```

## Transaction Workload

```rust,ignore
use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn transactions_plan() -> testing_framework_core::scenario::Scenario<()> {
    ScenarioBuilder::topology_with(|t| t.network_star().nodes(1))
        .wallets(50)
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
                .users(20) // Use 20 of the seeded wallets
        }) // Finish transaction workload config
        .build()
}
```

## Chaos Workload (Requires `enable_node_control()`)

```rust,ignore
use std::time::Duration;

use testing_framework_core::scenario::{NodeControlCapability, ScenarioBuilder};
use testing_framework_workflows::{ChaosBuilderExt, ScenarioBuilderExt};

pub fn chaos_plan() -> testing_framework_core::scenario::Scenario<NodeControlCapability> {
    ScenarioBuilder::topology_with(|t| t.network_star().nodes(3))
        .enable_node_control() // Enable node control capability
        .chaos_with(|c| {
            c.restart() // Random restart chaos
                .min_delay(Duration::from_secs(30)) // Min time between restarts
                .max_delay(Duration::from_secs(60)) // Max time between restarts
                .target_cooldown(Duration::from_secs(45)) // Cooldown after restart
                .apply() // Required for chaos configuration
        })
        .build()
}
```

## Expectations

```rust,ignore
use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn expectations_plan() -> testing_framework_core::scenario::Scenario<()> {
    ScenarioBuilder::topology_with(|t| t.network_star().nodes(1))
        .expect_consensus_liveness() // Assert blocks are produced continuously
        .build()
}
```

## Run Duration

```rust,ignore
use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn run_duration_plan() -> testing_framework_core::scenario::Scenario<()> {
    ScenarioBuilder::topology_with(|t| t.network_star().nodes(1))
        .with_run_duration(Duration::from_secs(120)) // Run for 120 seconds
        .build()
}
```

## Build

```rust,ignore
use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn build_plan() -> testing_framework_core::scenario::Scenario<()> {
    ScenarioBuilder::topology_with(|t| t.network_star().nodes(1)).build() // Construct the final Scenario
}
```

## Deployers

```rust,ignore
use testing_framework_runner_compose::ComposeDeployer;
use testing_framework_runner_k8s::K8sDeployer;
use testing_framework_runner_local::LocalDeployer;

pub fn deployers() {
    // Local processes
    let _deployer = LocalDeployer::default();

    // Docker Compose
    let _deployer = ComposeDeployer::default();

    // Kubernetes
    let _deployer = K8sDeployer::default();
}
```

## Execution

```rust,ignore
use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn execution() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().nodes(1))
        .expect_consensus_liveness()
        .build();

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}
```

## Complete Example

```rust,ignore
use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn run_test() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().nodes(3))
        .wallets(50)
        .transactions_with(|txs| {
            txs.rate(5) // 5 transactions per block
                .users(20)
        })
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(90))
        .build();

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}
```

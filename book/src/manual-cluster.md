# Manual Clusters: Imperative Control

**When should I read this?** You're integrating external test drivers (like Cucumber/BDD frameworks) that need imperative node orchestration. This is an escape hatch for when the test orchestration must live outside the framework—most tests should use the standard scenario approach.

---

## Overview

**Manual clusters** provide imperative, on-demand node control for scenarios that don't fit the declarative `ScenarioBuilder` pattern:

```rust
use testing_framework_core::topology::config::TopologyConfig;
use testing_framework_core::scenario::{PeerSelection, StartNodeOptions};
use testing_framework_runner_local::LocalDeployer;

let config = TopologyConfig::with_node_numbers(3);
let deployer = LocalDeployer::new();
let cluster = deployer.manual_cluster(config)?;

// Start nodes on demand with explicit peer selection
let node_a = cluster.start_node_with(
    "a",
    StartNodeOptions {
        peers: PeerSelection::None, // Start isolated
    }
).await?.api;

let node_b = cluster.start_node_with(
    "b",
    StartNodeOptions {
        peers: PeerSelection::Named(vec!["node-a".to_owned()]), // Connect to A
    }
).await?.api;

// Wait for network readiness
cluster.wait_network_ready().await?;

// Custom validation logic
let info_a = node_a.consensus_info().await?;
let info_b = node_b.consensus_info().await?;
assert!(info_a.height.abs_diff(info_b.height) <= 5);
```

**Key difference from scenarios:**
- **External orchestration:** Your code (or an external driver like Cucumber) controls the execution flow step-by-step
- **Imperative model:** You call `start_node()`, `sleep()`, poll APIs directly in test logic
- **No framework execution:** The scenario runner doesn't drive workloads—you do

Note: Scenarios with node control can also start nodes dynamically, control peer selection, and orchestrate timing—but via **workloads** within the framework's execution model. Use manual clusters only when the orchestration must be external (e.g., Cucumber steps).

---

## When to Use Manual Clusters

**Manual clusters are an escape hatch for when orchestration must live outside the framework.**

Prefer workloads for scenario logic; use manual clusters only when an external system needs to control node lifecycle—for example:

**Cucumber/BDD integration**  
Gherkin steps control when nodes start, which peers they connect to, and when to verify state. The test driver (Cucumber) orchestrates the scenario step-by-step.

**Custom test harnesses**  
External scripts or tools that need programmatic control over node lifecycle as part of a larger testing pipeline.

---

## Core API

### Starting the Cluster

```rust
use testing_framework_core::topology::config::TopologyConfig;
use testing_framework_runner_local::LocalDeployer;

// Define capacity (preallocates ports/configs for N nodes)
let config = TopologyConfig::with_node_numbers(5);

let deployer = LocalDeployer::new();
let cluster = deployer.manual_cluster(config)?;
// Nodes are stopped automatically when cluster is dropped
```

**Important:** The `TopologyConfig` defines the **maximum capacity**, not the initial state. Nodes are started on-demand via API calls.

### Starting Nodes

**Default peers (topology layout):**

```rust
let node = cluster.start_node("seed").await?;
```

**No peers (isolated):**

```rust
use testing_framework_core::scenario::{PeerSelection, StartNodeOptions};

let node = cluster.start_node_with(
    "isolated",
    StartNodeOptions {
        peers: PeerSelection::None,
    }
).await?;
```

**Explicit peers (named):**

```rust
let node = cluster.start_node_with(
    "follower",
    StartNodeOptions {
        peers: PeerSelection::Named(vec![
            "node-seed".to_owned(),
            "node-isolated".to_owned(),
        ]),
    }
).await?;
```

**Note:** Node names are prefixed with `node-` internally. If you start a node with name `"a"`, reference it as `"node-a"` in peer lists.

### Getting Node Clients

```rust
// From start result
let started = cluster.start_node("my-node").await?;
let client = started.api;

// Or lookup by name
if let Some(client) = cluster.node_client("node-my-node") {
    let info = client.consensus_info().await?;
    println!("Height: {}", info.height);
}
```

### Waiting for Readiness

```rust
// Waits until all started nodes have connected to their expected peers
cluster.wait_network_ready().await?;
```

**Behavior:**
- Single-node clusters always ready (no peers to verify)
- Multi-node clusters wait for peer counts to match expectations
- Timeout after 60 seconds (120 seconds if `SLOW_TEST_ENV=true`) with diagnostic message

---

## Complete Example: External Test Driver Pattern

This shows how an external test driver (like Cucumber) might use manual clusters to control node lifecycle:

```rust
use std::time::Duration;
use anyhow::Result;
use testing_framework_core::{
    scenario::{PeerSelection, StartNodeOptions},
    topology::config::TopologyConfig,
};
use testing_framework_runner_local::LocalDeployer;
use tokio::time::sleep;

#[tokio::test]
async fn external_driver_example() -> Result<()> {
    // Step 1: Create cluster with capacity for 3 nodes
    let config = TopologyConfig::with_node_numbers(3);
    let deployer = LocalDeployer::new();
    let cluster = deployer.manual_cluster(config)?;

    // Step 2: External driver decides to start 2 nodes initially
    println!("Starting initial topology...");
    let node_a = cluster.start_node("a").await?.api;
    let node_b = cluster
        .start_node_with(
            "b",
            StartNodeOptions {
                peers: PeerSelection::Named(vec!["node-a".to_owned()]),
            },
        )
        .await?
        .api;

    cluster.wait_network_ready().await?;

    // Step 3: External driver runs some protocol operations
    let info = node_a.consensus_info().await?;
    println!("Initial cluster height: {}", info.height);

    // Step 4: Later, external driver decides to add third node
    println!("External driver adding third node...");
    let node_c = cluster
        .start_node_with(
            "c",
            StartNodeOptions {
                peers: PeerSelection::Named(vec!["node-a".to_owned()]),
            },
        )
        .await?
        .api;

    cluster.wait_network_ready().await?;

    // Step 5: External driver validates final state
    let heights = vec![
        node_a.consensus_info().await?.height,
        node_b.consensus_info().await?.height,
        node_c.consensus_info().await?.height,
    ];
    println!("Final heights: {:?}", heights);

    Ok(())
}
```

**Key pattern:**
The external driver controls **when** nodes start and **which peers** they connect to, allowing test frameworks like Cucumber to orchestrate scenarios step-by-step based on Gherkin steps or other external logic.

---

## Peer Selection Strategies

**`PeerSelection::DefaultLayout`**  
Uses the topology's network layout (star/chain/full). Default behavior.

```rust
let node = cluster.start_node_with(
    "normal",
    StartNodeOptions {
        peers: PeerSelection::DefaultLayout,
    }
).await?;
```

**`PeerSelection::None`**  
Node starts with no initial peers. Use when an external driver needs to build topology incrementally.

```rust
let isolated = cluster.start_node_with(
    "isolated",
    StartNodeOptions {
        peers: PeerSelection::None,
    }
).await?;
```

**`PeerSelection::Named(vec!["node-a", "node-b"])`**  
Explicit peer list. Use when an external driver needs to construct specific peer relationships.

```rust
let follower = cluster.start_node_with(
    "follower",
    StartNodeOptions {
        peers: PeerSelection::Named(vec![
            "node-seed".to_owned(),
            "node-seed".to_owned(),
        ]),
    }
).await?;
```

**Remember:** Node names are automatically prefixed with `node-`. If you call `start_node("a")`, reference it as `"node-a"` in peer lists.

---

## Custom Validation Patterns

Manual clusters don't have built-in expectations—you write validation logic directly:

### Height Convergence

```rust
use tokio::time::{sleep, Duration};

let start = tokio::time::Instant::now();
loop {
    let heights: Vec<u64> = vec![
        node_a.consensus_info().await?.height,
        node_b.consensus_info().await?.height,
        node_c.consensus_info().await?.height,
    ];

    let max_diff = heights.iter().max().unwrap() - heights.iter().min().unwrap();
    if max_diff <= 5 {
        println!("Converged: heights={:?}", heights);
        break;
    }

    if start.elapsed() > Duration::from_secs(60) {
        return Err(anyhow::anyhow!("Convergence timeout: heights={:?}", heights));
    }

    sleep(Duration::from_secs(2)).await;
}
```

### Peer Count Verification

```rust
let info = node.network_info().await?;
assert_eq!(
    info.n_peers, 3,
    "Expected 3 peers, found {}",
    info.n_peers
);
```

### Block Production

```rust
// Verify node is producing blocks
let initial_height = node_a.consensus_info().await?.height;

sleep(Duration::from_secs(10)).await;

let current_height = node_a.consensus_info().await?.height;
assert!(
    current_height > initial_height,
    "Node should have produced blocks: initial={}, current={}",
    initial_height,
    current_height
);
```

---

## Limitations

**Local deployer only**  
Manual clusters currently only work with `LocalDeployer`. Compose and K8s support is not available.

**No built-in workloads**  
You must manually submit transactions via node API clients. The framework's transaction workloads are scenario-specific.

**No automatic expectations**  
You wire validation yourself. The `.expect_*()` methods from scenarios are not automatically attached—you write custom validation loops.

**No RunContext**  
Manual clusters don't provide `RunContext`, so features like `BlockFeed` and metrics queries require manual setup.

---

## Relationship to Node Control

Manual clusters and [node control](node-control.md) share the same underlying infrastructure (`LocalDynamicNodes`), but serve different purposes:

| Feature | Manual Cluster | Node Control (Scenario) |
|---------|---------------|-------------------------|
| **Orchestration** | External (your code/Cucumber) | Framework (workloads) |
| **Programming model** | Imperative (step-by-step) | Declarative (plan + execute) |
| **Node lifecycle** | Manual `start_node()` calls | Automatic + workload-driven |
| **Traffic generation** | Manual API calls | Built-in workloads (tx, chaos) |
| **Validation** | Manual polling loops | Built-in expectations + custom |
| **Use case** | Cucumber/BDD integration | Standard testing & chaos |

**When to use which:**
- **Scenarios with node control** → Standard testing (built-in workloads drive node control)
- **Manual clusters** → External drivers (Cucumber/BDD where external logic drives node control)

---

## Running Manual Cluster Tests

Manual cluster tests are typically marked with `#[ignore]` to prevent accidental runs:

```rust
#[tokio::test]
#[ignore = "run manually with: cargo test -- --ignored external_driver_example"]
async fn external_driver_example() -> Result<()> {
    // ...
}
```

**To run:**

```bash
# Required: dev mode for fast proofs
POL_PROOF_DEV_MODE=true \
cargo test -p runner-examples -- --ignored external_driver_example
```

**Logs:**

```bash
# Preserve logs after test
LOGOS_BLOCKCHAIN_TESTS_KEEP_LOGS=1 \
RUST_LOG=info \
POL_PROOF_DEV_MODE=true \
cargo test -p runner-examples -- --ignored external_driver_example
```

---

## See Also

- [Testing Philosophy](testing-philosophy.md) — Why the framework is declarative by default
- [RunContext: BlockFeed & Node Control](node-control.md) — Node control within scenarios
- [Chaos Testing](chaos.md) — Restart-based chaos (scenario approach)
- [Scenario Builder Extensions](scenario-builder-ext-patterns.md) — Extending the declarative model

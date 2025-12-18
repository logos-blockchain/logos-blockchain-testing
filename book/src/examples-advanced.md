# Advanced Examples

Realistic advanced scenarios demonstrating framework capabilities for production testing.

**Adapt from Complete Source:**
- [compose_runner.rs](https://github.com/logos-blockchain/logos-blockchain-testing/blob/master/examples/src/bin/compose_runner.rs) — Compose examples with workloads
- [k8s_runner.rs](https://github.com/logos-blockchain/logos-blockchain-testing/blob/master/examples/src/bin/k8s_runner.rs) — K8s production patterns
- [Chaos testing patterns](https://github.com/logos-blockchain/logos-blockchain-testing/blob/master/testing-framework/workflows/src/workloads/chaos.rs) — Node control implementation

## Summary

| Example | Topology | Workloads | Deployer | Key Feature |
|---------|----------|-----------|----------|-------------|
| Load Progression | 3 validators + 2 executors | Increasing tx rate | Compose | Dynamic load testing |
| Sustained Load | 4 validators + 2 executors | High tx + DA rate | Compose | Stress testing |
| Aggressive Chaos | 4 validators + 2 executors | Frequent restarts + traffic | Compose | Resilience validation |

## Load Progression Test

Test consensus under progressively increasing transaction load:

```rust
use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_compose::ComposeDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn load_progression_test() -> Result<()> {
    for rate in [5, 10, 20, 30] {
        println!("Testing with rate: {}", rate);

        let mut plan =
            ScenarioBuilder::topology_with(|t| t.network_star().validators(3).executors(2))
                .wallets(50)
                .transactions_with(|txs| txs.rate(rate).users(20))
                .expect_consensus_liveness()
                .with_run_duration(Duration::from_secs(60))
                .build();

        let deployer = ComposeDeployer::default();
        let runner = deployer.deploy(&plan).await?;
        let _handle = runner.run(&mut plan).await?;
    }

    Ok(())
}
```

**When to use:** Finding the maximum sustainable transaction rate for a given topology.

## Sustained Load Test

Run high transaction and DA load for extended duration:

```rust
use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_compose::ComposeDeployer;
use testing_framework_workflows::ScenarioBuilderExt;

pub async fn sustained_load_test() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(4).executors(2))
        .wallets(100)
        .transactions_with(|txs| txs.rate(15).users(50))
        .da_with(|da| da.channel_rate(2).blob_rate(3))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(300))
        .build();

    let deployer = ComposeDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}
```

**When to use:** Validating stability under continuous high load over extended periods.

## Aggressive Chaos Test

Frequent node restarts with active traffic:

```rust
use std::time::Duration;

use anyhow::Result;
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_compose::ComposeDeployer;
use testing_framework_workflows::{ChaosBuilderExt, ScenarioBuilderExt};

pub async fn aggressive_chaos_test() -> Result<()> {
    let mut plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(4).executors(2))
        .enable_node_control()
        .wallets(50)
        .transactions_with(|txs| txs.rate(10).users(20))
        .chaos_with(|c| {
            c.restart()
                .min_delay(Duration::from_secs(10))
                .max_delay(Duration::from_secs(20))
                .target_cooldown(Duration::from_secs(15))
                .apply()
        })
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(180))
        .build();

    let deployer = ComposeDeployer::default();
    let runner = deployer.deploy(&plan).await?;
    let _handle = runner.run(&mut plan).await?;

    Ok(())
}
```

**When to use:** Validating recovery and liveness under aggressive failure conditions.

**Note:** Requires `ComposeDeployer` for node control support.

## Extension Ideas

These scenarios require custom implementations but demonstrate framework extensibility:

### Mempool & Transaction Handling

#### Transaction Propagation & Inclusion Test

**Concept:** Submit the same batch of independent transactions to different nodes in randomized order/offsets, then verify all transactions are included and final state matches across nodes.

**Requirements:**
- **Custom workload:** Generates a fixed batch of transactions and submits the same set to different nodes via `ctx.node_clients()`, with randomized submission order and timing offsets per node
- **Custom expectation:** Verifies all transactions appear in blocks (order may vary), final state matches across all nodes (compare balances or state roots), and no transactions are dropped

**Why useful:** Exercises mempool propagation, proposer fairness, and transaction inclusion guarantees under realistic race conditions. Tests that the protocol maintains consistency regardless of which node receives transactions first.

**Implementation notes:** Requires both a custom `Workload` implementation (to submit same transactions to multiple nodes with jitter) and a custom `Expectation` implementation (to verify inclusion and state consistency).

#### Cross-Validator Mempool Divergence & Convergence

**Concept:** Drive different transaction subsets into different validators (or differing arrival orders) to create temporary mempool divergence, then verify mempools/blocks converge to contain the union (no permanent divergence).

**Requirements:**
- **Custom workload:** Targets specific nodes via `ctx.node_clients()` with disjoint or jittered transaction batches
- **Custom expectation:** After a convergence window, verifies that all transactions appear in blocks (order may vary) or that mempool contents converge across nodes
- Run normal workloads during convergence period

**Expectations:**
- Temporary mempool divergence is acceptable (different nodes see different transactions initially)
- After convergence window, all transactions appear in blocks or mempools converge
- No transactions are permanently dropped despite initial divergence
- Mempool gossip/reconciliation mechanisms work correctly

**Why useful:** Exercises mempool gossip and reconciliation under uneven input or latency. Ensures no node "drops" transactions seen elsewhere, validating that mempool synchronization mechanisms correctly propagate transactions across the network even when they arrive at different nodes in different orders.

**Implementation notes:** Requires both a custom `Workload` implementation (to inject disjoint/jittered batches per node) and a custom `Expectation` implementation (to verify mempool convergence or block inclusion). Uses existing `ctx.node_clients()` capability—no new infrastructure needed.

#### Adaptive Mempool Pressure Test

**Concept:** Ramp transaction load over time to observe mempool growth, fee prioritization/eviction, and block saturation behavior, detecting performance regressions and ensuring backpressure/eviction work under increasing load.

**Requirements:**
- **Custom workload:** Steadily increases transaction rate over time (optional: use fee tiers)
- **Custom expectation:** Monitors mempool size, evictions, and throughput (blocks/txs per slot), flagging runaway growth or stalls
- Run for extended duration to observe pressure buildup

**Expectations:**
- Mempool size grows predictably with load (not runaway growth)
- Fee prioritization/eviction mechanisms activate under pressure
- Block saturation behavior is acceptable (blocks fill appropriately)
- Throughput (blocks/txs per slot) remains stable or degrades gracefully
- No stalls or unbounded mempool growth

**Why useful:** Detects performance regressions in mempool management. Ensures backpressure and eviction mechanisms work correctly under increasing load, preventing memory exhaustion or unbounded growth. Validates that fee prioritization correctly selects high-value transactions when mempool is full.

**Implementation notes:** Can be built with current workload model (ramping rate). Requires custom `Expectation` implementation that reads mempool metrics (via node HTTP APIs or Prometheus) and monitors throughput to judge behavior. No new infrastructure needed—uses existing observability capabilities.

#### Invalid Transaction Fuzzing

**Concept:** Submit malformed transactions and verify they're rejected properly.

**Implementation approach:**
- Custom workload that generates invalid transactions (bad signatures, insufficient funds, malformed structure)
- Expectation verifies mempool rejects them and they never appear in blocks
- Test mempool resilience and filtering

**Why useful:** Ensures mempool doesn't crash or include invalid transactions under fuzzing.

### Network & Gossip

#### Gossip Latency Gradient Scenario

**Concept:** Test consensus robustness under skewed gossip delays by partitioning nodes into latency tiers (tier A ≈10ms, tier B ≈100ms, tier C ≈300ms) and observing propagation lag, fork rate, and eventual convergence.

**Requirements:**
- Partition nodes into three groups (tiers)
- Apply per-group network delay via chaos: `netem`/`iptables` in compose; NetworkPolicy + `netem` sidecar in k8s
- Run standard workload (transactions/block production)
- Optional: Remove delays at end to check recovery

**Expectations:**
- **Propagation:** Messages reach all tiers within acceptable bounds
- **Safety:** No divergent finalized heads; fork rate stays within tolerance
- **Liveness:** Chain keeps advancing; convergence after delays relaxed (if healed)

**Why useful:** Real networks have heterogeneous latency. This stress-tests proposer selection and fork resolution when some peers are "far" (high latency), validating that consensus remains safe and live under realistic network conditions.

**Current blocker:** Runner support for per-group delay injection (network delay via `netem`/`iptables`) is not present today. Would require new chaos plumbing in compose/k8s deployers to inject network delays per node group.

#### Byzantine Gossip Flooding (libp2p Peer)

**Concept:** Spin up a custom workload/sidecar that runs a libp2p host, joins the cluster's gossip mesh, and publishes a high rate of syntactically valid but useless/stale messages to selected topics, testing gossip backpressure, scoring, and queue handling under a "malicious" peer.

**Requirements:**
- Custom workload/sidecar that implements a libp2p host
- Join the cluster's gossip mesh as a peer
- Publish high-rate syntactically valid but useless/stale messages to selected gossip topics
- Run alongside normal workloads (transactions/block production)

**Expectations:**
- Gossip backpressure mechanisms prevent message flooding from overwhelming nodes
- Peer scoring correctly identifies and penalizes the malicious peer
- Queue handling remains stable under flood conditions
- Normal consensus operation continues despite malicious peer

**Why useful:** Tests Byzantine behavior (malicious peer) which is critical for consensus protocol robustness. More realistic than RPC spam since it uses the actual gossip protocol. Validates that gossip backpressure, peer scoring, and queue management correctly handle adversarial peers without disrupting consensus.

**Current blocker:** Requires adding gossip-capable helper (libp2p integration) to the framework. Would need a custom workload/sidecar implementation that can join the gossip mesh and inject messages. The rest of the scenario can use existing runners/workloads.

#### Network Partition Recovery

**Concept:** Test consensus recovery after network partitions.

**Requirements:**
- Needs `block_peer()` / `unblock_peer()` methods in `NodeControlHandle`
- Partition subsets of validators, wait, then restore connectivity
- Verify chain convergence after partition heals

**Why useful:** Tests the most realistic failure mode in distributed systems.

**Current blocker:** Node control doesn't yet support network-level actions (only process restarts).

### Time & Timing

#### Time-Shifted Blocks (Clock Skew Test)

**Concept:** Test consensus and timestamp handling when nodes run with skewed clocks (e.g., +1s, −1s, +200ms jitter) to surface timestamp validation issues, reorg sensitivity, and clock drift handling.

**Requirements:**
- Assign per-node time offsets (e.g., +1s, −1s, +200ms jitter)
- Run normal workload (transactions/block production)
- Observe whether blocks are accepted/propagated and the chain stays consistent

**Expectations:**
- Blocks with skewed timestamps are handled correctly (accepted or rejected per protocol rules)
- Chain remains consistent across nodes despite clock differences
- No unexpected reorgs or chain splits due to timestamp validation issues

**Why useful:** Clock skew is a common real-world issue in distributed systems. This validates that consensus correctly handles timestamp validation and maintains safety/liveness when nodes have different clock offsets, preventing timestamp-based attacks or failures.

**Current blocker:** Runner ability to skew per-node clocks (e.g., privileged containers with `libfaketime`/`chrony` or time-offset netns) is not available today. Would require a new chaos/time-skew hook in deployers to inject clock offsets per node.

#### Block Timing Consistency

**Concept:** Verify block production intervals stay within expected bounds.

**Implementation approach:**
- Custom expectation that consumes `BlockFeed`
- Collect block timestamps during run
- Assert intervals are within `(slot_duration * active_slot_coeff) ± tolerance`

**Why useful:** Validates consensus timing under various loads.

### Topology & Membership

#### Dynamic Topology (Churn) Scenario

**Concept:** Nodes join and leave mid-run (new identities/addresses added; some nodes permanently removed) to exercise peer discovery, bootstrapping, reputation, and load balancing under churn.

**Requirements:**
- Runner must be able to spin up new nodes with fresh keys/addresses at runtime
- Update peer lists and bootstraps dynamically as nodes join/leave
- Optionally tear down nodes permanently (not just restart)
- Run normal workloads (transactions/block production) during churn

**Expectations:**
- New nodes successfully discover and join the network
- Peer discovery mechanisms correctly handle dynamic topology changes
- Reputation systems adapt to new/removed peers
- Load balancing adjusts to changing node set
- Consensus remains safe and live despite topology churn

**Why useful:** Real networks experience churn (nodes joining/leaving). Unlike restarts (which preserve topology), churn changes the actual topology size and peer set, testing how the protocol handles dynamic membership. This exercises peer discovery, bootstrapping, reputation systems, and load balancing under realistic conditions.

**Current blocker:** Runner support for dynamic node addition/removal at runtime is not available today. Chaos today only restarts existing nodes; churn would require the ability to spin up new nodes with fresh identities/addresses, update peer lists/bootstraps dynamically, and permanently remove nodes. Would need new topology management capabilities in deployers.

### API & External Interfaces

#### API DoS/Stress Test

**Concept:** Adversarial workload floods node HTTP/WS APIs with high QPS and malformed/bursty requests; expectation checks nodes remain responsive or rate-limit without harming consensus.

**Requirements:**
- **Custom workload:** Targets node HTTP/WS API endpoints with mixed valid/invalid requests at high rate
- **Custom expectation:** Monitors error rates, latency, and confirms block production/liveness unaffected
- Run alongside normal workloads (transactions/block production)

**Expectations:**
- Nodes remain responsive or correctly rate-limit under API flood
- Error rates/latency are acceptable (rate limiting works)
- Block production/liveness unaffected by API abuse
- Consensus continues normally despite API stress

**Why useful:** Validates API hardening under abuse and ensures control/telemetry endpoints don't destabilize the node. Tests that API abuse is properly isolated from consensus operations, preventing DoS attacks on API endpoints from affecting blockchain functionality.

**Implementation notes:** Requires custom `Workload` implementation that directs high-QPS traffic to node APIs (via `ctx.node_clients()` or direct HTTP clients) and custom `Expectation` implementation that monitors API responsiveness metrics and consensus liveness. Uses existing node API access—no new infrastructure needed.

### State & Correctness

#### Wallet Balance Verification

**Concept:** Track wallet balances and verify state consistency.

**Description:** After transaction workload completes, query all wallet balances via node API and verify total supply is conserved. Requires tracking initial state, submitted transactions, and final balances. Validates that the ledger maintains correctness under load (no funds lost or created). This is a **state assertion** expectation that checks correctness, not just liveness.

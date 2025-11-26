# Nomos Testing Framework — Complete Reference

> **GitBook Structure Note**: This document is organized with `<!-- FILE: path/to/file.md -->` markers indicating how to split for GitBook deployment.

---

<!-- FILE: README.md -->

# Nomos Testing Framework

A purpose-built toolkit for exercising Nomos in realistic, multi-node environments.

## Quick Links

- [5-Minute Quickstart](#5-minute-quickstart) — Get running immediately
- [Foundations](#part-i--foundations) — Core concepts and architecture
- [User Guide](#part-ii--user-guide) — Authoring and running scenarios
- [Developer Reference](#part-iii--developer-reference) — Extending the framework
- [Recipes](#part-v--scenario-recipes) — Copy-paste runnable examples

## Reading Guide by Role

| If you are... | Start with... | Then read... |
|---------------|---------------|--------------|
| **Protocol/Core Engineer** | Quickstart → Testing Philosophy | Workloads & Expectations → Recipes |
| **Infra/DevOps** | Quickstart → Runners | Operations → Configuration Sync → Troubleshooting |
| **Test Designer** | Quickstart → Authoring Scenarios | DSL Cheat Sheet → Recipes → Extending |

## Prerequisites

This book assumes:

- Rust competency (async/await, traits, cargo)
- Basic familiarity with Nomos architecture (validators, executors, DA)
- Docker knowledge (for Compose runner)
- Optional: Kubernetes access (for K8s runner)

---

<!-- FILE: quickstart.md -->

# 5-Minute Quickstart

Get a scenario running in under 5 minutes.

## Step 1: Clone and Build

```bash
# Clone the testing framework (assumes nomos-node sibling checkout)
# Note: If the testing framework lives inside the main Nomos monorepo,
# adjust the clone URL and paths accordingly.
git clone https://github.com/logos-co/nomos-testing.git
cd nomos-testing

# Build the testing framework crates
cargo build -p testing-framework-core -p testing-framework-workflows
```

> **Build modes**: Node binaries use `--release` for realistic performance. Framework crates use debug for faster iteration. For pure development speed, you can build everything in debug mode.

## Step 2: Run the Simplest Scenario

```bash
# Run a local 2-validator smoke test
cargo test --package tests-workflows --test local_runner -- local_runner_mixed_workloads --nocapture
```

## Step 3: What Good Output Looks Like

```
running 1 test
[INFO] Spawning validator 0 on port 18800
[INFO] Spawning validator 1 on port 18810
[INFO] Waiting for network readiness...
[INFO] Network ready: all peers connected
[INFO] Waiting for membership readiness...
[INFO] Membership ready for session 0
[INFO] Starting workloads...
[INFO] Transaction workload submitting at 5 tx/block
[INFO] DA workload: channel inscription submitted
[INFO] Block 1 observed: 3 transactions
[INFO] Block 2 observed: 5 transactions
...
[INFO] Workloads complete, evaluating expectations
[INFO] consensus_liveness: target=8, observed heights=[12, 11] ✓
[INFO] tx_inclusion_expectation: 42/50 included (84%) ✓
test local_runner_mixed_workloads ... ok
```

## Step 4: What Failure Looks Like

```
[ERROR] consensus_liveness violated (target=8):
- validator-0 height 2 below target 8
- validator-1 height 3 below target 8

test local_runner_mixed_workloads ... FAILED
```

Common causes: run duration too short, readiness not complete, node crashed.

## Step 5: Modify a Scenario

Open `tests/workflows/tests/local_runner.rs`:

```rust
// Change this:
const RUN_DURATION: Duration = Duration::from_secs(60);

// To this for a longer run:
const RUN_DURATION: Duration = Duration::from_secs(120);

// Or change validator count:
const VALIDATORS: usize = 3;  // was 2
```

Re-run:

```bash
cargo test --package tests-workflows --test local_runner -- --nocapture
```

You're now ready to explore the framework!

---

<!-- FILE: foundations/introduction.md -->

# Part I — Foundations

## Introduction

The Nomos Testing Framework solves the gap between small, isolated unit tests and full-system validation by letting teams:

1. **Describe** a cluster layout (topology)
2. **Drive** meaningful traffic (workloads)
3. **Assert** outcomes (expectations)

...all in one coherent, portable plan (a `Scenario` in code terms).

### Why Multi-Node Testing?

Many Nomos behaviors only emerge when multiple roles interact:

```
┌─────────────────────────────────────────────────────────────────┐
│                BEHAVIORS REQUIRING MULTI-NODE                   │
├─────────────────────────────────────────────────────────────────┤
│ • Block progression across validators                           │
│ • Data availability sampling and dispersal                      │
│ • Consensus under network partitions                            │
│ • Liveness recovery after node restarts                         │
│ • Transaction propagation and inclusion                         │
│ • Membership and session transitions                            │
└─────────────────────────────────────────────────────────────────┘
```

Unit tests can't catch these. This framework makes multi-node checks declarative, observable, and repeatable.

### Target Audience

| Role | Primary Concerns |
|------|------------------|
| **Protocol Engineers** | Consensus correctness, DA behavior, block progression |
| **Infrastructure/DevOps** | Runners, CI integration, logs, failure triage |
| **QA/Test Designers** | Scenario composition, workload tuning, coverage |

---

<!-- FILE: foundations/architecture.md -->

## Architecture Overview

The framework follows a clear pipeline:

```
┌──────────┐    ┌──────────┐    ┌──────────┐    ┌──────────┐    ┌─────────────┐
│ TOPOLOGY │───▶│ SCENARIO │───▶│  RUNNER  │───▶│ WORKLOADS│───▶│EXPECTATIONS │
│          │    │          │    │          │    │          │    │             │
│ Shape    │    │ Assemble │    │ Deploy & │    │ Drive    │    │ Verify      │
│ cluster  │    │ plan     │    │ wait     │    │ traffic  │    │ outcomes    │
└──────────┘    └──────────┘    └──────────┘    └──────────┘    └─────────────┘
```

### Component Responsibilities

| Component | Responsibility | Key Types |
|-----------|----------------|-----------|
| **Topology** | Declares cluster shape: node counts, network layout, DA parameters | `TopologyConfig`, `GeneratedTopology`, `TopologyBuilder` |
| **Scenario** | Assembles topology + workloads + expectations + duration | `Scenario<Caps>`, `ScenarioBuilder` |
| **Runner** | Deploys to environment, waits for readiness, provides `RunContext` | `Runner`, `LocalDeployer`, `ComposeRunner`, `K8sRunner` |
| **Workloads** | Generate traffic/conditions during the run | `Workload` trait, `TransactionWorkload`, `DaWorkload`, `RandomRestartWorkload` |
| **Expectations** | Judge success/failure after workloads complete | `Expectation` trait, `ConsensusLiveness`, `TxInclusionExpectation` |

### Type Flow Diagram

```
TopologyConfig
    │
    │ TopologyBuilder::new()
    ▼
TopologyBuilder ──.build()──▶ GeneratedTopology
                                    │
                                    │ contains
                                    ▼
                            GeneratedNodeConfig[]
                                    │
                                    │ Runner spawns
                                    ▼
                              Topology (live nodes)
                                    │
                                    │ provides
                                    ▼
                              NodeClients
                                    │
                                    │ wrapped in
                                    ▼
                              RunContext
```

```
ScenarioBuilder
    │
    │ .with_workload() / .with_expectation() / .with_run_duration()
    │
    │ .build()
    ▼
Scenario<Caps>
    │
    │ Deployer::deploy()
    ▼
Runner
    │
    │ .run(&mut scenario)
    ▼
RunHandle (success) or ScenarioError (failure)
```

---

<!-- FILE: foundations/testing-philosophy.md -->

## Testing Philosophy

### Core Principles

1. **Declarative over imperative**
   - Describe desired state, let framework orchestrate
   - Scenarios are data, not scripts

2. **Observable health signals**
   - Prefer liveness/inclusion signals over internal debug state
   - If users can't see it, don't assert on it

3. **Determinism first**
   - Fixed topologies and traffic rates by default
   - Variability is opt-in (chaos workloads)

4. **Protocol time, not wall time**
   - Reason in blocks and slots
   - Reduces host speed dependence

5. **Minimum run window**
   - Always allow enough blocks for meaningful assertions
   - Framework enforces minimum 2 blocks

6. **Chaos with intent**
   - Chaos workloads for resilience testing only
   - Avoid chaos in basic functional smoke tests; reserve it for dedicated resilience scenarios

### Testing Spectrum

```
┌────────────────────────────────────────────────────────────────┐
│                    WHERE THIS FRAMEWORK FITS                   │
├──────────────┬────────────────────┬────────────────────────────┤
│  UNIT TESTS  │  INTEGRATION       │  MULTI-NODE SCENARIOS      │
│              │                    │                            │
│  Fast        │  Single process    │  ◀── THIS FRAMEWORK        │
│  Isolated    │  Mock network      │                            │
│  Deterministic│  No real timing   │  Real networking           │
│              │                    │  Protocol timing           │
│  ~1000s/sec  │  ~100s/sec         │  ~1-10/hour                │
└──────────────┴────────────────────┴────────────────────────────┘
```

---

<!-- FILE: foundations/lifecycle.md -->

## Scenario Lifecycle

### Phase Overview

```
┌─────────┐   ┌─────────┐   ┌───────────┐   ┌─────────┐   ┌──────────┐   ┌──────────┐   ┌─────────┐
│  PLAN   │──▶│ DEPLOY  │──▶│ READINESS │──▶│  DRIVE  │──▶│ COOLDOWN │──▶│ EVALUATE │──▶│ CLEANUP │
└─────────┘   └─────────┘   └───────────┘   └─────────┘   └──────────┘   └──────────┘   └─────────┘
```

### Detailed Timeline

```
Time ──────────────────────────────────────────────────────────────────────▶

     │ PLAN          │ DEPLOY        │ READY    │ WORKLOADS      │COOL│ EVAL │
     │               │               │          │                │DOWN│      │
     │ Build         │ Spawn         │ Network  │ Traffic runs   │    │Check │
     │ scenario      │ nodes         │ DA       │ Blocks produce │ 5× │ all  │
     │               │ (local/       │ Member   │                │blk │expect│
     │               │ docker/k8s)   │ ship     │                │    │      │
     │               │               │          │                │    │      │
     ▼               ▼               ▼          ▼                ▼    ▼      ▼
   t=0            t=5s           t=30s       t=35s            t=95s t=100s t=105s
                                                                          │
                                                              (example    │
                                                               60s run)   ▼
                                                                       CLEANUP
```

### Phase Details

| Phase | What Happens | Code Entry Point |
|-------|--------------|------------------|
| **Plan** | Declare topology, attach workloads/expectations, set duration | `ScenarioBuilder::build()` |
| **Deploy** | Runner provisions environment | `deployer.deploy(&scenario)` |
| **Readiness** | Wait for network peers, DA balancer, membership | `wait_network_ready()`, `wait_membership_ready()`, `wait_da_balancer_ready()` |
| **Drive** | Workloads run concurrently for configured duration | `workload.start(ctx)` inside `Runner::run_workloads()` |
| **Cooldown** | Stabilization period (5× block interval, 30s min if chaos used) | Automatic in `Runner::cooldown()` |
| **Evaluate** | All expectations run; failures **aggregated** (not short-circuited) | `expectation.evaluate(ctx)` |
| **Cleanup** | Resources reclaimed via `CleanupGuard` | `Drop` impl on `Runner` |

### Readiness Phases (Detail)

Runners perform three distinct readiness checks:

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│ NETWORK         │────▶│ MEMBERSHIP      │────▶│ DA BALANCER     │
│                 │     │                 │     │                 │
│ libp2p peers    │     │ Session 0       │     │ Dispersal peers │
│ connected       │     │ assignments     │     │ available       │
│                 │     │ propagated      │     │                 │
│ Timeout: 60s    │     │ Timeout: 60s    │     │ Timeout: 60s    │
└─────────────────┘     └─────────────────┘     └─────────────────┘
```

---

<!-- FILE: guide/authoring-scenarios.md -->

# Part II — User Guide

## Authoring Scenarios

### The 5-Step Process

```
┌─────────────────────────────────────────────────────────────────┐
│                    SCENARIO AUTHORING FLOW                      │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  1. SHAPE TOPOLOGY          2. ATTACH WORKLOADS                 │
│     ┌─────────────┐            ┌─────────────┐                  │
│     │ Validators  │            │ Transactions│                  │
│     │ Executors   │            │ DA blobs    │                  │
│     │ Network     │            │ Chaos       │                  │
│     │ DA params   │            └─────────────┘                  │
│     └─────────────┘                                             │
│                                                                 │
│  3. DEFINE EXPECTATIONS     4. SET DURATION                     │
│     ┌─────────────┐            ┌─────────────┐                  │
│     │ Liveness    │            │ See duration│                  │
│     │ Inclusion   │            │ heuristics  │                  │
│     │ Custom      │            │ table below │                  │
│     └─────────────┘            └─────────────┘                  │
│                                                                 │
│  5. CHOOSE RUNNER                                               │
│     ┌─────────┐ ┌─────────┐ ┌─────────┐                         │
│     │ Local   │ │ Compose │ │ K8s     │                         │
│     └─────────┘ └─────────┘ └─────────┘                         │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

### Duration Heuristics

Use protocol time (blocks), not wall time. With default 2-second slots and active slot coefficient of 0.9, expect roughly one block every ~2–3 seconds (subject to randomness). Individual topologies may override these defaults.

| Scenario Type | Min Blocks | Recommended Duration | Notes |
|---------------|------------|---------------------|-------|
| Smoke test | 5-10 | 30-60s | Quick validation |
| Tx throughput | 20-50 | 2-3 min | Capture steady state |
| DA + tx combined | 30-50 | 3-5 min | Observe interaction |
| Chaos/resilience | 50-100 | 5-10 min | Allow restart recovery |
| Long-run stability | 100+ | 10-30 min | Trend validation |

> **Note**: The framework enforces a minimum of 2 blocks. Very short durations are clamped automatically.

### Builder Pattern Overview

```rust
ScenarioBuilder::with_node_counts(validators, executors)
    // 1. Topology sub-builder
    .topology()
        .network_star()
        .validators(n)
        .executors(n)
        .apply()  // Returns to main builder
    
    // 2. Wallet seeding
    .wallets(user_count)
    
    // 3. Workload sub-builders
    .transactions()
        .rate(per_block)
        .users(actors)
        .apply()
    
    .da()
        .channel_rate(n)
        .blob_rate(n)
        .apply()
    
    // 4. Optional chaos (changes Caps type)
    .enable_node_control()
    .chaos_random_restart()
        .validators(true)
        .executors(true)
        .min_delay(Duration)
        .max_delay(Duration)
        .target_cooldown(Duration)
        .apply()
    
    // 5. Duration and expectations
    .with_run_duration(duration)
    .expect_consensus_liveness()
    
    // 6. Build
    .build()
```

---

<!-- FILE: guide/workloads.md -->

## Workloads

Workloads generate traffic and conditions during a scenario run.

### Available Workloads

| Workload | Purpose | Key Config | Bundled Expectation |
|----------|---------|------------|---------------------|
| **Transaction** | Submit transactions at configurable rate | `rate`, `users` | `TxInclusionExpectation` |
| **DA** | Create channels, publish blobs | `channel_rate`, `blob_rate` | `DaWorkloadExpectation` |
| **Chaos** | Restart nodes randomly | `min_delay`, `max_delay`, `target_cooldown` | None (use `ConsensusLiveness`) |

### Transaction Workload

Submits user-level transactions at a configurable rate.

```rust
.transactions()
    .rate(5)      // 5 transactions per block opportunity
    .users(8)     // Use 8 distinct wallet actors
    .apply()
```

**Requires**: Seeded wallets (`.wallets(n)`)

### DA Workload

Drives data-availability paths: channel inscriptions and blob publishing.

```rust
.da()
    .channel_rate(1)  // 1 channel operation per block
    .blob_rate(1)     // 1 blob per channel
    .apply()
```

**Requires**: At least one executor for blob publishing.

### Chaos Workload

Triggers controlled node restarts to test resilience.

```rust
.enable_node_control()  // Required capability
.chaos_random_restart()
    .validators(true)           // Include validators
    .executors(true)            // Include executors
    .min_delay(Duration::from_secs(45))    // Min time between restarts
    .max_delay(Duration::from_secs(75))    // Max time between restarts
    .target_cooldown(Duration::from_secs(120))  // Per-node cooldown
    .apply()
```

**Safety behavior**: If only one validator is configured, the chaos workload automatically skips validator restarts to avoid halting consensus.

**Cooldown behavior**: After chaos workloads, the runner adds a minimum 30-second cooldown before evaluating expectations.

---

<!-- FILE: guide/expectations.md -->

## Expectations

Expectations are post-run assertions that judge success or failure.

### Available Expectations

| Expectation | Asserts | Default Tolerance |
|-------------|---------|-------------------|
| **ConsensusLiveness** | All validators reach minimum block height | 80% of expected blocks |
| **TxInclusionExpectation** | Submitted transactions appear in blocks | 50% inclusion ratio |
| **DaWorkloadExpectation** | Planned channels/blobs were included | 80% inclusion ratio |
| **PrometheusBlockProduction** | Prometheus metrics show block production | Exact minimum |

### ConsensusLiveness

The primary health check. Polls each validator's HTTP consensus info.

```rust
// With default 80% tolerance:
.expect_consensus_liveness()

// Or with specific minimum:
.with_expectation(ConsensusLiveness::with_minimum(10))

// Or with custom tolerance:
.with_expectation(ConsensusLiveness::with_tolerance(0.9))
```

> **Note for advanced users**: There are two `ConsensusLiveness` implementations in the codebase:
> - `testing_framework_workflows::ConsensusLiveness` — HTTP-based, checks heights via `consensus_info()` API. This is what `.expect_consensus_liveness()` uses.
> - `testing_framework_core::scenario::expectations::ConsensusLiveness` — Also HTTP-based but with different tolerance semantics.
> 
> There's also `PrometheusBlockProduction` in core for Prometheus-based metrics checks when telemetry is configured.

### Expectation Lifecycle

```
┌─────────────┐     ┌─────────────┐     ┌─────────────┐
│    init()   │────▶│start_capture│────▶│  evaluate() │
│             │     │    ()       │     │             │
│ Validate    │     │ Snapshot    │     │ Assert      │
│ prereqs     │     │ baseline    │     │ conditions  │
│             │     │ (optional)  │     │             │
└─────────────┘     └─────────────┘     └─────────────┘
     │                    │                    │
     ▼                    ▼                    ▼
  At build()         Before workloads     After workloads
```

### Common Expectation Mistakes

| Mistake | Why It Fails | Fix |
|---------|--------------|-----|
| Expecting inclusion too soon | Transactions need blocks to be included | Increase duration |
| Wall-clock timing assertions | Host speed varies | Use block counts via `RunMetrics` |
| Duration too short | Not enough blocks observed | Use duration heuristics table |
| Skipping `start_capture()` | Baseline not established | Implement if comparing before/after |
| Asserting on internal state | Framework can't observe it | Use `consensus_info()` or `BlockFeed` |

---

<!-- FILE: guide/blockfeed.md -->

## BlockFeed Deep Dive

The `BlockFeed` is the primary mechanism for observing block production during a run.

### What BlockFeed Provides

```rust
pub struct BlockFeed {
    // Subscribe to receive block notifications
    pub fn subscribe(&self) -> broadcast::Receiver<Arc<BlockRecord>>;
    
    // Access aggregate statistics
    pub fn stats(&self) -> Arc<BlockStats>;
}

pub struct BlockRecord {
    pub header: HeaderId,                    // Block header ID
    pub block: Arc<Block<SignedMantleTx>>,   // Full block with transactions
}

pub struct BlockStats {
    // Total transactions observed across all blocks
    pub fn total_transactions(&self) -> u64;
}
```

### How It Works

```
┌────────────────┐     ┌────────────────┐     ┌────────────────┐
│  BlockScanner  │────▶│   BlockFeed    │────▶│  Subscribers   │
│                │     │                │     │                │
│ Polls validator│     │ broadcast      │     │ Workloads      │
│ consensus_info │     │ channel        │     │ Expectations   │
│ every 1 second │     │ (1024 buffer)  │     │                │
│                │     │                │     │                │
│ Fetches blocks │     │ Records stats  │     │                │
│ via storage_   │     │                │     │                │
│ block()        │     │                │     │                │
└────────────────┘     └────────────────┘     └────────────────┘
```

### Using BlockFeed in Workloads

```rust
async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
    let mut receiver = ctx.block_feed().subscribe();
    
    loop {
        match receiver.recv().await {
            Ok(record) => {
                // Process block
                let height = record.block.header().slot().into();
                let tx_count = record.block.transactions().len();
                
                // Check for specific transactions
                for tx in record.block.transactions() {
                    // ... examine transaction
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                // Fell behind, n messages skipped
                continue;
            }
            Err(broadcast::error::RecvError::Closed) => {
                return Err("block feed closed".into());
            }
        }
    }
}
```

### Using BlockFeed in Expectations

```rust
async fn start_capture(&mut self, ctx: &RunContext) -> Result<(), DynError> {
    let mut receiver = ctx.block_feed().subscribe();
    let observed = Arc::new(Mutex::new(HashSet::new()));
    let observed_clone = Arc::clone(&observed);
    
    // Spawn background task to collect observations
    tokio::spawn(async move {
        while let Ok(record) = receiver.recv().await {
            // Record what we observe
            let mut guard = observed_clone.lock().unwrap();
            for tx in record.block.transactions() {
                guard.insert(tx.hash());
            }
        }
    });
    
    self.observed = Some(observed);
    Ok(())
}

async fn evaluate(&mut self, ctx: &RunContext) -> Result<(), DynError> {
    let observed = self.observed.as_ref().ok_or("not captured")?;
    let guard = observed.lock().unwrap();
    
    // Compare observed vs expected
    if guard.len() < self.expected_count {
        return Err(format!(
            "insufficient inclusions: {} < {}",
            guard.len(), self.expected_count
        ).into());
    }
    Ok(())
}
```

---

<!-- FILE: runners/local.md -->

# Runner: Local

Runs node binaries as local processes on the host.

## What It Does

- Spawns validators/executors directly on the host with ephemeral data dirs.
- Binds HTTP/libp2p ports on localhost; no containers involved.
- Fastest feedback loop; best for unit-level scenarios and debugging.

## Prerequisites

- Rust toolchain installed.
- No ports in use on the default ranges (see runner config if you need to override).

## How to Run

```bash
cargo test -p tests-workflows --test local_runner -- local_runner_mixed_workloads --nocapture
```

Adjust validator/executor counts inside the test file or via the scenario builder.

## Troubleshooting

- Port already in use → change base ports in the test or stop the conflicting process.
- Slow start on first run → binaries need to be built; reruns are faster.
- No blocks → ensure workloads enabled and duration long enough (≥60s default).

---

<!-- FILE: runners/compose.md -->

# Runner: Docker Compose

Runs validators/executors in Docker containers using docker-compose.

## What It Does

- Builds/pulls the node image, then creates a network and one container per role.
- Uses Compose health checks for readiness, then runs workloads/expectations.
- Cleans up containers and network unless preservation is requested.

## Prerequisites

- Docker with the Compose plugin.
- Built node image available locally (default `nomos-testnet:local`).
  - Build from repo root: `testnet/scripts/build_test_image.sh`
- Optional env vars:
  - `NOMOS_TESTNET_IMAGE` (override tag)
  - `COMPOSE_NODE_PAIRS=1x1` (validators x executors)
  - `COMPOSE_RUNNER_PRESERVE=1` to keep the stack for inspection

## How to Run

```bash
POL_PROOF_DEV_MODE=true COMPOSE_NODE_PAIRS=1x1 \
cargo test -p tests-workflows compose_runner_mixed_workloads -- --nocapture
```

## Troubleshooting

- Image not found → set `NOMOS_TESTNET_IMAGE` to a built/pulled tag.
- Peers not connecting → inspect `docker compose logs` for validator/executor.
- Stack left behind → `docker compose -p <project> down` and remove the network.

---

<!-- FILE: runners/k8s.md -->

# Runner: Kubernetes

Deploys validators/executors as a Helm release into the current Kubernetes context.

## What It Does

- Builds/pulls the node image, packages Helm assets, installs into a unique namespace.
- Waits for pod readiness and validator HTTP endpoint, then drives workloads.
- Tears down the namespace unless preservation is requested.

## Prerequisites

- kubectl and Helm on PATH; a running Kubernetes cluster/context (e.g., Docker Desktop, kind).
- Docker buildx to build the node image for your arch.
- Built image tag exported:
  - Build: `testnet/scripts/build_test_image.sh` (default tag `nomos-testnet:local`)
  - Export: `export NOMOS_TESTNET_IMAGE=nomos-testnet:local`
- Optional: `K8S_RUNNER_PRESERVE=1` to keep the namespace for debugging.

## How to Run

```bash
NOMOS_TESTNET_IMAGE=nomos-testnet:local \
cargo test -p tests-workflows demo_k8s_runner_tx_workload -- --nocapture
```

## Troubleshooting

- Timeout waiting for validator HTTP → check pod logs: `kubectl logs -n <ns> deploy/validator`.
- No peers/tx inclusion → inspect rendered `/config.yaml` in the pod and cfgsync logs.
- Cleanup stuck → `kubectl delete namespace <ns>` from the preserved namespace name.

---

<!-- FILE: guide/runners.md -->

## Runners

Runners deploy scenarios to different environments.

### Runner Decision Matrix

| Goal | Recommended Runner | Why |
|------|-------------------|-----|
| Fast local iteration | `LocalDeployer` | No container overhead |
| Reproducible e2e checks | `ComposeRunner` | Stable multi-node isolation |
| High fidelity / CI | `K8sRunner` | Real cluster behavior |
| Config validation only | Dry-run (future) | Catch errors before nodes |

### Runner Comparison

| Aspect | LocalDeployer | ComposeRunner | K8sRunner |
|--------|---------------|---------------|-----------|
| **Speed** | ⚡ Fastest | 🔄 Medium | 🏗️ Slowest |
| **Setup** | Binaries only | Docker daemon | Cluster access |
| **Isolation** | Process-level | Container-level | Pod-level |
| **Port discovery** | Direct | Auto via Docker | NodePort |
| **Node control** | Full | Via container restart | Via pod restart |
| **Observability** | Local files | Container logs | Prometheus + logs |
| **CI suitability** | Dev only | Good | Best |

### LocalDeployer

Spawns nodes as host processes.

```rust
let deployer = LocalDeployer::default();
// Or skip membership check for faster startup:
let deployer = LocalDeployer::new().with_membership_check(false);

let runner = deployer.deploy(&scenario).await?;
```

### ComposeRunner

Starts nodes in Docker containers via Docker Compose.

```rust
let deployer = ComposeRunner::default();
let runner = deployer.deploy(&scenario).await?;
```

**Uses Configuration Sync (cfgsync)** — see Operations section.

### K8sRunner

Deploys to a Kubernetes cluster.

```rust
let deployer = K8sRunner::new();
let runner = match deployer.deploy(&scenario).await {
    Ok(r) => r,
    Err(K8sRunnerError::ClientInit { source }) => {
        // Cluster unavailable
        return;
    }
    Err(e) => panic!("deployment failed: {e}"),
};
```

---

<!-- FILE: guide/operations.md -->

## Operations

### Prerequisites Checklist

```
□ nomos-node checkout available (sibling directory)
□ Binaries built: cargo build -p nomos-node -p nomos-executor
□ Runner platform ready:
  □ Local: binaries in target/debug/
  □ Compose: Docker daemon running
  □ K8s: kubectl configured, cluster accessible
□ KZG prover assets fetched (for DA scenarios)
□ Ports available (default ranges: 18800+, 4400 for cfgsync)
```

### Environment Variables

| Variable | Effect | Default |
|----------|--------|---------|
| `SLOW_TEST_ENV=true` | 2× timeout multiplier for all readiness checks | `false` |
| `NOMOS_TESTS_TRACING=true` | Enable debug tracing output | `false` |
| `NOMOS_TESTS_KEEP_LOGS=1` | Preserve temp directories after run | Delete |
| `NOMOS_TESTNET_IMAGE` | Docker image for Compose/K8s runners | `nomos-testnet:local` |
| `COMPOSE_RUNNER_PRESERVE=1` | Keep Compose resources after run | Delete |
| `TEST_FRAMEWORK_PROMETHEUS_PORT` | Host port for Prometheus (Compose) | `9090` |

### Configuration Synchronization (cfgsync)

When running in Docker Compose or Kubernetes, the framework uses **dynamic configuration injection** instead of static config files.

```
┌─────────────────┐                    ┌─────────────────┐
│  RUNNER HOST    │                    │  NODE CONTAINER │
│                 │                    │                 │
│ ┌─────────────┐ │   HTTP :4400       │ ┌─────────────┐ │
│ │ cfgsync     │◀├───────────────────┤│ cfgsync     │ │
│ │ server      │ │                    │ │ client      │ │
│ │             │ │  1. Request config │ │             │ │
│ │ Holds       │ │  2. Receive YAML   │ │ Fetches     │ │
│ │ generated   │ │  3. Start node     │ │ config at   │ │
│ │ topology    │ │                    │ │ startup     │ │
│ └─────────────┘ │                    │ └─────────────┘ │
└─────────────────┘                    └─────────────────┘
```

**Why cfgsync?**
- Handles dynamic port discovery
- Injects cryptographic keys
- Supports topology changes without rebuilding images

**Troubleshooting cfgsync:**

| Symptom | Cause | Fix |
|---------|-------|-----|
| Containers stuck at startup | cfgsync server unreachable | Check port 4400 is not blocked |
| "connection refused" in logs | Server not started | Verify runner started cfgsync |
| Config mismatch errors | Stale cfgsync template | Clean temp directories |

---

<!-- FILE: reference/troubleshooting.md -->

# Part IV — Reference

## Troubleshooting

### Error Messages and Fixes

#### Readiness Timeout

```
Error: readiness probe failed: timed out waiting for network readiness:
  validator#0@18800: 0 peers (expected 1)
  validator#1@18810: 0 peers (expected 1)
```

**Causes:**
- Nodes not fully started
- Network configuration mismatch
- Ports blocked

**Fixes:**
- Set `SLOW_TEST_ENV=true` for 2× timeout
- Check node logs for startup errors
- Verify ports are available

#### Consensus Liveness Violation

```
Error: expectations failed:
consensus liveness violated (target=8):
- validator-0 height 2 below target 8
- validator-1 height 3 below target 8
```

**Causes:**
- Run duration too short
- Node crashed during run
- Consensus stalled

**Fixes:**
- Increase `with_run_duration()`
- Check node logs for panics
- Verify network connectivity

#### Transaction Inclusion Below Threshold

```
Error: tx_inclusion_expectation: observed 15 below required 25
```

**Causes:**
- Wallet not seeded
- Transaction rate too high
- Mempool full

**Fixes:**
- Add `.wallets(n)` to scenario
- Reduce `.rate()` in transaction workload
- Increase duration for more blocks

#### Chaos Workload No Targets

```
Error: chaos restart workload has no eligible targets
```

**Causes:**
- No validators or executors configured
- Only one validator (skipped for safety)
- Chaos disabled for both roles

**Fixes:**
- Add more validators (≥2) for chaos
- Enable `.executors(true)` if executors present
- Use different workload for single-validator tests

#### BlockFeed Closed

```
Error: block feed closed while waiting for channel operations
```

**Causes:**
- Source validator crashed
- Network partition
- Run ended prematurely

**Fixes:**
- Check validator logs
- Increase run duration
- Verify readiness completed

### Log Locations

| Runner | Log Location |
|--------|--------------|
| Local | Temp directory (printed at startup), or set `NOMOS_TESTS_KEEP_LOGS=1` |
| Compose | `docker logs <container_name>` |
| K8s | `kubectl logs <pod_name>` |

### Debugging Flow

```
┌─────────────────┐
│ Scenario fails  │
└────────┬────────┘
         ▼
┌────────────────────────────────────────┐
│ 1. Check error message category        │
│    - Readiness? → Check startup logs   │
│    - Workload? → Check workload config │
│    - Expectation? → Check assertions   │
└────────┬───────────────────────────────┘
         ▼
┌────────────────────────────────────────┐
│ 2. Check node logs                     │
│    - Panics? → Bug in node             │
│    - Connection errors? → Network      │
│    - Config errors? → cfgsync issue    │
└────────┬───────────────────────────────┘
         ▼
┌────────────────────────────────────────┐
│ 3. Reproduce with tracing              │
│    NOMOS_TESTS_TRACING=true cargo test │
└────────┬───────────────────────────────┘
         ▼
┌────────────────────────────────────────┐
│ 4. Simplify scenario                   │
│    - Reduce validators                 │
│    - Remove workloads one by one       │
│    - Increase duration                 │
└────────────────────────────────────────┘
```

---

<!-- FILE: reference/dsl-cheat-sheet.md -->

## DSL Cheat Sheet

### Complete Builder Reference

```rust
// ═══════════════════════════════════════════════════════════════
// TOPOLOGY
// ═══════════════════════════════════════════════════════════════

ScenarioBuilder::with_node_counts(validators, executors)

    .topology()
        .network_star()              // Star layout (hub-spoke)
        .validators(count)           // Validator count
        .executors(count)            // Executor count
        .apply()                     // Return to main builder

// ═══════════════════════════════════════════════════════════════
// WALLET SEEDING
// ═══════════════════════════════════════════════════════════════

    .wallets(user_count)             // Uniform: 100 funds/user
    .with_wallet_config(custom)      // Custom WalletConfig

// ═══════════════════════════════════════════════════════════════
// TRANSACTION WORKLOAD
// ═══════════════════════════════════════════════════════════════

    .transactions()
        .rate(txs_per_block)         // NonZeroU64
        .users(actor_count)          // NonZeroUsize
        .apply()

// ═══════════════════════════════════════════════════════════════
// DA WORKLOAD
// ═══════════════════════════════════════════════════════════════

    .da()
        .channel_rate(ops_per_block) // Channel inscriptions
        .blob_rate(blobs_per_chan)   // Blobs per channel
        .apply()

// ═══════════════════════════════════════════════════════════════
// CHAOS WORKLOAD (requires .enable_node_control())
// ═══════════════════════════════════════════════════════════════

    .enable_node_control()           // Required first!
    
    .chaos_random_restart()
        .validators(bool)            // Restart validators?
        .executors(bool)             // Restart executors?
        .min_delay(Duration)         // Min between restarts
        .max_delay(Duration)         // Max between restarts
        .target_cooldown(Duration)   // Per-node cooldown
        .apply()

// ═══════════════════════════════════════════════════════════════
// DURATION & EXPECTATIONS
// ═══════════════════════════════════════════════════════════════

    .with_run_duration(Duration)     // Clamped to ≥2 blocks
    
    .expect_consensus_liveness()     // Default 80% tolerance
    
    .with_expectation(custom)        // Add custom Expectation
    .with_workload(custom)           // Add custom Workload

// ═══════════════════════════════════════════════════════════════
// BUILD
// ═══════════════════════════════════════════════════════════════

    .build()                         // Returns Scenario<Caps>
```

### Quick Patterns

```rust
// Minimal smoke test
ScenarioBuilder::with_node_counts(2, 0)
    .with_run_duration(Duration::from_secs(30))
    .expect_consensus_liveness()
    .build()

// Transaction throughput
ScenarioBuilder::with_node_counts(2, 0)
    .wallets(64)
    .transactions().rate(10).users(8).apply()
    .with_run_duration(Duration::from_secs(120))
    .expect_consensus_liveness()
    .build()

// DA + transactions
ScenarioBuilder::with_node_counts(1, 1)
    .wallets(64)
    .transactions().rate(5).users(4).apply()
    .da().channel_rate(1).blob_rate(1).apply()
    .with_run_duration(Duration::from_secs(180))
    .expect_consensus_liveness()
    .build()

// Chaos resilience
ScenarioBuilder::with_node_counts(3, 1)
    .enable_node_control()
    .wallets(64)
    .transactions().rate(3).users(4).apply()
    .chaos_random_restart()
        .validators(true).executors(true)
        .min_delay(Duration::from_secs(45))
        .max_delay(Duration::from_secs(75))
        .target_cooldown(Duration::from_secs(120))
        .apply()
    .with_run_duration(Duration::from_secs(300))
    .expect_consensus_liveness()
    .build()
```

---

<!-- FILE: reference/api-reference.md -->

## API Quick Reference

### RunContext

```rust
impl RunContext {
    // ─────────────────────────────────────────────────────────────
    // TOPOLOGY ACCESS
    // ─────────────────────────────────────────────────────────────
    
    /// Static topology configuration
    pub fn descriptors(&self) -> &GeneratedTopology;
    
    /// Live node handles (if available)
    pub fn topology(&self) -> Option<&Topology>;
    
    // ─────────────────────────────────────────────────────────────
    // CLIENT ACCESS
    // ─────────────────────────────────────────────────────────────
    
    /// All node clients
    pub fn node_clients(&self) -> &NodeClients;
    
    /// Random node client
    pub fn random_node_client(&self) -> Option<&ApiClient>;
    
    /// Cluster client with retry logic
    pub fn cluster_client(&self) -> ClusterClient<'_>;
    
    // ─────────────────────────────────────────────────────────────
    // WALLET ACCESS
    // ─────────────────────────────────────────────────────────────
    
    /// Seeded wallet accounts
    pub fn wallet_accounts(&self) -> &[WalletAccount];
    
    // ─────────────────────────────────────────────────────────────
    // OBSERVABILITY
    // ─────────────────────────────────────────────────────────────
    
    /// Block observation stream
    pub fn block_feed(&self) -> BlockFeed;
    
    /// Prometheus metrics (if configured)
    pub fn telemetry(&self) -> &Metrics;
    
    // ─────────────────────────────────────────────────────────────
    // TIMING
    // ─────────────────────────────────────────────────────────────
    
    /// Configured run duration
    pub fn run_duration(&self) -> Duration;
    
    /// Expected block count for this run
    pub fn expected_blocks(&self) -> u64;
    
    /// Full timing metrics
    pub fn run_metrics(&self) -> RunMetrics;
    
    // ─────────────────────────────────────────────────────────────
    // NODE CONTROL (CHAOS)
    // ─────────────────────────────────────────────────────────────
    
    /// Node control handle (if enabled)
    pub fn node_control(&self) -> Option<Arc<dyn NodeControlHandle>>;
}
```

### NodeClients

```rust
impl NodeClients {
    pub fn validator_clients(&self) -> &[ApiClient];
    pub fn executor_clients(&self) -> &[ApiClient];
    pub fn random_validator(&self) -> Option<&ApiClient>;
    pub fn random_executor(&self) -> Option<&ApiClient>;
    pub fn all_clients(&self) -> impl Iterator<Item = &ApiClient>;
    pub fn any_client(&self) -> Option<&ApiClient>;
    pub fn cluster_client(&self) -> ClusterClient<'_>;
}
```

### ApiClient

```rust
impl ApiClient {
    // Consensus
    pub async fn consensus_info(&self) -> reqwest::Result<CryptarchiaInfo>;
    
    // Network
    pub async fn network_info(&self) -> reqwest::Result<Libp2pInfo>;
    
    // Transactions
    pub async fn submit_transaction(&self, tx: &SignedMantleTx) -> reqwest::Result<()>;
    
    // Storage
    pub async fn storage_block(&self, id: &HeaderId) 
        -> reqwest::Result<Option<Block<SignedMantleTx>>>;
    
    // DA
    pub async fn balancer_stats(&self) -> reqwest::Result<BalancerStats>;
    pub async fn monitor_stats(&self) -> reqwest::Result<MonitorStats>;
    pub async fn da_get_membership(&self, session: &SessionNumber)
        -> reqwest::Result<MembershipResponse>;
    
    // URLs
    pub fn base_url(&self) -> &Url;
}
```

### CryptarchiaInfo

```rust
pub struct CryptarchiaInfo {
    pub height: u64,      // Current block height
    pub slot: Slot,       // Current slot number
    pub tip: HeaderId,    // Tip of the chain
    // ... additional fields
}
```

### Key Traits

```rust
#[async_trait]
pub trait Workload: Send + Sync {
    fn name(&self) -> &str;
    fn expectations(&self) -> Vec<Box<dyn Expectation>> { vec![] }
    fn init(&mut self, topology: &GeneratedTopology, metrics: &RunMetrics) 
        -> Result<(), DynError> { Ok(()) }
    async fn start(&self, ctx: &RunContext) -> Result<(), DynError>;
}

#[async_trait]
pub trait Expectation: Send + Sync {
    fn name(&self) -> &str;
    fn init(&mut self, topology: &GeneratedTopology, metrics: &RunMetrics)
        -> Result<(), DynError> { Ok(()) }
    async fn start_capture(&mut self, ctx: &RunContext) -> Result<(), DynError> { Ok(()) }
    async fn evaluate(&mut self, ctx: &RunContext) -> Result<(), DynError>;
}

#[async_trait]
pub trait Deployer<Caps = ()>: Send + Sync {
    type Error;
    async fn deploy(&self, scenario: &Scenario<Caps>) -> Result<Runner, Self::Error>;
}

#[async_trait]
pub trait NodeControlHandle: Send + Sync {
    async fn restart_validator(&self, index: usize) -> Result<(), DynError>;
    async fn restart_executor(&self, index: usize) -> Result<(), DynError>;
}
```

---

<!-- FILE: reference/glossary.md -->

## Glossary

### Protocol Terms

| Term | Definition |
|------|------------|
| **Slot** | Fixed time interval in the consensus protocol (default: 2 seconds) |
| **Block** | Unit of consensus; contains transactions and header |
| **Active Slot Coefficient** | Probability of block production per slot (default: 0.5) |
| **Protocol Interval** | Expected time between blocks: `slot_duration / active_slot_coeff` |

### Framework Terms

| Term | Definition |
|------|------------|
| **Topology** | Declarative description of cluster shape, roles, and parameters |
| **GeneratedTopology** | Concrete topology with generated configs, ports, and keys |
| **Scenario** | Plan combining topology + workloads + expectations + duration |
| **Workload** | Traffic/behavior generator during a run |
| **Expectation** | Post-run assertion judging success/failure |
| **BlockFeed** | Stream of block observations for workloads/expectations |
| **RunContext** | Shared context with clients, metrics, observability |
| **RunMetrics** | Computed timing: expected blocks, block interval, duration |
| **NodeClients** | Collection of API clients for validators and executors |
| **ApiClient** | HTTP client for node consensus, network, and DA endpoints |
| **cfgsync** | Dynamic configuration injection for distributed runners |

### Runner Terms

| Term | Definition |
|------|------------|
| **Deployer** | Creates a `Runner` from a `Scenario` |
| **Runner** | Manages execution: workloads, expectations, cleanup |
| **RunHandle** | Returned on success; holds context and cleanup |
| **CleanupGuard** | Ensures resources are reclaimed on drop |
| **NodeControlHandle** | Interface for restarting nodes (chaos) |

---

<!-- FILE: recipes/index.md -->

# Part V — Scenario Recipes

Complete, copy-paste runnable scenarios.

## Recipe 1: Minimal Smoke Test

**Goal**: Verify basic consensus works with minimal setup.

```rust
use std::time::Duration;
use testing_framework_core::scenario::{Deployer as _, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;

#[tokio::test]
async fn smoke_test_consensus() {
    // Minimal: 2 validators, no workloads, just check blocks produced
    let mut plan = ScenarioBuilder::with_node_counts(2, 0)
        .topology()
            .network_star()
            .validators(2)
            .executors(0)
            .apply()
        .with_run_duration(Duration::from_secs(30))
        .expect_consensus_liveness()
        .build();

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await.expect("deployment");
    runner.run(&mut plan).await.expect("scenario passed");
}
```

**Expected output**:
```
[INFO] consensus_liveness: target=4, observed heights=[6, 5] ✓
```

**Common failures**:
- `height 0 below target`: Nodes didn't start, check binaries exist
- Timeout: Increase to 60s or set `SLOW_TEST_ENV=true`

---

## Recipe 2: Transaction Throughput Baseline

**Goal**: Measure transaction inclusion under load.

```rust
use std::time::Duration;
use testing_framework_core::scenario::{Deployer as _, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use tests_workflows::ScenarioBuilderExt as _;

const VALIDATORS: usize = 2;
const TX_RATE: u64 = 10;
const USERS: usize = 8;
const WALLETS: usize = 64;
const DURATION: Duration = Duration::from_secs(120);

#[tokio::test]
async fn transaction_throughput_baseline() {
    let mut plan = ScenarioBuilder::with_node_counts(VALIDATORS, 0)
        .topology()
            .network_star()
            .validators(VALIDATORS)
            .executors(0)
            .apply()
        .wallets(WALLETS)
        .transactions()
            .rate(TX_RATE)
            .users(USERS)
            .apply()
        .with_run_duration(DURATION)
        .expect_consensus_liveness()
        .build();

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await.expect("deployment");
    
    let handle = runner.run(&mut plan).await.expect("scenario passed");
    
    // Optional: Check stats
    let stats = handle.context().block_feed().stats();
    println!("Total transactions included: {}", stats.total_transactions());
}
```

**Expected output**:
```
[INFO] tx_inclusion_expectation: 180/200 included (90%) ✓
[INFO] consensus_liveness: target=15, observed heights=[18, 17] ✓
Total transactions included: 180
```

**Common failures**:
- `observed 0 below required`: Forgot `.wallets()`
- Low inclusion: Reduce `TX_RATE` or increase `DURATION`

---

## Recipe 3: DA + Transaction Combined Stress

**Goal**: Exercise both transaction and data-availability paths.

```rust
use std::time::Duration;
use testing_framework_core::scenario::{Deployer as _, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use tests_workflows::ScenarioBuilderExt as _;

#[tokio::test]
async fn da_tx_combined_stress() {
    let mut plan = ScenarioBuilder::with_node_counts(1, 1)  // Need executor for DA
        .topology()
            .network_star()
            .validators(1)
            .executors(1)
            .apply()
        .wallets(64)
        .transactions()
            .rate(5)
            .users(4)
            .apply()
        .da()
            .channel_rate(2)   // 2 channel inscriptions per block
            .blob_rate(1)      // 1 blob per channel
            .apply()
        .with_run_duration(Duration::from_secs(180))
        .expect_consensus_liveness()
        .build();

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await.expect("deployment");
    runner.run(&mut plan).await.expect("scenario passed");
}
```

**Expected output**:
```
[INFO] da_workload_inclusions: 2/2 channels inscribed ✓
[INFO] tx_inclusion_expectation: 45/50 included (90%) ✓
[INFO] consensus_liveness: target=22, observed heights=[25, 24] ✓
```

**Common failures**:
- `da workload requires at least one executor`: Add executor to topology
- Blob publish failures: Check DA balancer readiness

---

## Recipe 4: Chaos Resilience Test

**Goal**: Verify system recovers from node restarts.

```rust
use std::time::Duration;
use testing_framework_core::scenario::{Deployer as _, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use tests_workflows::{ChaosBuilderExt as _, ScenarioBuilderExt as _};

#[tokio::test]
async fn chaos_resilience_test() {
    let mut plan = ScenarioBuilder::with_node_counts(3, 1)  // Need >1 validator for chaos
        .enable_node_control()  // Required for chaos!
        .topology()
            .network_star()
            .validators(3)
            .executors(1)
            .apply()
        .wallets(64)
        .transactions()
            .rate(3)  // Lower rate for stability during chaos
            .users(4)
            .apply()
        .chaos_random_restart()
            .validators(true)
            .executors(true)
            .min_delay(Duration::from_secs(45))
            .max_delay(Duration::from_secs(75))
            .target_cooldown(Duration::from_secs(120))
            .apply()
        .with_run_duration(Duration::from_secs(300))  // 5 minutes
        .expect_consensus_liveness()
        .build();

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&plan).await.expect("deployment");
    runner.run(&mut plan).await.expect("chaos scenario passed");
}
```

**Expected output**:
```
[INFO] Restarting validator-1
[INFO] Restarting executor-0
[INFO] Restarting validator-2
[INFO] consensus_liveness: target=35, observed heights=[42, 38, 40, 39] ✓
```

**Common failures**:
- `no eligible targets`: Need ≥2 validators (safety skips single validator)
- Liveness violation: Increase `target_cooldown`, reduce restart frequency

---

## Recipe 5: Docker Compose Reproducible Test

**Goal**: Run in containers for CI reproducibility.

```rust
use std::time::Duration;
use testing_framework_core::scenario::{Deployer as _, ScenarioBuilder};
use testing_framework_runner_compose::ComposeRunner;
use tests_workflows::ScenarioBuilderExt as _;

#[tokio::test]
#[ignore = "requires Docker"]
async fn compose_reproducible_test() {
    let mut plan = ScenarioBuilder::with_node_counts(2, 1)
        .topology()
            .network_star()
            .validators(2)
            .executors(1)
            .apply()
        .wallets(64)
        .transactions()
            .rate(5)
            .users(8)
            .apply()
        .da()
            .channel_rate(1)
            .blob_rate(1)
            .apply()
        .with_run_duration(Duration::from_secs(120))
        .expect_consensus_liveness()
        .build();

    let deployer = ComposeRunner::default();
    let runner = deployer.deploy(&plan).await.expect("compose deployment");
    
    // Verify Prometheus is available
    assert!(runner.context().telemetry().is_configured());
    
    runner.run(&mut plan).await.expect("compose scenario passed");
}
```

**Required environment**:
```bash
# Build the Docker image first
docker build -t nomos-testnet:local .

# Or use custom image
export NOMOS_TESTNET_IMAGE=myregistry/nomos-testnet:v1.0
```

**Common failures**:
- `cfgsync connection refused`: Check port 4400 is accessible
- Image not found: Build or pull `nomos-testnet:local`

---

<!-- FILE: reference/faq.md -->

## FAQ

**Q: Why does chaos skip validators when only one is configured?**

A: Restarting the only validator would halt consensus entirely. The framework protects against this by requiring ≥2 validators for chaos to restart validators. See `RandomRestartWorkload::targets()`.

**Q: Can I run the same scenario on different runners?**

A: Yes! The `Scenario` is runner-agnostic. Just swap the deployer:

```rust
let plan = build_my_scenario();  // Same plan

// Local
let runner = LocalDeployer::default().deploy(&plan).await?;

// Or Compose
let runner = ComposeRunner::default().deploy(&plan).await?;

// Or K8s
let runner = K8sRunner::new().deploy(&plan).await?;
```

**Q: How do I debug a flaky scenario?**

A: 
1. Enable tracing: `NOMOS_TESTS_TRACING=true`
2. Keep logs: `NOMOS_TESTS_KEEP_LOGS=1`
3. Increase duration
4. Simplify (remove workloads one by one)

**Q: Why are expectations evaluated after all workloads, not during?**

A: This ensures the system has reached steady state. If you need continuous assertions, implement them inside your workload using `BlockFeed`.

**Q: How long should my scenario run?**

A: See the [Duration Heuristics](#duration-heuristics) table. Rule of thumb: enough blocks to observe your workload's effects plus margin for variability.

**Q: What's the difference between `Plan` and `Scenario`?**

A: In the code, `ScenarioBuilder` builds a `Scenario`. The term "plan" is informal shorthand for "fully constructed scenario ready for deployment."

---

## Changelog

### v3 (Current)

**New sections:**
- 5-Minute Quickstart
- Reading Guide by Role
- Duration Heuristics table
- BlockFeed Deep Dive
- Configuration Sync (cfgsync) documentation
- Environment Variables reference
- Complete Scenario Recipes (5 recipes)
- Common Expectation Mistakes table
- Debugging Flow diagram
- GitBook structure markers

**Fixes from v2:**
- All API method names verified against codebase
- Error messages taken from actual error types
- Environment variables verified in source

**Improvements:**
- More diagrams (timeline, readiness phases, type flow)
- Troubleshooting with actual error messages
- FAQ expanded with common questions

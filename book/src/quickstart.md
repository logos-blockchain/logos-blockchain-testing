# Quickstart

Get a working example running quickly.

## Prerequisites

- Rust toolchain (nightly)
- This repository cloned
- Unix-like system (tested on Linux and macOS)
- For Docker Compose examples: Docker daemon running

**Note:** `nomos-node` binaries are built automatically on demand or can be provided via prebuilt bundles.

## Your First Test

The framework ships with runnable example binaries in `examples/src/bin/`.

**Recommended:** Use the convenience script:

```bash
# From the nomos-testing directory
scripts/run-examples.sh -t 60 -v 1 -e 1 host
```

This handles circuit setup, binary building, and runs a complete scenario: 1 validator + 1 executor, mixed transaction + DA workload (5 tx/block + 1 channel + 1 blob), 60s duration.

**Alternative:** Direct cargo run (requires manual setup):

```bash
# Requires circuits in place and NOMOS_NODE_BIN/NOMOS_EXECUTOR_BIN set
POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin local_runner
```

**Core API Pattern** (simplified example):

```rust
use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ScenarioBuilderExt;
use std::time::Duration;

// Define the scenario (1 validator + 1 executor, tx + DA workload)
let mut plan = ScenarioBuilder::topology_with(|t| {
        t.network_star()
            .validators(1)
            .executors(1)
    })
    .wallets(64)
    .transactions_with(|txs| {
        txs.rate(5)                 // 5 transactions per block
            .users(8)
    })
    .da_with(|da| {
        da.channel_rate(1)         // 1 channel operation per block
            .blob_rate(1)          // 1 blob dispersal per block
    })
    .expect_consensus_liveness()
    .with_run_duration(Duration::from_secs(60))
    .build();

// Deploy and run
let deployer = LocalDeployer::default();
let runner = deployer.deploy(&plan).await?;
let _handle = runner.run(&mut plan).await?;
```

**Note:** The examples are binaries with `#[tokio::main]`, not test functions. If you want to write integration tests, wrap this pattern in `#[tokio::test]` functions in your own test suite.

**Important:** `POL_PROOF_DEV_MODE=true` disables expensive Groth16 zero-knowledge proof generation for leader election. Without it, proof generation is CPU-intensive and tests will timeout. **This is required for all runners** (local, compose, k8s) for practical testing. Never use in production.

**What you should see:**
- Nodes spawn as local processes
- Consensus starts producing blocks
- Scenario runs for the configured duration
- Node logs written to temporary directories in working directory (auto-cleaned up after test)
- To persist logs: set `NOMOS_TESTS_TRACING=true` and `NOMOS_LOG_DIR=/path/to/logs` (files will have prefix like `nomos-node-0*`, may include timestamps)

## What Just Happened?

Let's unpack the code:

### 1. Topology Configuration

```rust
ScenarioBuilder::topology_with(|t| {
        t.network_star()      // Star topology: all nodes connect to seed
            .validators(1)    // 1 validator node
            .executors(1)     // 1 executor node (validator + DA dispersal)
    })
```

This defines **what** your test network looks like.

### 2. Wallet Seeding

```rust
.wallets(64)                 // Seed 64 funded wallet accounts
```

Provides funded accounts for transaction submission.

### 3. Workloads

```rust
.transactions_with(|txs| {
    txs.rate(5)              // 5 transactions per block
        .users(8)            // Use 8 of the 64 wallets
})
.da_with(|da| {
    da.channel_rate(1)       // 1 channel operation per block
        .blob_rate(1)        // 1 blob dispersal per block
})
```

Generates both transaction and DA traffic to stress both subsystems.

### 4. Expectation

```rust
.expect_consensus_liveness()
```

This says **what success means**: blocks must be produced continuously.

### 5. Run Duration

```rust
.with_run_duration(Duration::from_secs(60))
```

Run for 60 seconds (~27 blocks with default 2s slots, 0.9 coefficient). Framework ensures this is at least 2× the consensus slot duration.

### 6. Deploy and Execute

```rust
let deployer = LocalDeployer::default();  // Use local process deployer
let runner = deployer.deploy(&plan).await?;  // Provision infrastructure
let _handle = runner.run(&mut plan).await?;  // Execute workloads & expectations
```

**Deployer** provisions the infrastructure. **Runner** orchestrates execution.

## Adjust the Topology

**With run-examples.sh** (recommended):

```bash
# Scale up to 3 validators + 2 executors, run for 2 minutes
scripts/run-examples.sh -t 120 -v 3 -e 2 host
```

**With direct cargo run:**

```bash
# Uses NOMOS_DEMO_* env vars (or legacy *_DEMO_* vars)
NOMOS_DEMO_VALIDATORS=3 \
NOMOS_DEMO_EXECUTORS=2 \
NOMOS_DEMO_RUN_SECS=120 \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin local_runner
```

## Try Docker Compose

Use the same API with a different deployer for reproducible containerized environment.

**Recommended:** Use the convenience script (handles everything):

```bash
scripts/run-examples.sh -t 60 -v 1 -e 1 compose
```

This automatically:
- Fetches circuit assets (to `testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params`)
- Builds/uses prebuilt binaries (via `NOMOS_BINARIES_TAR` if available)
- Builds the Docker image
- Runs the compose scenario

**Alternative:** Direct cargo run with manual setup:

```bash
# Option 1: Use prebuilt bundle (recommended for compose/k8s)
scripts/build-bundle.sh --platform linux  # Creates .tmp/nomos-binaries-linux-v0.3.1.tar.gz
export NOMOS_BINARIES_TAR=.tmp/nomos-binaries-linux-v0.3.1.tar.gz

# Option 2: Manual circuit/image setup (rebuilds during image build)
scripts/setup-nomos-circuits.sh v0.3.1 /tmp/nomos-circuits
cp -r /tmp/nomos-circuits/* testing-framework/assets/stack/kzgrs_test_params/
testing-framework/assets/stack/scripts/build_test_image.sh

# Run with Compose
NOMOS_TESTNET_IMAGE=nomos-testnet:local \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin compose_runner
```

**Benefit:** Reproducible containerized environment with Prometheus at `http://localhost:9090`.

**Note:** Compose expects KZG parameters at `/kzgrs_test_params/kzgrs_test_params` inside containers (the directory name is repeated as the filename).

**In code:** Just swap the deployer:

```rust
use testing_framework_runner_compose::ComposeDeployer;

// ... same scenario definition ...

let deployer = ComposeDeployer::default();  // Use Docker Compose
let runner = deployer.deploy(&plan).await?;
let _handle = runner.run(&mut plan).await?;
```

## Next Steps

Now that you have a working test:

- **Understand the philosophy**: [Testing Philosophy](testing-philosophy.md)
- **Learn the architecture**: [Architecture Overview](architecture-overview.md)
- **See more examples**: [Examples](examples.md)
- **API reference**: [Builder API Quick Reference](dsl-cheat-sheet.md)
- **Debug failures**: [Troubleshooting](troubleshooting.md)

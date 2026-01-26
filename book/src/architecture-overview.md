# Architecture Overview

The framework follows a clear flow: **Topology → Scenario → Deployer → Runner → Workloads → Expectations**.

## Core Flow

```mermaid
flowchart LR
    A(Topology<br/>shape cluster) --> B(Scenario<br/>plan)
    B --> C(Deployer<br/>provision & readiness)
    C --> D(Runner<br/>orchestrate execution)
    D --> E(Workloads<br/>drive traffic)
    E --> F(Expectations<br/>verify outcomes)
```

## Crate Architecture

```mermaid
flowchart TB
    subgraph Examples["Runner Examples"]
        LocalBin[local_runner.rs]
        ComposeBin[compose_runner.rs]
        K8sBin[k8s_runner.rs]
    end
    
    subgraph Workflows["Workflows (Batteries Included)"]
        DSL[ScenarioBuilderExt<br/>Fluent API]
        TxWorkload[Transaction Workload]
        ChaosWorkload[Chaos Workload]
        Expectations[Built-in Expectations]
    end
    
    subgraph Core["Core Framework"]
        ScenarioModel[Scenario Model]
        Traits[Deployer + Runner Traits]
        BlockFeed[BlockFeed]
        NodeClients[Node Clients]
        Topology[Topology Generation]
    end
    
    subgraph Deployers["Runner Implementations"]
        LocalDeployer[LocalDeployer]
        ComposeDeployer[ComposeDeployer]
        K8sDeployer[K8sDeployer]
    end
    
    subgraph Support["Supporting Crates"]
        Configs[Configs & Topology]
        Nodes[Node API Clients]
    end
    
    Examples --> Workflows
    Examples --> Deployers
    Workflows --> Core
    Deployers --> Core
    Deployers --> Support
    Core --> Support
    Workflows --> Support
    
    style Examples fill:#e1f5ff
    style Workflows fill:#e1ffe1
    style Core fill:#fff4e1
    style Deployers fill:#ffe1f5
    style Support fill:#f0f0f0
```

### Layer Responsibilities

**Runner Examples (Entry Points)**
- Executable binaries that demonstrate framework usage
- Wire together deployers, scenarios, and execution
- Provide CLI interfaces for different modes

**Workflows (High-Level API)**
- `ScenarioBuilderExt` trait provides fluent DSL
- Built-in workloads (transactions, chaos)
- Common expectations (liveness, inclusion)
- Simplifies scenario authoring

**Core Framework (Foundation)**
- `Scenario` model and lifecycle orchestration
- `Deployer` and `Runner` traits (extension points)
- `BlockFeed` for real-time block observation
- `RunContext` providing node clients and metrics
- Topology generation and validation

**Runner Implementations**
- `LocalDeployer` - spawns processes on host
- `ComposeDeployer` - orchestrates Docker Compose
- `K8sDeployer` - deploys to Kubernetes cluster
- Each implements `Deployer` trait

**Supporting Crates**
- `configs` - Topology configuration and generation
- `nodes` - HTTP/RPC client for node APIs

### Extension Points

```mermaid
flowchart LR
    Custom[Your Code] -.implements.-> Workload[Workload Trait]
    Custom -.implements.-> Expectation[Expectation Trait]
    Custom -.implements.-> Deployer[Deployer Trait]
    
    Workload --> Core[Core Framework]
    Expectation --> Core
    Deployer --> Core
    
    style Custom fill:#ffe1f5
    style Core fill:#fff4e1
```

**Extend by implementing:**
- `Workload` - Custom traffic generation patterns
- `Expectation` - Custom success criteria
- `Deployer` - Support for new deployment targets

See [Extending the Framework](extending.md) for details.

### Components

- **Topology** describes the cluster: how many nodes and the high-level network parameters they should follow.
- **Scenario** combines that topology with the activities to run and the checks to perform, forming a single plan.
- **Deployer** provisions infrastructure on the chosen backend (local processes, Docker Compose, or Kubernetes), waits for readiness, and returns a Runner.
- **Runner** orchestrates scenario execution: starts workloads, observes signals, evaluates expectations, and triggers cleanup.
- **Workloads** generate traffic and conditions that exercise the system.
- **Expectations** observe the run and judge success or failure once activity completes.

Each layer has a narrow responsibility so that cluster shape, deployment choice,
traffic generation, and health checks can evolve independently while fitting
together predictably.

## Entry Points

The framework is consumed via **runnable example binaries** in `examples/src/bin/`:

- `local_runner.rs` — Spawns nodes as host processes
- `compose_runner.rs` — Deploys via Docker Compose (requires `LOGOS_BLOCKCHAIN_TESTNET_IMAGE` built)
- `k8s_runner.rs` — Deploys via Kubernetes Helm (requires cluster + image)

**Recommended:** Use the convenience script:

```bash
scripts/run/run-examples.sh -t <duration> -n <nodes> <mode>
# mode: host, compose, or k8s
```

This handles circuit setup, binary building/bundling, image building, and execution.

**Alternative:** Direct cargo run (requires manual setup):

```bash
POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin <name>
```

**Important:** All runners require `POL_PROOF_DEV_MODE=true` to avoid expensive Groth16 proof generation that causes timeouts.

These binaries use the framework API (`ScenarioBuilder`) to construct and execute scenarios.

## Builder API

Scenarios are defined using a fluent builder pattern:

```rust,ignore
use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

pub fn scenario_plan() -> testing_framework_core::scenario::Scenario<()> {
    ScenarioBuilder::topology_with(|t| t.network_star().nodes(3))
        .wallets(50)
        .transactions_with(|txs| txs.rate(5).users(20))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(90))
        .build()
}
```

**Key API Points:**
- Topology uses `.topology_with(|t| { t.nodes(N) })` closure pattern
- Workloads are configured via `_with` closures (`transactions_with`, `chaos_with`)
- Chaos workloads require `.enable_node_control()` and a compatible runner

## Deployers

Three deployer implementations:

| Deployer | Backend | Prerequisites | Node Control |
|----------|---------|---------------|--------------|
| `LocalDeployer` | Host processes | Binaries (built on demand or via bundle) | No |
| `ComposeDeployer` | Docker Compose | Image with embedded assets/binaries | Yes |
| `K8sDeployer` | Kubernetes Helm | Cluster + image loaded | Not yet |

**Compose-specific features:**
- Observability is external (set `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL` / `LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL` / `LOGOS_BLOCKCHAIN_GRAFANA_URL` as needed)
- Optional OTLP trace/metrics endpoints (`LOGOS_BLOCKCHAIN_OTLP_ENDPOINT`, `LOGOS_BLOCKCHAIN_OTLP_METRICS_ENDPOINT`)
- Node control for chaos testing (restart nodes)

## Assets and Images

### Docker Image
Built via `scripts/build/build_test_image.sh`:
- Embeds circuit assets and binaries
- Includes runner scripts: `run_nomos_node.sh`
- Tagged as `LOGOS_BLOCKCHAIN_TESTNET_IMAGE` (default: `logos-blockchain-testing:local`)
- **Recommended:** Use prebuilt bundle via `scripts/build/build-bundle.sh --platform linux` and set `LOGOS_BLOCKCHAIN_BINARIES_TAR` before building image

### Circuit Assets
Circuit assets required by the node binary:
- **Host path:** `~/.logos-blockchain-circuits` (default)
- **Container path:** `/opt/circuits` (for compose/k8s)
- **Override:** `LOGOS_BLOCKCHAIN_CIRCUITS=/custom/path/to/dir` (must point to a directory)
- **Fetch via:** `scripts/setup/setup-logos-blockchain-circuits.sh v0.3.1 ~/.logos-blockchain-circuits` or use `scripts/run/run-examples.sh`

### Compose Stack
Templates and configs in `testing-framework/runners/compose/assets/`:
- `docker-compose.yml.tera` — Stack template (nodes)
- Cfgsync config: `testing-framework/assets/stack/cfgsync.yaml`
- Monitoring assets (not deployed by the framework): `testing-framework/assets/stack/monitoring/`

## Logging Architecture

**Two separate logging pipelines:**

| Component | Configuration | Output |
|-----------|--------------|--------|
| **Runner binaries** | `RUST_LOG` | Framework orchestration logs |
| **Node processes** | `LOGOS_BLOCKCHAIN_LOG_LEVEL`, `LOGOS_BLOCKCHAIN_LOG_FILTER` (+ `LOGOS_BLOCKCHAIN_LOG_DIR` on host runner) | Consensus, mempool, network logs |

**Node logging:**
- **Local runner:** Writes to temporary directories by default (cleaned up). Set `LOGOS_BLOCKCHAIN_TESTS_TRACING=true` + `LOGOS_BLOCKCHAIN_LOG_DIR` for persistent files.
- **Compose runner:** Default logs to container stdout/stderr (`docker logs`). To write per-node files, set `tracing_settings.logger: !File` in `testing-framework/assets/stack/cfgsync.yaml` (and mount a writable directory).
- **K8s runner:** Logs to pod stdout/stderr (`kubectl logs`). To write per-node files, set `tracing_settings.logger: !File` in `testing-framework/assets/stack/cfgsync.yaml` (and mount a writable directory).

**File naming:** Per-node files use prefix `logos-blockchain-node-{index}` (may include timestamps).

## Observability

**Prometheus-compatible metrics querying (optional):**
- The framework does **not** deploy Prometheus/Grafana.
- Provide a Prometheus-compatible base URL (PromQL API) via `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL`.
- Accessible in expectations when configured: `ctx.telemetry().prometheus().map(|p| p.base_url())`

**Grafana dashboards (optional):**
- Dashboards live in `testing-framework/assets/stack/monitoring/grafana/dashboards/` and can be imported into your Grafana.
- If you set `LOGOS_BLOCKCHAIN_GRAFANA_URL`, the deployer prints it in `TESTNET_ENDPOINTS`.

**Node APIs:**
- HTTP endpoints per node for consensus info and network status
- Accessible in expectations: `ctx.node_clients().node_clients().get(0)`

**OTLP (optional):**
- Trace endpoint: `LOGOS_BLOCKCHAIN_OTLP_ENDPOINT=http://localhost:4317`
- Metrics endpoint: `LOGOS_BLOCKCHAIN_OTLP_METRICS_ENDPOINT=http://localhost:4318`
- Disabled by default (no noise if unset)

For detailed logging configuration, see [Logging & Observability](logging-observability.md).

# Annotated Tree

Directory structure with key paths annotated:

```text
logos-blockchain-testing/
├─ testing-framework/           # Core library crates
│  ├─ configs/                  # Node config builders, topology generation, tracing/logging config
│  ├─ core/                     # Scenario model (ScenarioBuilder), runtime (Runner, Deployer), topology, node spawning
│  ├─ workflows/                # Workloads (transactions, chaos), expectations (liveness), builder DSL extensions
│  ├─ deployers/                # Deployment backends
│  │  ├─ local/                 # LocalDeployer (spawns local processes)
│  │  ├─ compose/               # ComposeDeployer (Docker Compose + Prometheus)
│  │  └─ k8s/                   # K8sDeployer (Kubernetes Helm)
│  └─ assets/                   # Docker/K8s stack assets
│     └─ stack/
│        ├─ monitoring/         # Prometheus config
│        ├─ scripts/            # Container entrypoints
│        └─ cfgsync.yaml        # Config sync server template
│
├─ examples/                    # PRIMARY ENTRY POINT: runnable binaries
│  └─ src/bin/
│     ├─ local_runner.rs        # Host processes demo (LocalDeployer)
│     ├─ compose_runner.rs      # Docker Compose demo (ComposeDeployer)
│     └─ k8s_runner.rs          # Kubernetes demo (K8sDeployer)
│
├─ scripts/                     # Helper utilities
│  ├─ run-examples.sh           # Convenience script (handles setup + runs examples)
│  ├─ build-bundle.sh           # Build prebuilt binaries+circuits bundle
│  └─ setup-logos-blockchain-circuits.sh  # Fetch circuit assets (Linux + host)
│
└─ book/                        # This documentation (mdBook)
```

## Key Directories Explained

### `testing-framework/`
Core library crates providing the testing API.

| Crate | Purpose | Key Exports |
|-------|---------|-------------|
| `configs` | Node configuration builders | Topology generation, tracing config |
| `core` | Scenario model & runtime | `ScenarioBuilder`, `Deployer`, `Runner` |
| `workflows` | Workloads & expectations | `ScenarioBuilderExt`, `ChaosBuilderExt` |
| `deployers/local` | Local process deployer | `LocalDeployer` |
| `deployers/compose` | Docker Compose deployer | `ComposeDeployer` |
| `deployers/k8s` | Kubernetes deployer | `K8sDeployer` |

### `testing-framework/assets/stack/`
Docker/K8s deployment assets:
- **`monitoring/`**: Prometheus config
- **`scripts/`**: Container entrypoints

### `scripts/`
Convenience utilities:
- **`run-examples.sh`**: All-in-one script for host/compose/k8s modes (recommended)
- **`build-bundle.sh`**: Create prebuilt binaries+circuits bundle for compose/k8s
- **`build_test_image.sh`**: Build the compose/k8s Docker image (bakes in assets)
- **`setup-logos-blockchain-circuits.sh`**: Fetch circuit assets for both Linux and host
- **`cfgsync.yaml`**: Configuration sync server template

### `examples/` (Start Here!)
**Runnable binaries** demonstrating framework usage:
- `local_runner.rs` — Local processes
- `compose_runner.rs` — Docker Compose (requires `LOGOS_BLOCKCHAIN_TESTNET_IMAGE` built)
- `k8s_runner.rs` — Kubernetes (requires cluster + image)

**Run with:** `POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin <name>`

**All runners require `POL_PROOF_DEV_MODE=true`** to avoid expensive proof generation.

### `scripts/`
Helper utilities:
- **`setup-logos-blockchain-circuits.sh`**: Fetch circuit assets from releases

## Observability

**Compose runner** includes:
- **Prometheus** at `http://localhost:9090` (metrics scraping)
- Node metrics exposed per node
- Access in expectations: `ctx.telemetry().prometheus().map(|p| p.base_url())`

**Logging** controlled by:
- `LOGOS_BLOCKCHAIN_LOG_DIR` — Write per-node log files
- `LOGOS_BLOCKCHAIN_LOG_LEVEL` — Global log level (error/warn/info/debug/trace)
- `LOGOS_BLOCKCHAIN_LOG_FILTER` — Target-specific filtering (e.g., `cryptarchia=trace`)
- `LOGOS_BLOCKCHAIN_TESTS_TRACING` — Enable file logging for local runner

See [Logging & Observability](logging-observability.md) for details.

## Navigation Guide

| To Do This | Go Here |
|------------|---------|
| **Run an example** | `examples/src/bin/` → `cargo run -p runner-examples --bin <name>` |
| **Write a custom scenario** | `testing-framework/core/` → Implement using `ScenarioBuilder` |
| **Add a new workload** | `testing-framework/workflows/src/workloads/` → Implement `Workload` trait |
| **Add a new expectation** | `testing-framework/workflows/src/expectations/` → Implement `Expectation` trait |
| **Modify node configs** | `testing-framework/configs/src/topology/configs/` |
| **Extend builder DSL** | `testing-framework/workflows/src/builder/` → Add trait methods |
| **Add a new deployer** | `testing-framework/deployers/` → Implement `Deployer` trait |

For detailed guidance, see [Internal Crate Reference](internal-crate-reference.md).

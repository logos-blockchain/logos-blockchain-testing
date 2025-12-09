# Annotated Tree

Directory structure with key paths annotated:

```
nomos-testing/
├─ testing-framework/           # Core library crates
│  ├─ configs/                  # Node config builders, topology generation, tracing/logging config
│  ├─ core/                     # Scenario model (ScenarioBuilder), runtime (Runner, Deployer), topology, node spawning
│  ├─ workflows/                # Workloads (transactions, DA, chaos), expectations (liveness), builder DSL extensions
│  ├─ runners/                  # Deployment backends
│  │  ├─ local/                 # LocalDeployer (spawns local processes)
│  │  ├─ compose/               # ComposeDeployer (Docker Compose + Prometheus)
│  │  └─ k8s/                   # K8sDeployer (Kubernetes Helm)
│  └─ assets/                   # Docker/K8s stack assets
│     └─ stack/
│        ├─ kzgrs_test_params/  # KZG circuit parameters directory
│        │  └─ kzgrs_test_params  # Actual proving key file (note repeated name)
│        ├─ monitoring/         # Prometheus config
│        ├─ scripts/            # Container entrypoints, image builder
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
│  ├─ setup-circuits-stack.sh  # Fetch KZG parameters (Linux + host)
│  └─ setup-nomos-circuits.sh  # Legacy circuit fetcher
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
| `runners/local` | Local process deployer | `LocalDeployer` |
| `runners/compose` | Docker Compose deployer | `ComposeDeployer` |
| `runners/k8s` | Kubernetes deployer | `K8sDeployer` |

### `testing-framework/assets/stack/`
Docker/K8s deployment assets:
- **`kzgrs_test_params/kzgrs_test_params`**: Circuit parameters file (note repeated name; override via `NOMOS_KZGRS_PARAMS_PATH`)
- **`monitoring/`**: Prometheus config
- **`scripts/`**: Container entrypoints and image builder

### `scripts/`
Convenience utilities:
- **`run-examples.sh`**: All-in-one script for host/compose/k8s modes (recommended)
- **`build-bundle.sh`**: Create prebuilt binaries+circuits bundle for compose/k8s
- **`setup-circuits-stack.sh`**: Fetch KZG parameters for both Linux and host
- **`cfgsync.yaml`**: Configuration sync server template

### `examples/` (Start Here!)
**Runnable binaries** demonstrating framework usage:
- `local_runner.rs` — Local processes
- `compose_runner.rs` — Docker Compose (requires `NOMOS_TESTNET_IMAGE` built)
- `k8s_runner.rs` — Kubernetes (requires cluster + image)

**Run with:** `POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin <name>`

**All runners require `POL_PROOF_DEV_MODE=true`** to avoid expensive proof generation.

### `scripts/`
Helper utilities:
- **`setup-nomos-circuits.sh`**: Fetch KZG parameters from releases

## Observability

**Compose runner** includes:
- **Prometheus** at `http://localhost:9090` (metrics scraping)
- Node metrics exposed per validator/executor
- Access in expectations: `ctx.telemetry().prometheus_endpoint()`

**Logging** controlled by:
- `NOMOS_LOG_DIR` — Write per-node log files
- `NOMOS_LOG_LEVEL` — Global log level (error/warn/info/debug/trace)
- `NOMOS_LOG_FILTER` — Target-specific filtering (e.g., `consensus=trace,da=debug`)
- `NOMOS_TESTS_TRACING` — Enable file logging for local runner

See [Logging and Observability](operations.md#logging-and-observability) for details.

## Navigation Guide

| To Do This | Go Here |
|------------|---------|
| **Run an example** | `examples/src/bin/` → `cargo run -p runner-examples --bin <name>` |
| **Write a custom scenario** | `testing-framework/core/` → Implement using `ScenarioBuilder` |
| **Add a new workload** | `testing-framework/workflows/src/workloads/` → Implement `Workload` trait |
| **Add a new expectation** | `testing-framework/workflows/src/expectations/` → Implement `Expectation` trait |
| **Modify node configs** | `testing-framework/configs/src/topology/configs/` |
| **Extend builder DSL** | `testing-framework/workflows/src/builder/` → Add trait methods |
| **Add a new deployer** | `testing-framework/runners/` → Implement `Deployer` trait |

For detailed guidance, see [Internal Crate Reference](internal-crate-reference.md).

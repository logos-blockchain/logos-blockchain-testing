# Running Examples

The framework provides three runner modes: **host** (local processes), **compose** (Docker Compose), and **k8s** (Kubernetes).

## Quick Start (Recommended)

Use `scripts/run/run-examples.sh` for all modes—it handles all setup automatically:

```bash
# Host mode (local processes)
scripts/run/run-examples.sh -t 60 -n 3 host

# Compose mode (Docker Compose)
scripts/run/run-examples.sh -t 60 -n 3 compose

# K8s mode (Kubernetes)
scripts/run/run-examples.sh -t 60 -n 3 k8s
```

**Parameters:**
- `-t 60` — Run duration in seconds
- `-n 3` — Number of nodes
- `host|compose|k8s` — Deployment mode

This script handles:
- Circuit asset setup
- Binary building/bundling
- Image building (compose/k8s)
- Image loading into cluster (k8s)
- Execution with proper environment

**Note:** For `k8s` runs against non-local clusters (e.g. EKS), the cluster pulls images from a registry. In that case, build + push your image separately (see `scripts/build/build_test_image.sh`) and set `LOGOS_BLOCKCHAIN_TESTNET_IMAGE` to the pushed reference.

## Quick Smoke Matrix

For a small "does everything still run?" matrix across all runners:

```bash
scripts/run/run-test-matrix.sh -t 120 -n 1
```

This runs host, compose, and k8s modes with various image-build configurations. Useful after making runner/image/script changes. Forwards `--metrics-*` options through to `scripts/run/run-examples.sh`.

**Common options:**
- `--modes host,compose,k8s` — Restrict which modes run
- `--no-clean` — Skip `scripts/ops/clean.sh` step
- `--no-bundles` — Skip `scripts/build/build-bundle.sh` (reuses existing `.tmp` tarballs)
- `--no-image-build` — Skip the “rebuild image” variants in the matrix (compose/k8s)
- `--allow-nonzero-progress` — Soft-pass expectation failures if logs show non-zero progress (local iteration only)
- `--force-k8s-image-build` — Allow the k8s image-build variant even on non-docker-desktop clusters

**Environment overrides:**
- `VERSION=v0.3.1` — Circuit version
- `LOGOS_BLOCKCHAIN_NODE_REV=<commit>` — logos-blockchain-node git revision
- `LOGOS_BLOCKCHAIN_BINARIES_TAR=path/to/bundle.tar.gz` — Use prebuilt bundle
- `LOGOS_BLOCKCHAIN_SKIP_IMAGE_BUILD=1` — Skip image rebuild inside `run-examples.sh` (compose/k8s)
- `LOGOS_BLOCKCHAIN_BUNDLE_DOCKER_PLATFORM=linux/arm64|linux/amd64` — Docker platform for bundle builds (macOS/Windows)
- `COMPOSE_CIRCUITS_PLATFORM=linux-aarch64|linux-x86_64` — Circuits platform for image builds
- `SLOW_TEST_ENV=true` — Doubles built-in readiness timeouts (useful in CI / constrained laptops)
- `TESTNET_PRINT_ENDPOINTS=1` — Print `TESTNET_ENDPOINTS` / `TESTNET_PPROF` lines during deploy

## Dev Workflow: Updating logos-blockchain-node Revision

The repo pins a `logos-blockchain-node` revision in `versions.env` for reproducible builds. To update it or point to a local checkout:

```bash
# Pin to a new git revision (updates versions.env + Cargo.toml git revs)
scripts/ops/update-nomos-rev.sh --rev <git_sha>

# Use a local logos-blockchain-node checkout instead (for development)
scripts/ops/update-nomos-rev.sh --path /path/to/logos-blockchain-node

# If Cargo.toml was marked skip-worktree, clear it
scripts/ops/update-nomos-rev.sh --unskip-worktree
```

**Notes:**
- Don't commit absolute `LOGOS_BLOCKCHAIN_NODE_PATH` values; prefer `--rev` for shared history/CI
- After changing rev/path, expect `Cargo.lock` to update on the next `cargo build`/`cargo test`

## Cleanup Helper

If you hit Docker build failures, I/O errors, or disk space issues:

```bash
scripts/ops/clean.sh
```

For extra Docker cache cleanup:

```bash
scripts/ops/clean.sh --docker
```

---

## Host Runner (Direct Cargo Run)

For manual control, run the `local_runner` binary directly:

```bash
POL_PROOF_DEV_MODE=true \
LOGOS_BLOCKCHAIN_NODE_BIN=/path/to/logos-blockchain-node \
cargo run -p runner-examples --bin local_runner
```

### Host Runner Environment Variables

| Variable | Default | Effect |
|----------|---------|--------|
| `LOGOS_BLOCKCHAIN_DEMO_NODES` | 1 | Number of nodes (legacy: `LOCAL_DEMO_NODES`) |
| `LOGOS_BLOCKCHAIN_DEMO_RUN_SECS` | 60 | Run duration in seconds (legacy: `LOCAL_DEMO_RUN_SECS`) |
| `LOGOS_BLOCKCHAIN_NODE_BIN` | — | Path to logos-blockchain-node binary (required) |
| `LOGOS_BLOCKCHAIN_LOG_DIR` | None | Directory for per-node log files |
| `LOGOS_BLOCKCHAIN_TESTS_KEEP_LOGS` | 0 | Keep per-run temporary directories (useful for debugging/CI) |
| `LOGOS_BLOCKCHAIN_TESTS_TRACING` | false | Enable debug tracing preset |
| `LOGOS_BLOCKCHAIN_LOG_LEVEL` | info | Global log level: error, warn, info, debug, trace |
| `LOGOS_BLOCKCHAIN_LOG_FILTER` | None | Fine-grained module filtering (e.g., `cryptarchia=trace`) |
| `POL_PROOF_DEV_MODE` | — | **REQUIRED**: Set to `true` for all runners |

**Note:** Requires circuit assets and host binaries. Use `scripts/run/run-examples.sh host` to handle setup automatically.

---

## Compose Runner (Direct Cargo Run)

For manual control, run the `compose_runner` binary directly. Compose requires a Docker image with embedded assets.

### Option 1: Prebuilt Bundle (Recommended)

```bash
# 1. Build a Linux bundle (includes binaries + circuits)
scripts/build/build-bundle.sh --platform linux
# Creates .tmp/nomos-binaries-linux-v0.3.1.tar.gz

# 2. Build image (embeds bundle assets)
export LOGOS_BLOCKCHAIN_BINARIES_TAR=.tmp/nomos-binaries-linux-v0.3.1.tar.gz
scripts/build/build_test_image.sh

# 3. Run
LOGOS_BLOCKCHAIN_TESTNET_IMAGE=logos-blockchain-testing:local \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin compose_runner
```

### Option 2: Manual Circuit/Image Setup

```bash
# Fetch circuits
scripts/setup/setup-logos-blockchain-circuits.sh v0.3.1 ~/.logos-blockchain-circuits

# Build image
scripts/build/build_test_image.sh

# Run
LOGOS_BLOCKCHAIN_TESTNET_IMAGE=logos-blockchain-testing:local \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin compose_runner
```

### Platform Note (macOS / Apple Silicon)

- Docker Desktop runs a `linux/arm64` engine by default
- For native performance: `LOGOS_BLOCKCHAIN_BUNDLE_DOCKER_PLATFORM=linux/arm64` (recommended for local testing)
- For amd64 targets: `LOGOS_BLOCKCHAIN_BUNDLE_DOCKER_PLATFORM=linux/amd64` (slower via emulation)

### Compose Runner Environment Variables

| Variable | Default | Effect |
|----------|---------|--------|
| `LOGOS_BLOCKCHAIN_TESTNET_IMAGE` | — | Image tag (required, must match built image) |
| `POL_PROOF_DEV_MODE` | — | **REQUIRED**: Set to `true` for all runners |
| `LOGOS_BLOCKCHAIN_DEMO_NODES` | 1 | Number of nodes |
| `LOGOS_BLOCKCHAIN_DEMO_RUN_SECS` | 60 | Run duration in seconds |
| `COMPOSE_NODE_PAIRS` | — | Alternative topology format: "nodes" (e.g., `3`) |
| `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL` | None | Prometheus-compatible base URL for runner to query |
| `LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL` | None | Full OTLP HTTP ingest URL for node metrics export |
| `LOGOS_BLOCKCHAIN_GRAFANA_URL` | None | Grafana base URL for printing/logging |
| `COMPOSE_RUNNER_HOST` | 127.0.0.1 | Host address for port mappings |
| `COMPOSE_RUNNER_PRESERVE` | 0 | Keep containers running after test |
| `LOGOS_BLOCKCHAIN_LOG_LEVEL` | info | Node log level (stdout/stderr) |
| `LOGOS_BLOCKCHAIN_LOG_FILTER` | None | Fine-grained module filtering |

**Config file option:** `testing-framework/assets/stack/cfgsync.yaml` (`tracing_settings.logger`) — Switch node logs between stdout/stderr and file output

### Compose-Specific Features

- **Node control support**: Only runner that supports chaos testing (`.enable_node_control()` + chaos workloads)
- **External observability**: Set `LOGOS_BLOCKCHAIN_METRICS_*` / `LOGOS_BLOCKCHAIN_GRAFANA_URL` to enable telemetry links and querying
  - Quickstart: `scripts/setup/setup-observability.sh compose up` then `scripts/setup/setup-observability.sh compose env`

**Important:**
- Containers expect circuits at `/opt/circuits` (set by the image build)
- Use `scripts/run/run-examples.sh compose` to handle all setup automatically

---

## K8s Runner (Direct Cargo Run)

For manual control, run the `k8s_runner` binary directly. K8s requires the same image setup as Compose.

### Prerequisites

1. **Kubernetes cluster** with `kubectl` configured
2. **Test image built** (same as Compose, preferably with prebuilt bundle)
3. **Image available in cluster** (loaded or pushed to registry)

### Build and Load Image

```bash
# 1. Build image with bundle (recommended)
scripts/build/build-bundle.sh --platform linux
export LOGOS_BLOCKCHAIN_BINARIES_TAR=.tmp/nomos-binaries-linux-v0.3.1.tar.gz
scripts/build/build_test_image.sh

# 2. Load into cluster (choose one)
export LOGOS_BLOCKCHAIN_TESTNET_IMAGE=logos-blockchain-testing:local

# For kind:
kind load docker-image logos-blockchain-testing:local

# For minikube:
minikube image load logos-blockchain-testing:local

# For remote cluster (push to registry):
docker tag logos-blockchain-testing:local your-registry/logos-blockchain-testing:latest
docker push your-registry/logos-blockchain-testing:latest
export LOGOS_BLOCKCHAIN_TESTNET_IMAGE=your-registry/logos-blockchain-testing:latest
```

### Run the Example

```bash
export LOGOS_BLOCKCHAIN_TESTNET_IMAGE=logos-blockchain-testing:local
export POL_PROOF_DEV_MODE=true
cargo run -p runner-examples --bin k8s_runner
```

### K8s Runner Environment Variables

| Variable | Default | Effect |
|----------|---------|--------|
| `LOGOS_BLOCKCHAIN_TESTNET_IMAGE` | — | Image tag (required) |
| `POL_PROOF_DEV_MODE` | — | **REQUIRED**: Set to `true` for all runners |
| `LOGOS_BLOCKCHAIN_DEMO_NODES` | 1 | Number of nodes |
| `LOGOS_BLOCKCHAIN_DEMO_RUN_SECS` | 60 | Run duration in seconds |
| `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL` | None | Prometheus-compatible base URL for runner to query (PromQL) |
| `LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL` | None | Full OTLP HTTP ingest URL for node metrics export |
| `LOGOS_BLOCKCHAIN_GRAFANA_URL` | None | Grafana base URL for printing/logging |
| `K8S_RUNNER_NAMESPACE` | Random | Kubernetes namespace (pin for debugging) |
| `K8S_RUNNER_RELEASE` | Random | Helm release name (pin for debugging) |
| `K8S_RUNNER_NODE_HOST` | — | NodePort host resolution for non-local clusters |
| `K8S_RUNNER_DEBUG` | 0 | Log Helm stdout/stderr for install commands |
| `K8S_RUNNER_PRESERVE` | 0 | Keep namespace/release after run (for debugging) |

### K8s + Observability (Optional)

```bash
export LOGOS_BLOCKCHAIN_METRICS_QUERY_URL=http://your-prometheus:9090
# Prometheus OTLP receiver example:
export LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL=http://your-prometheus:9090/api/v1/otlp/v1/metrics
# Optional: print Grafana link in TESTNET_ENDPOINTS
export LOGOS_BLOCKCHAIN_GRAFANA_URL=http://your-grafana:3000
cargo run -p runner-examples --bin k8s_runner
```

**Notes:**
- `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL` must be reachable from the runner process (often via `kubectl port-forward`)
- `LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL` must be reachable from nodes (pods/containers) and is backend-specific
  - Quickstart installer: `scripts/setup/setup-observability.sh k8s install` then `scripts/setup/setup-observability.sh k8s env`
  - Optional dashboards: `scripts/setup/setup-observability.sh k8s dashboards`

### Via `scripts/run/run-examples.sh` (Recommended)

```bash
scripts/run/run-examples.sh -t 60 -n 3 k8s \
  --metrics-query-url http://your-prometheus:9090 \
  --metrics-otlp-ingest-url http://your-prometheus:9090/api/v1/otlp/v1/metrics
```

### In Code (Optional)

```rust,ignore
use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ObservabilityBuilderExt as _;

let plan = ScenarioBuilder::with_node_counts(1)
    .with_metrics_query_url_str("http://your-prometheus:9090")
    .with_metrics_otlp_ingest_url_str("http://your-prometheus:9090/api/v1/otlp/v1/metrics")
    .build();
```

### Important K8s Notes

- K8s runner uses circuits baked into the image
- File path inside pods: `/opt/circuits`
- **No node control support yet**: Chaos workloads (`.enable_node_control()`) will fail
- Optimized for local clusters (Docker Desktop K8s / minikube / kind)
  - Remote clusters require additional setup (registry push, PV/CSI for assets, etc.)
- Use `scripts/run/run-examples.sh k8s` to handle all setup automatically

## Next Steps

- [CI Integration](ci-integration.md) — Automate tests in continuous integration
- [Environment Variables](environment-variables.md) — Full variable reference
- [Logging & Observability](logging-observability.md) — Log collection and metrics
- [Troubleshooting](troubleshooting.md) — Common issues and fixes

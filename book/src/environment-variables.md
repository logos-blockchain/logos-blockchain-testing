# Environment Variables Reference

Complete reference of environment variables used by the testing framework, organized by category.

## Critical Variables

These MUST be set for successful test runs:

| Variable | Required | Default | Effect |
|----------|----------|---------|--------|
| `POL_PROOF_DEV_MODE` | **YES** | — | **REQUIRED for all runners**. Set to `true` to use fast dev-mode proving instead of expensive Groth16. Without this, tests will hang/timeout. |

**Example:**

```bash
export POL_PROOF_DEV_MODE=true
```

Or add to your shell profile (`~/.bashrc`, `~/.zshrc`):

```bash
# Required for nomos-testing framework
export POL_PROOF_DEV_MODE=true
```

---

## Runner Selection & Topology

Control which runner to use and the test topology:

| Variable | Default | Effect |
|----------|---------|--------|
| `NOMOS_DEMO_VALIDATORS` | 1 | Number of validators (all runners) |
| `NOMOS_DEMO_EXECUTORS` | 1 | Number of executors (all runners) |
| `NOMOS_DEMO_RUN_SECS` | 60 | Run duration in seconds (all runners) |
| `LOCAL_DEMO_VALIDATORS` | — | Legacy: Number of validators (host runner only) |
| `LOCAL_DEMO_EXECUTORS` | — | Legacy: Number of executors (host runner only) |
| `LOCAL_DEMO_RUN_SECS` | — | Legacy: Run duration (host runner only) |
| `COMPOSE_NODE_PAIRS` | — | Compose-specific topology format: "validators×executors" (e.g., `3x2`) |

**Example:**

```bash
# Run with 5 validators, 2 executors, for 120 seconds
NOMOS_DEMO_VALIDATORS=5 \
NOMOS_DEMO_EXECUTORS=2 \
NOMOS_DEMO_RUN_SECS=120 \
scripts/run/run-examples.sh -t 120 -v 5 -e 2 host
```

---

## Node Binaries (Host Runner)

Required for host runner when not using helper scripts:

| Variable | Required | Default | Effect |
|----------|----------|---------|--------|
| `NOMOS_NODE_BIN` | Yes (host) | — | Path to `nomos-node` binary |
| `NOMOS_EXECUTOR_BIN` | Yes (host) | — | Path to `nomos-executor` binary |
| `NOMOS_NODE_PATH` | No | — | Path to nomos-node git checkout (dev workflow) |

**Example:**

```bash
export NOMOS_NODE_BIN=/path/to/nomos-node/target/release/nomos-node
export NOMOS_EXECUTOR_BIN=/path/to/nomos-node/target/release/nomos-executor
```

---

## Docker Images (Compose / K8s)

Required for compose and k8s runners:

| Variable | Required | Default | Effect |
|----------|----------|---------|--------|
| `NOMOS_TESTNET_IMAGE` | Yes (compose/k8s) | `logos-blockchain-testing:local` | Docker image tag for node containers |
| `NOMOS_TESTNET_IMAGE_PULL_POLICY` | No | `IfNotPresent` (local) / `Always` (ECR) | K8s `imagePullPolicy` used by the runner |
| `NOMOS_BINARIES_TAR` | No | — | Path to prebuilt bundle (`.tar.gz`) for image build |
| `NOMOS_SKIP_IMAGE_BUILD` | No | 0 | Skip image rebuild (compose/k8s); assumes image already exists |
| `NOMOS_FORCE_IMAGE_BUILD` | No | 0 | Force rebuilding the image even when the script would normally skip it (e.g. non-local k8s) |

**Example:**

```bash
# Using prebuilt bundle
export NOMOS_BINARIES_TAR=.tmp/nomos-binaries-linux-v0.3.1.tar.gz
export NOMOS_TESTNET_IMAGE=logos-blockchain-testing:local
scripts/build/build_test_image.sh

# Using pre-existing image (skip build)
export NOMOS_SKIP_IMAGE_BUILD=1
scripts/run/run-examples.sh -t 60 -v 3 -e 1 compose
```

---

## Circuit Assets (KZG Parameters)

Circuit asset configuration for DA workloads:

| Variable | Default | Effect |
|----------|---------|--------|
| `NOMOS_KZGRS_PARAMS_PATH` | `testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params` | Path to KZG proving key file |
| `NOMOS_KZG_DIR_REL` | `testing-framework/assets/stack/kzgrs_test_params` | Directory containing KZG assets (relative to workspace root) |
| `NOMOS_KZG_FILE` | `kzgrs_test_params` | Filename of the proving key within `NOMOS_KZG_DIR_REL` |
| `NOMOS_KZG_CONTAINER_PATH` | `/kzgrs_test_params/kzgrs_test_params` | File path where the node expects KZG params inside containers |
| `NOMOS_KZG_MODE` | Runner-specific | K8s only: `hostPath` (mount from host) or `inImage` (embed into image) |
| `NOMOS_KZG_IN_IMAGE_PARAMS_PATH` | `/opt/nomos/kzg-params/kzgrs_test_params` | K8s `inImage` mode: where the proving key is stored inside the image |
| `VERSION` | From `versions.env` | Circuit release tag (used by helper scripts) |
| `NOMOS_CIRCUITS` | — | Directory containing fetched circuit bundles (set by `scripts/setup/setup-circuits-stack.sh`) |
| `NOMOS_CIRCUITS_VERSION` | — | Legacy alias for `VERSION` (supported by some build scripts) |
| `NOMOS_CIRCUITS_PLATFORM` | Auto-detected | Override circuits platform (e.g. `linux-x86_64`, `macos-aarch64`) |
| `NOMOS_CIRCUITS_HOST_DIR_REL` | `.tmp/nomos-circuits-host` | Output dir for host circuits bundle (relative to repo root) |
| `NOMOS_CIRCUITS_LINUX_DIR_REL` | `.tmp/nomos-circuits-linux` | Output dir for linux circuits bundle (relative to repo root) |
| `NOMOS_CIRCUITS_NONINTERACTIVE` | 0 | Set to `1` to overwrite outputs without prompting in setup scripts |
| `NOMOS_CIRCUITS_REBUILD_RAPIDSNARK` | 0 | Set to `1` to force rebuilding rapidsnark (host bundle only) |

**Example:**

```bash
# Use custom circuit assets
NOMOS_KZGRS_PARAMS_PATH=/custom/path/to/kzgrs_test_params \
cargo run -p runner-examples --bin local_runner
```

---

## Node Logging

Control node log output (not framework runner logs):

| Variable | Default | Effect |
|----------|---------|--------|
| `NOMOS_LOG_LEVEL` | `info` | Global log level: `error`, `warn`, `info`, `debug`, `trace` |
| `NOMOS_LOG_FILTER` | — | Fine-grained module filtering (e.g., `cryptarchia=trace,nomos_da_sampling=debug`) |
| `NOMOS_LOG_DIR` | — | Host runner: directory for per-node log files (persistent). Compose/k8s: use `cfgsync.yaml` for file logging. |
| `NOMOS_TESTS_KEEP_LOGS` | 0 | Keep per-run temporary directories (useful for debugging/CI artifacts) |
| `NOMOS_TESTS_TRACING` | false | Enable debug tracing preset (combine with `NOMOS_LOG_DIR` unless external tracing backends configured) |

**Important:** Node logging ignores `RUST_LOG`; use `NOMOS_LOG_LEVEL` and `NOMOS_LOG_FILTER` for node logs.

**Example:**

```bash
# Debug logging to files
NOMOS_LOG_DIR=/tmp/test-logs \
NOMOS_LOG_LEVEL=debug \
NOMOS_LOG_FILTER="cryptarchia=trace,nomos_da_sampling=debug" \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin local_runner

# Inspect logs
ls /tmp/test-logs/
# nomos-node-0.2024-12-18T14-30-00.log
# nomos-node-1.2024-12-18T14-30-00.log
```

**Common filter targets:**

| Target Prefix | Subsystem |
|---------------|-----------|
| `cryptarchia` | Consensus (Cryptarchia) |
| `nomos_da_sampling` | DA sampling service |
| `nomos_da_dispersal` | DA dispersal service |
| `nomos_da_verifier` | DA verification |
| `nomos_blend` | Mix network/privacy layer |
| `chain_service` | Chain service (node APIs/state) |
| `chain_network` | P2P networking |
| `chain_leader` | Leader election |

---

## Observability & Metrics

Optional observability integration:

| Variable | Default | Effect |
|----------|---------|--------|
| `NOMOS_METRICS_QUERY_URL` | — | Prometheus-compatible base URL for runner to query (e.g., `http://localhost:9090`) |
| `NOMOS_METRICS_OTLP_INGEST_URL` | — | Full OTLP HTTP ingest URL for node metrics export (e.g., `http://localhost:9090/api/v1/otlp/v1/metrics`) |
| `NOMOS_GRAFANA_URL` | — | Grafana base URL for printing/logging (e.g., `http://localhost:3000`) |
| `NOMOS_OTLP_ENDPOINT` | — | OTLP trace endpoint (optional) |
| `NOMOS_OTLP_METRICS_ENDPOINT` | — | OTLP metrics endpoint (optional) |

**Example:**

```bash
# Enable Prometheus querying
export NOMOS_METRICS_QUERY_URL=http://localhost:9090
export NOMOS_METRICS_OTLP_INGEST_URL=http://localhost:9090/api/v1/otlp/v1/metrics
export NOMOS_GRAFANA_URL=http://localhost:3000

scripts/run/run-examples.sh -t 60 -v 3 -e 1 compose
```

---

## Compose Runner Specific

Variables specific to Docker Compose deployment:

| Variable | Default | Effect |
|----------|---------|--------|
| `COMPOSE_RUNNER_HOST` | `127.0.0.1` | Host address for port mappings |
| `COMPOSE_RUNNER_PRESERVE` | 0 | Keep containers running after test (for debugging) |
| `COMPOSE_RUNNER_HTTP_TIMEOUT_SECS` | — | Override HTTP readiness timeout (seconds) |
| `COMPOSE_RUNNER_HOST_GATEWAY` | `host.docker.internal:host-gateway` | Controls `extra_hosts` entry injected into compose (set to `disable` to omit) |
| `TESTNET_RUNNER_PRESERVE` | — | Alias for `COMPOSE_RUNNER_PRESERVE` |

**Example:**

```bash
# Keep containers after test for debugging
COMPOSE_RUNNER_PRESERVE=1 \
scripts/run/run-examples.sh -t 60 -v 3 -e 1 compose

# Containers remain running
docker ps --filter "name=nomos-compose-"
docker logs <container-id>
```

---

## K8s Runner Specific

Variables specific to Kubernetes deployment:

| Variable | Default | Effect |
|----------|---------|--------|
| `K8S_RUNNER_NAMESPACE` | Random UUID | Kubernetes namespace (pin for debugging) |
| `K8S_RUNNER_RELEASE` | Random UUID | Helm release name (pin for debugging) |
| `K8S_RUNNER_NODE_HOST` | — | NodePort host resolution for non-local clusters |
| `K8S_RUNNER_DEBUG` | 0 | Log Helm stdout/stderr for install commands |
| `K8S_RUNNER_PRESERVE` | 0 | Keep namespace/release after run (for debugging) |
| `K8S_RUNNER_DEPLOYMENT_TIMEOUT_SECS` | — | Override deployment readiness timeout |
| `K8S_RUNNER_HTTP_TIMEOUT_SECS` | — | Override HTTP readiness timeout (port-forwards) |
| `K8S_RUNNER_HTTP_PROBE_TIMEOUT_SECS` | — | Override HTTP readiness timeout (NodePort probes) |
| `K8S_RUNNER_PROMETHEUS_HTTP_TIMEOUT_SECS` | — | Override Prometheus readiness timeout |
| `K8S_RUNNER_PROMETHEUS_HTTP_PROBE_TIMEOUT_SECS` | — | Override Prometheus NodePort probe timeout |

**Example:**

```bash
# Pin namespace for debugging
K8S_RUNNER_NAMESPACE=nomos-test-debug \
K8S_RUNNER_PRESERVE=1 \
K8S_RUNNER_DEBUG=1 \
scripts/run/run-examples.sh -t 60 -v 3 -e 1 k8s

# Inspect resources
kubectl get pods -n nomos-test-debug
kubectl logs -n nomos-test-debug -l nomos/logical-role=validator
```

---

## Platform & Build Configuration

Platform-specific build configuration:

| Variable | Default | Effect |
|----------|---------|--------|
| `NOMOS_BUNDLE_DOCKER_PLATFORM` | Host arch | Docker platform for bundle builds: `linux/arm64` or `linux/amd64` (macOS/Windows hosts) |
| `NOMOS_BIN_PLATFORM` | — | Legacy alias for `NOMOS_BUNDLE_DOCKER_PLATFORM` |
| `COMPOSE_CIRCUITS_PLATFORM` | Host arch | Circuits platform for image builds: `linux-aarch64` or `linux-x86_64` |
| `NOMOS_EXTRA_FEATURES` | — | Extra cargo features to enable when building bundles (used by `scripts/build/build-bundle.sh`) |

**macOS / Apple Silicon:**

```bash
# Native performance (recommended for local testing)
export NOMOS_BUNDLE_DOCKER_PLATFORM=linux/arm64

# Or target amd64 (slower via emulation)
export NOMOS_BUNDLE_DOCKER_PLATFORM=linux/amd64
```

---

## Timeouts & Performance

Timeout and performance tuning:

| Variable | Default | Effect |
|----------|---------|--------|
| `SLOW_TEST_ENV` | false | Doubles built-in readiness timeouts (useful in CI / constrained laptops) |
| `TESTNET_PRINT_ENDPOINTS` | 0 | Print `TESTNET_ENDPOINTS` / `TESTNET_PPROF` lines during deploy (set automatically by `scripts/run/run-examples.sh`) |
| `NOMOS_DISPERSAL_TIMEOUT_SECS` | 20 | DA dispersal timeout (seconds) |
| `NOMOS_RETRY_COOLDOWN_SECS` | 3 | Cooldown between retries (seconds) |
| `NOMOS_GRACE_PERIOD_SECS` | 1200 | Grace period before enforcing strict time-based expectations (seconds) |
| `NOMOS_PRUNE_DURATION_SECS` | 30 | Prune step duration (seconds) |
| `NOMOS_PRUNE_INTERVAL_SECS` | 5 | Interval between prune cycles (seconds) |
| `NOMOS_SHARE_DURATION_SECS` | 5 | Share duration (seconds) |
| `NOMOS_COMMITMENTS_WAIT_SECS` | 1 | Commitments wait duration (seconds) |
| `NOMOS_SDP_TRIGGER_DELAY_SECS` | 5 | SDP trigger delay (seconds) |

**Example:**

```bash
# Increase timeouts for slow environments
SLOW_TEST_ENV=true \
scripts/run/run-examples.sh -t 120 -v 5 -e 2 compose
```

---

## Node Configuration (Advanced)

Node-level configuration passed through to nomos-node/nomos-executor:

| Variable | Default | Effect |
|----------|---------|--------|
| `CONSENSUS_SLOT_TIME` | — | Consensus slot time (seconds) |
| `CONSENSUS_ACTIVE_SLOT_COEFF` | — | Active slot coefficient (0.0-1.0) |
| `NOMOS_USE_AUTONAT` | Unset | If set, use AutoNAT instead of a static loopback address for libp2p NAT settings |
| `NOMOS_CFGSYNC_PORT` | 4400 | Port used for cfgsync service inside the stack |
| `NOMOS_TIME_BACKEND` | `monotonic` | Select time backend (used by compose/k8s stack scripts and deployers) |

**Example:**

```bash
# Faster block production
CONSENSUS_SLOT_TIME=5 \
CONSENSUS_ACTIVE_SLOT_COEFF=0.9 \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin local_runner
```

---

## Framework Runner Logging (Not Node Logs)

Control framework runner process logs (uses `RUST_LOG`, not `NOMOS_*`):

| Variable | Default | Effect |
|----------|---------|--------|
| `RUST_LOG` | — | Framework runner log level (e.g., `debug`, `info`) |
| `RUST_BACKTRACE` | — | Enable Rust backtraces on panic (`1` or `full`) |
| `CARGO_TERM_COLOR` | — | Cargo output color (`always`, `never`, `auto`) |

**Example:**

```bash
# Debug framework runner (not nodes)
RUST_LOG=debug \
RUST_BACKTRACE=1 \
cargo run -p runner-examples --bin local_runner
```

---

## Helper Script Variables

Variables used by helper scripts (`scripts/run/run-examples.sh`, etc.):

| Variable | Default | Effect |
|----------|---------|--------|
| `NOMOS_NODE_REV` | From `versions.env` | nomos-node git revision to build/fetch |
| `NOMOS_BUNDLE_VERSION` | From `versions.env` | Bundle schema version |
| `NOMOS_IMAGE_SELECTION` | — | Internal: image selection mode set by `run-examples.sh` (`local`/`ecr`/`auto`) |
| `NOMOS_NODE_APPLY_PATCHES` | 1 | Set to `0` to disable applying local patches when building bundles |
| `NOMOS_NODE_PATCH_DIR` | `patches/nomos-node` | Patch directory applied to nomos-node checkout during bundle builds |
| `NOMOS_NODE_PATCH_LEVEL` | — | Patch application level (`all` or an integer) for bundle builds |

---

## Quick Reference Examples

### Minimal Host Run

```bash
POL_PROOF_DEV_MODE=true \
scripts/run/run-examples.sh -t 60 -v 3 -e 1 host
```

### Debug Logging (Host)

```bash
POL_PROOF_DEV_MODE=true \
NOMOS_LOG_DIR=/tmp/logs \
NOMOS_LOG_LEVEL=debug \
NOMOS_LOG_FILTER="cryptarchia=trace" \
scripts/run/run-examples.sh -t 60 -v 3 -e 1 host
```

### Compose with Observability

```bash
POL_PROOF_DEV_MODE=true \
NOMOS_METRICS_QUERY_URL=http://localhost:9090 \
NOMOS_GRAFANA_URL=http://localhost:3000 \
scripts/run/run-examples.sh -t 60 -v 3 -e 1 compose
```

### K8s with Debug

```bash
POL_PROOF_DEV_MODE=true \
K8S_RUNNER_NAMESPACE=nomos-debug \
K8S_RUNNER_DEBUG=1 \
K8S_RUNNER_PRESERVE=1 \
scripts/run/run-examples.sh -t 60 -v 3 -e 1 k8s
```

### CI Environment

```yaml
env:
  POL_PROOF_DEV_MODE: true
  RUST_BACKTRACE: 1
  NOMOS_TESTS_KEEP_LOGS: 1
```

---

## See Also

- [Prerequisites & Setup](prerequisites.md) — Required files and setup
- [Running Examples](running-examples.md) — How to run scenarios
- [Logging & Observability](logging-observability.md) — Log collection details
- [CI Integration](ci-integration.md) — CI-specific variables
- [Troubleshooting](troubleshooting.md) — Common issues with variables

# Operations

Operational readiness focuses on prerequisites, environment fit, and clear
signals:

- **Prerequisites**: keep a sibling `nomos-node` checkout available; ensure the
  chosen runner’s platform needs are met (local binaries for host runs, Docker
  for compose, cluster access for k8s).
- **Artifacts**: DA scenarios require KZG parameters (circuit assets) located at
  `testing-framework/assets/stack/kzgrs_test_params`. Fetch them via
  `scripts/setup-nomos-circuits.sh` or override the path with `NOMOS_KZGRS_PARAMS_PATH`.
- **Environment flags**: `POL_PROOF_DEV_MODE=true` is **required for all runners**
  (local, compose, k8s) unless you want expensive Groth16 proof generation that
  will cause tests to timeout. Configure logging via `NOMOS_LOG_DIR`, `NOMOS_LOG_LEVEL`,
  and `NOMOS_LOG_FILTER` (see [Logging and Observability](#logging-and-observability)
  for details). Note that nodes ignore `RUST_LOG` and only respond to `NOMOS_*` variables.
- **Readiness checks**: verify runners report node readiness before starting
  workloads; this avoids false negatives from starting too early.
- **Failure triage**: map failures to missing prerequisites (wallet seeding,
  node control availability), runner platform issues, or unmet expectations.
  Start with liveness signals, then dive into workload-specific assertions.

Treat operational hygiene—assets present, prerequisites satisfied, observability
reachable—as the first step to reliable scenario outcomes.

## CI Usage

Both **LocalDeployer** and **ComposeDeployer** work in CI environments:

**LocalDeployer in CI:**
- Faster (no Docker overhead)
- Good for quick smoke tests
- **Trade-off:** Less isolation (processes share host)

**ComposeDeployer in CI (recommended):**
- Better isolation (containerized)
- Reproducible environment
- Includes Prometheus/observability
- **Trade-off:** Slower startup (Docker image build)
- **Trade-off:** Requires Docker daemon

See `.github/workflows/compose-mixed.yml` for a complete CI example using ComposeDeployer.

## Running Examples

The framework provides three runner modes: **host** (local processes), **compose** (Docker Compose), and **k8s** (Kubernetes).

**Recommended:** Use `scripts/run-examples.sh` for all modes:

```bash
# Host mode (local processes)
scripts/run-examples.sh -t 60 -v 1 -e 1 host

# Compose mode (Docker Compose)
scripts/run-examples.sh -t 60 -v 1 -e 1 compose

# K8s mode (Kubernetes)
scripts/run-examples.sh -t 60 -v 1 -e 1 k8s
```

This script handles circuit setup, binary building/bundling, image building, and execution.

**Environment overrides:**
- `VERSION=v0.3.1` — Circuit version
- `NOMOS_NODE_REV=<commit>` — nomos-node git revision
- `NOMOS_BINARIES_TAR=path/to/bundle.tar.gz` — Use prebuilt bundle
- `NOMOS_SKIP_IMAGE_BUILD=1` — Skip image rebuild (compose/k8s)

### Host Runner (Direct Cargo Run)

For manual control, you can run the `local_runner` binary directly:

```bash
POL_PROOF_DEV_MODE=true \
NOMOS_NODE_BIN=/path/to/nomos-node \
NOMOS_EXECUTOR_BIN=/path/to/nomos-executor \
cargo run -p runner-examples --bin local_runner
```

**Environment variables:**
- `NOMOS_DEMO_VALIDATORS=3` — Number of validators (default: 1, or use legacy `LOCAL_DEMO_VALIDATORS`)
- `NOMOS_DEMO_EXECUTORS=2` — Number of executors (default: 1, or use legacy `LOCAL_DEMO_EXECUTORS`)
- `NOMOS_DEMO_RUN_SECS=120` — Run duration in seconds (default: 60, or use legacy `LOCAL_DEMO_RUN_SECS`)
- `NOMOS_NODE_BIN` / `NOMOS_EXECUTOR_BIN` — Paths to binaries (required for direct run)
- `NOMOS_TESTS_TRACING=true` — Enable persistent file logging
- `NOMOS_LOG_DIR=/tmp/logs` — Directory for per-node log files
- `NOMOS_LOG_LEVEL=debug` — Set log level (default: info)
- `NOMOS_LOG_FILTER=consensus=trace,da=debug` — Fine-grained module filtering

**Note:** Requires circuit assets and host binaries. Use `scripts/run-examples.sh host` to handle setup automatically.

### Compose Runner (Direct Cargo Run)

For manual control, you can run the `compose_runner` binary directly. Compose requires a Docker image with embedded assets.

**Recommended setup:** Use a prebuilt bundle:

```bash
# Build a Linux bundle (includes binaries + circuits)
scripts/build-bundle.sh --platform linux
# Creates .tmp/nomos-binaries-linux-v0.3.1.tar.gz

# Build image (embeds bundle assets)
export NOMOS_BINARIES_TAR=.tmp/nomos-binaries-linux-v0.3.1.tar.gz
testing-framework/assets/stack/scripts/build_test_image.sh

# Run
NOMOS_TESTNET_IMAGE=nomos-testnet:local \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin compose_runner
```

**Alternative:** Manual circuit/image setup (rebuilds during image build):

```bash
# Fetch and copy circuits
scripts/setup-nomos-circuits.sh v0.3.1 /tmp/nomos-circuits
cp -r /tmp/nomos-circuits/* testing-framework/assets/stack/kzgrs_test_params/

# Build image
testing-framework/assets/stack/scripts/build_test_image.sh

# Run
NOMOS_TESTNET_IMAGE=nomos-testnet:local \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin compose_runner
```

**Environment variables:**
- `NOMOS_TESTNET_IMAGE=nomos-testnet:local` — Image tag (required, must match built image)
- `POL_PROOF_DEV_MODE=true` — **Required** for all runners
- `NOMOS_DEMO_VALIDATORS=3` / `NOMOS_DEMO_EXECUTORS=2` / `NOMOS_DEMO_RUN_SECS=120` — Topology overrides
- `COMPOSE_NODE_PAIRS=1x1` — Alternative topology format: "validators×executors"
- `TEST_FRAMEWORK_PROMETHEUS_PORT=9091` — Override Prometheus port (default: 9090)
- `COMPOSE_RUNNER_HOST=127.0.0.1` — Host address for port mappings
- `COMPOSE_RUNNER_PRESERVE=1` — Keep containers running after test
- `NOMOS_LOG_DIR=/tmp/compose-logs` — Write logs to files inside containers

**Compose-specific features:**
- **Node control support**: Only runner that supports chaos testing (`.enable_node_control()` + chaos workloads)
- **Prometheus observability**: Metrics at `http://localhost:9090`

**Important:** 
- Containers expect KZG parameters at `/kzgrs_test_params/kzgrs_test_params` (note the repeated filename)
- Use `scripts/run-examples.sh compose` to handle all setup automatically

### K8s Runner (Direct Cargo Run)

For manual control, you can run the `k8s_runner` binary directly. K8s requires the same image setup as Compose.

**Prerequisites:**
1. **Kubernetes cluster** with `kubectl` configured
2. **Test image built** (same as Compose, preferably with prebuilt bundle)
3. **Image available in cluster** (loaded or pushed to registry)

**Build and load image:**
```bash
# Build image with bundle (recommended)
scripts/build-bundle.sh --platform linux
export NOMOS_BINARIES_TAR=.tmp/nomos-binaries-linux-v0.3.1.tar.gz
testing-framework/assets/stack/scripts/build_test_image.sh

# Load into cluster
export NOMOS_TESTNET_IMAGE=nomos-testnet:local
kind load docker-image nomos-testnet:local  # For kind
# OR: minikube image load nomos-testnet:local  # For minikube
# OR: docker push your-registry/nomos-testnet:local  # For remote
```

**Run the example:**
```bash
export NOMOS_TESTNET_IMAGE=nomos-testnet:local
export POL_PROOF_DEV_MODE=true
cargo run -p runner-examples --bin k8s_runner
```

**Environment variables:**
- `NOMOS_TESTNET_IMAGE` — Image tag (required)
- `POL_PROOF_DEV_MODE=true` — **Required** for all runners
- `NOMOS_DEMO_VALIDATORS` / `NOMOS_DEMO_EXECUTORS` / `NOMOS_DEMO_RUN_SECS` — Topology overrides

**Important:** 
- K8s runner mounts `testing-framework/assets/stack/kzgrs_test_params` as a hostPath volume with file `/kzgrs_test_params/kzgrs_test_params` inside pods
- **No node control support yet**: Chaos workloads (`.enable_node_control()`) will fail
- Use `scripts/run-examples.sh k8s` to handle all setup automatically

## Circuit Assets (KZG Parameters)

DA workloads require KZG cryptographic parameters for polynomial commitment schemes.

### Asset Location

**Default path:** `testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params`

Note the repeated filename: the directory `kzgrs_test_params/` contains a file named `kzgrs_test_params`. This is the actual proving key file.

**Container path** (compose/k8s): `/kzgrs_test_params/kzgrs_test_params`

**Override:** Set `NOMOS_KZGRS_PARAMS_PATH` to use a custom location (must point to the file):
```bash
NOMOS_KZGRS_PARAMS_PATH=/path/to/custom/params cargo run -p runner-examples --bin local_runner
```

### Getting Circuit Assets

**Option 1: Use helper script** (recommended):
```bash
# From the repository root
chmod +x scripts/setup-nomos-circuits.sh
scripts/setup-nomos-circuits.sh v0.3.1 /tmp/nomos-circuits

# Copy to default location
cp -r /tmp/nomos-circuits/* testing-framework/assets/stack/kzgrs_test_params/
```

**Option 2: Build locally** (advanced):
```bash
# Requires Go, Rust, and circuit build tools
make kzgrs_test_params
```

### CI Workflow

The CI automatically fetches and places assets:
```yaml
- name: Install circuits for host build
  run: |
    scripts/setup-nomos-circuits.sh v0.3.1 "$TMPDIR/nomos-circuits"
    cp -a "$TMPDIR/nomos-circuits"/. testing-framework/assets/stack/kzgrs_test_params/
```

### When Are Assets Needed?

| Runner | When Required |
|--------|---------------|
| **Local** | Always (for DA workloads) |
| **Compose** | During image build (baked into `NOMOS_TESTNET_IMAGE`) |
| **K8s** | During image build + deployed to cluster via hostPath volume |

**Error without assets:**
```
Error: missing KZG parameters at testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params
```

If you see this error, the file `kzgrs_test_params` is missing from the directory. Use `scripts/run-examples.sh` or `scripts/setup-nomos-circuits.sh` to fetch it.

## Logging and Observability

### Node Logging vs Framework Logging

**Critical distinction:** Node logs and framework logs use different configuration mechanisms.

| Component | Controlled By | Purpose |
|-----------|--------------|---------|
| **Framework binaries** (`cargo run -p runner-examples --bin local_runner`) | `RUST_LOG` | Runner orchestration, deployment logs |
| **Node processes** (validators, executors spawned by runner) | `NOMOS_LOG_LEVEL`, `NOMOS_LOG_FILTER`, `NOMOS_LOG_DIR` | Consensus, DA, mempool, network logs |

**Common mistake:** Setting `RUST_LOG=debug` only increases verbosity of the runner binary itself. Node logs remain at their default level unless you also set `NOMOS_LOG_LEVEL=debug`.

**Example:**
```bash
# This only makes the RUNNER verbose, not the nodes:
RUST_LOG=debug cargo run -p runner-examples --bin local_runner

# This makes the NODES verbose:
NOMOS_LOG_LEVEL=debug cargo run -p runner-examples --bin local_runner

# Both verbose (typically not needed):
RUST_LOG=debug NOMOS_LOG_LEVEL=debug cargo run -p runner-examples --bin local_runner
```

### Logging Environment Variables

| Variable | Default | Effect |
|----------|---------|--------|
| `NOMOS_LOG_DIR` | None (console only) | Directory for per-node log files. If unset, logs go to stdout/stderr. |
| `NOMOS_LOG_LEVEL` | `info` | Global log level: `error`, `warn`, `info`, `debug`, `trace` |
| `NOMOS_LOG_FILTER` | None | Fine-grained target filtering (e.g., `consensus=trace,da=debug`) |
| `NOMOS_TESTS_TRACING` | `false` | Enable tracing subscriber for local runner file logging |
| `NOMOS_OTLP_ENDPOINT` | None | OTLP trace endpoint (optional, disables OTLP noise if unset) |
| `NOMOS_OTLP_METRICS_ENDPOINT` | None | OTLP metrics endpoint (optional) |

**Example:** Full debug logging to files:
```bash
NOMOS_TESTS_TRACING=true \
NOMOS_LOG_DIR=/tmp/test-logs \
NOMOS_LOG_LEVEL=debug \
NOMOS_LOG_FILTER="nomos_consensus=trace,nomos_da_sampling=debug" \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin local_runner
```

### Per-Node Log Files

When `NOMOS_LOG_DIR` is set, each node writes logs to separate files:

**File naming pattern:**
- **Validators**: Prefix `nomos-node-0`, `nomos-node-1`, etc. (may include timestamp suffix)
- **Executors**: Prefix `nomos-executor-0`, `nomos-executor-1`, etc. (may include timestamp suffix)

**Local runner caveat:** By default, the local runner writes logs to temporary directories in the working directory. These are automatically cleaned up after tests complete. To preserve logs, you MUST set both `NOMOS_TESTS_TRACING=true` AND `NOMOS_LOG_DIR=/path/to/logs`.

### Filter Target Names

Common target prefixes for `NOMOS_LOG_FILTER`:

| Target Prefix | Subsystem |
|---------------|-----------|
| `nomos_consensus` | Consensus (Cryptarchia) |
| `nomos_da_sampling` | DA sampling service |
| `nomos_da_dispersal` | DA dispersal service |
| `nomos_da_verifier` | DA verification |
| `nomos_mempool` | Transaction mempool |
| `nomos_blend` | Mix network/privacy layer |
| `chain_network` | P2P networking |
| `chain_leader` | Leader election |

**Example filter:**
```bash
NOMOS_LOG_FILTER="nomos_consensus=trace,nomos_da_sampling=debug,chain_network=info"
```

### Accessing Logs Per Runner

#### Local Runner

**Default (temporary directories, auto-cleanup):**
```bash
POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin local_runner
# Logs written to temporary directories in working directory
# Automatically cleaned up after test completes
```

**Persistent file output:**
```bash
NOMOS_TESTS_TRACING=true \
NOMOS_LOG_DIR=/tmp/local-logs \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin local_runner

# After test completes:
ls /tmp/local-logs/
# Files with prefix: nomos-node-0*, nomos-node-1*, nomos-executor-0*
# May include timestamps in filename
```

**Both flags required:** You MUST set both `NOMOS_TESTS_TRACING=true` (enables tracing file sink) AND `NOMOS_LOG_DIR` (specifies directory) to get persistent logs.

#### Compose Runner

**Via Docker logs (default, recommended):**
```bash
# List containers (note the UUID prefix in names)
docker ps --filter "name=nomos-compose-"

# Stream logs from specific container
docker logs -f <container-id-or-name>

# Or use name pattern matching:
docker logs -f $(docker ps --filter "name=nomos-compose-.*-validator-0" -q | head -1)
```

**Via file collection (advanced):**

Setting `NOMOS_LOG_DIR` writes files **inside the container**. To access them, you must either:

1. **Copy files out after the run:**
```bash
NOMOS_LOG_DIR=/logs \
NOMOS_TESTNET_IMAGE=nomos-testnet:local \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin compose_runner

# After test, copy files from containers:
docker ps --filter "name=nomos-compose-"
docker cp <container-id>:/logs/nomos-node-0* /tmp/
```

2. **Mount a host volume** (requires modifying compose template):
```yaml
volumes:
  - /tmp/host-logs:/logs  # Add to docker-compose.yml.tera
```

**Recommendation:** Use `docker logs` by default. File collection inside containers is complex and rarely needed.

**Keep containers for debugging:**
```bash
COMPOSE_RUNNER_PRESERVE=1 \
NOMOS_TESTNET_IMAGE=nomos-testnet:local \
cargo run -p runner-examples --bin compose_runner
# Containers remain running after test—inspect with docker logs or docker exec
```

**Note:** Container names follow pattern `nomos-compose-{uuid}-validator-{index}-1` where `{uuid}` changes per run.

#### K8s Runner

**Via kubectl logs (use label selectors):**
```bash
# List pods
kubectl get pods

# Stream logs using label selectors (recommended)
kubectl logs -l app=nomos-validator -f
kubectl logs -l app=nomos-executor -f

# Stream logs from specific pod
kubectl logs -f nomos-validator-0

# Previous logs from crashed pods
kubectl logs --previous -l app=nomos-validator
```

**Download logs for offline analysis:**
```bash
# Using label selectors
kubectl logs -l app=nomos-validator --tail=1000 > all-validators.log
kubectl logs -l app=nomos-executor --tail=1000 > all-executors.log

# Specific pods
kubectl logs nomos-validator-0 > validator-0.log
kubectl logs nomos-executor-1 > executor-1.log
```

**Specify namespace (if not using default):**
```bash
kubectl logs -n my-namespace -l app=nomos-validator -f
```

### OTLP and Telemetry

**OTLP exporters are optional.** If you see errors about unreachable OTLP endpoints, it's safe to ignore them unless you're actively collecting traces/metrics.

**To enable OTLP:**
```bash
NOMOS_OTLP_ENDPOINT=http://localhost:4317 \
NOMOS_OTLP_METRICS_ENDPOINT=http://localhost:4318 \
cargo run -p runner-examples --bin local_runner
```

**To silence OTLP errors:** Simply leave these variables unset (the default).

### Observability: Prometheus and Node APIs

Runners expose metrics and node HTTP endpoints for expectation code and debugging:

**Prometheus (Compose only):**
- Default: `http://localhost:9090`
- Override: `TEST_FRAMEWORK_PROMETHEUS_PORT=9091`
- Access from expectations: `ctx.telemetry().prometheus_endpoint()`

**Node APIs:**
- Access from expectations: `ctx.node_clients().validators().get(0)`
- Endpoints: consensus info, network info, DA membership, etc.
- See `testing-framework/core/src/nodes/api_client.rs` for available methods

```mermaid
flowchart TD
    Expose[Runner exposes endpoints/ports] --> Collect[Runtime collects block/health signals]
    Collect --> Consume[Expectations consume signals<br/>decide pass/fail]
    Consume --> Inspect[Operators inspect logs/metrics<br/>when failures arise]
```

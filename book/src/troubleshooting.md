# Troubleshooting Scenarios

**Prerequisites for All Runners:**
- **`versions.env` file** at repository root (required by helper scripts)
- **`POL_PROOF_DEV_MODE=true`** MUST be set for all runners (host, compose, k8s) to avoid expensive Groth16 proof generation that causes timeouts
- **KZG circuit assets** must be present at `testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params` (note the repeated filename) for DA workloads

**Platform/Environment Notes:**
- **macOS + Docker Desktop (Apple silicon):** prefer `NOMOS_BUNDLE_DOCKER_PLATFORM=linux/arm64` for local compose/k8s runs to avoid slow/fragile amd64 emulation builds.
- **Disk space:** bundle/image builds are storage-heavy. If you see I/O errors or Docker build failures, check free space and prune old artifacts (`.tmp/`, `target/`, and Docker build cache) before retrying.
- **K8s runner scope:** the default Helm chart mounts KZG params via `hostPath` and uses a local image tag (`logos-blockchain-testing:local`). This is intended for local clusters (Docker Desktop / minikube / kind), not remote managed clusters without additional setup.
  - Quick cleanup: `scripts/ops/clean.sh` (and `scripts/ops/clean.sh --docker` if needed).
  - Destructive cleanup (last resort): `scripts/ops/clean.sh --docker-system --dangerous` (add `--volumes` if you also want to prune Docker volumes).

**Recommended:** Use `scripts/run/run-examples.sh` which handles all setup automatically.

## Quick Symptom Guide

Common symptoms and likely causes:

- **No or slow block progression**: missing `POL_PROOF_DEV_MODE=true`, missing KZG circuit assets (`/kzgrs_test_params/kzgrs_test_params` file) for DA workloads, too-short run window, port conflicts, or resource exhaustion—set required env vars, verify assets exist, extend duration, check node logs for startup errors.
- **Transactions not included**: unfunded or misconfigured wallets (check `.wallets(N)` vs `.users(M)`), transaction rate exceeding block capacity, or rates exceeding block production speed—reduce rate, increase wallet count, verify wallet setup in logs.
- **Chaos stalls the run**: chaos (node control) only works with ComposeDeployer; host runner (LocalDeployer) and K8sDeployer don't support it (won't "stall", just can't execute chaos workloads). With compose, aggressive restart cadence can prevent consensus recovery—widen restart intervals.
- **Observability gaps**: metrics or logs unreachable because ports clash or services are not exposed—adjust observability ports and confirm runner wiring.
- **Flaky behavior across runs**: mixing chaos with functional smoke tests or inconsistent topology between environments—separate deterministic and chaos scenarios and standardize topology presets.

## What Failure Looks Like

This section shows what you'll actually see when common issues occur. Each example includes realistic console output and the fix.

### 1. Missing `POL_PROOF_DEV_MODE=true` (Most Common!)

**Symptoms:**
- Test "hangs" with no visible progress
- CPU usage spikes to 100%
- Eventually hits timeout after several minutes
- Nodes appear to start but blocks aren't produced

**What you'll see:**

```
$ cargo run -p runner-examples --bin local_runner
    Finished dev [unoptimized + debuginfo] target(s) in 0.48s
     Running `target/debug/local_runner`
[INFO  runner_examples::local_runner] Starting local runner scenario
[INFO  testing_framework_runner_local] Launching 3 validators
[INFO  testing_framework_runner_local] Waiting for node readiness...
(hangs here for 5+ minutes, CPU at 100%)
thread 'main' panicked at 'readiness timeout expired'
```

**Root Cause:** Groth16 proof generation is extremely slow without dev mode. The system tries to compute real cryptographic proofs, which can take minutes per block.

**Fix:**

```bash
POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin local_runner
```

**Prevention:** Set this in your shell profile or `.env` file so you never forget it.

---

### 2. Missing `versions.env` File

**Symptoms:**
- Helper scripts fail immediately
- Error about missing file at repo root
- Scripts can't determine which circuit/node versions to use

**What you'll see:**

```
$ scripts/run/run-examples.sh -t 60 -v 1 -e 1 host
ERROR: versions.env not found at repository root
This file is required and should define:
  VERSION=<circuit release tag>
  NOMOS_NODE_REV=<nomos-node git revision>
  NOMOS_BUNDLE_VERSION=<bundle schema version>
```

**Root Cause:** Helper scripts need `versions.env` to know which versions to build/fetch.

**Fix:** Ensure you're in the repository root directory. The `versions.env` file should already exist—verify it's present:

```bash
cat versions.env
# Should show:
# VERSION=v0.3.1
# NOMOS_NODE_REV=abc123def456
# NOMOS_BUNDLE_VERSION=v1
```

---

### 3. Missing KZG Circuit Assets (DA Workloads)

**Symptoms:**
- DA workload tests fail
- Error messages about missing circuit files
- Nodes crash during DA operations

**What you'll see:**

```
$ POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin local_runner
[INFO  testing_framework_runner_local] Starting DA workload
[ERROR nomos_da_dispersal] Failed to load KZG parameters
Error: Custom { kind: NotFound, error: "Circuit file not found at: testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params" }
thread 'main' panicked at 'workload init failed'
```

**Root Cause:** DA (Data Availability) workloads require KZG cryptographic parameters. The file must exist at: `testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params` (note the repeated filename).

**Fix (recommended):**

```bash
# Use run-examples.sh which handles setup automatically
scripts/run/run-examples.sh -t 60 -v 1 -e 1 host
```

**Fix (manual):**

```bash
# Fetch circuits
scripts/setup/setup-nomos-circuits.sh v0.3.1 /tmp/nomos-circuits

# Copy to expected location
mkdir -p testing-framework/assets/stack/kzgrs_test_params
cp -r /tmp/nomos-circuits/* testing-framework/assets/stack/kzgrs_test_params/

# Verify (should be ~120MB)
ls -lh testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params
```

---

### 4. Node Binaries Not Found

**Symptoms:**
- Error about missing `nomos-node` or `nomos-executor` binary
- "file not found" or "no such file or directory"
- Environment variables `NOMOS_NODE_BIN` / `NOMOS_EXECUTOR_BIN` not set

**What you'll see:**

```
$ POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin local_runner
[INFO  testing_framework_runner_local] Spawning validator 0
Error: Os { code: 2, kind: NotFound, message: "No such file or directory" }
thread 'main' panicked at 'failed to spawn nomos-node process'
```

**Root Cause:** The local runner needs compiled `nomos-node` and `nomos-executor` binaries, but doesn't know where they are.

**Fix (recommended):**

```bash
# Use run-examples.sh which builds binaries automatically
scripts/run/run-examples.sh -t 60 -v 1 -e 1 host
```

**Fix (manual - set paths explicitly):**

```bash
# Build binaries first
cd ../nomos-node  # or wherever your nomos-node checkout is
cargo build --release --bin nomos-node --bin nomos-executor

# Set environment variables
export NOMOS_NODE_BIN=$PWD/target/release/nomos-node
export NOMOS_EXECUTOR_BIN=$PWD/target/release/nomos-executor

# Return to testing framework
cd ../nomos-testing
POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin local_runner
```

---

### 5. Docker Daemon Not Running (Compose)

**Symptoms:**
- Compose tests fail immediately
- "Cannot connect to Docker daemon"
- Docker commands don't work

**What you'll see:**

```
$ scripts/run/run-examples.sh -t 60 -v 1 -e 1 compose
[INFO  runner_examples::compose_runner] Starting compose deployment
Error: Cannot connect to the Docker daemon at unix:///var/run/docker.sock. Is the docker daemon running?
thread 'main' panicked at 'compose deployment failed'
```

**Root Cause:** Docker Desktop isn't running, or your user doesn't have permission to access Docker.

**Fix:**

```bash
# macOS: Start Docker Desktop application
open -a Docker

# Linux: Start Docker daemon
sudo systemctl start docker

# Verify Docker is working
docker ps

# If permission denied, add your user to docker group (Linux)
sudo usermod -aG docker $USER
# Then log out and log back in
```

---

### 6. Image Not Found (Compose/K8s)

**Symptoms:**
- Compose/K8s tests fail during deployment
- "Image not found: logos-blockchain-testing:local"
- Containers fail to start

**What you'll see:**

```
$ POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin compose_runner
[INFO  testing_framework_runner_compose] Starting compose deployment
Error: Failed to pull image 'logos-blockchain-testing:local': No such image
thread 'main' panicked at 'compose deployment failed'
```

**Root Cause:** The Docker image hasn't been built yet, or was pruned.

**Fix (recommended):**

```bash
# Use run-examples.sh which builds the image automatically
scripts/run/run-examples.sh -t 60 -v 1 -e 1 compose
```

**Fix (manual):**

```bash
# 1. Build Linux bundle
scripts/build/build-bundle.sh --platform linux

# 2. Set bundle path
export NOMOS_BINARIES_TAR=$(ls -t .tmp/nomos-binaries-linux-*.tar.gz | head -1)

# 3. Build Docker image
scripts/build/build_test_image.sh

# 4. Verify image exists
docker images | grep logos-blockchain-testing

# 5. For kind/minikube: load image into cluster
kind load docker-image logos-blockchain-testing:local
# OR: minikube image load logos-blockchain-testing:local
```

---

### 7. Port Conflicts

**Symptoms:**
- "Address already in use" errors
- Tests fail during node startup
- Observability stack (Prometheus/Grafana) won't start

**What you'll see:**

```
$ POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin local_runner
[INFO  testing_framework_runner_local] Launching validator 0 on port 18080
Error: Os { code: 48, kind: AddrInUse, message: "Address already in use" }
thread 'main' panicked at 'failed to bind port 18080'
```

**Root Cause:** Previous test didn't clean up properly, or another service is using the port.

**Fix:**

```bash
# Find processes using the port
lsof -i :18080   # macOS/Linux
netstat -ano | findstr :18080  # Windows

# Kill orphaned nomos processes
pkill nomos-node
pkill nomos-executor

# For compose: ensure containers are stopped
docker compose down
docker ps -a --filter "name=nomos-compose-" -q | xargs docker rm -f

# Check if port is now free
lsof -i :18080  # Should return nothing
```

**For Observability Stack Port Conflicts:**

```bash
# Edit ports in observability compose file
vim scripts/observability/compose/docker-compose.yml

# Change conflicting port mappings:
# ports:
#   - "9090:9090"  # Prometheus - change to "19090:9090" if needed
#   - "3000:3000"  # Grafana - change to "13000:3000" if needed
```

---

### 8. Wallet Seeding Failed (Insufficient Funds)

**Symptoms:**
- Transaction workload reports wallet issues
- "Insufficient funds" errors
- Transactions aren't being submitted

**What you'll see:**

```
$ POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin local_runner
[INFO  testing_framework_workflows] Starting transaction workload with 10 users
[ERROR testing_framework_workflows] Wallet seeding failed: requested 10 users but only 3 wallets available
thread 'main' panicked at 'workload init failed: insufficient wallets'
```

**Root Cause:** Topology configured fewer wallets than the workload needs. Transaction workload has `.users(M)` but topology only has `.wallets(N)` where N < M.

**Fix:**

```rust
use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

let scenario = ScenarioBuilder::topology_with(|t| t.network_star().validators(3).executors(1))
    .wallets(20) // ← Increase wallet count
    .transactions_with(|tx| {
        tx.users(10) // ← Must be ≤ wallets(20)
            .rate(5)
    })
    .build();
```

---

### 9. Resource Exhaustion (OOM / CPU)

**Symptoms:**
- Nodes crash randomly
- "OOM Killed" messages
- Test becomes flaky under load
- Docker containers restart repeatedly

**What you'll see:**

```
$ docker ps --filter "name=nomos-compose-"
CONTAINER ID   STATUS
abc123def456   Restarting (137) 30 seconds ago  # 137 = OOM killed

$ docker logs abc123def456
[INFO  nomos_node] Starting validator
[INFO  consensus] Processing block
Killed  # ← OOM killer terminated the process
```

**Root Cause:** Too many nodes, too much workload traffic, or insufficient Docker resources.

**Fix:**

```bash
# 1. Reduce topology size
# In your scenario:
#   .topology(Topology::preset_3v1e())  # Instead of preset_10v2e()

# 2. Reduce workload rates
#   .workload(TransactionWorkload::new().rate(5.0))  # Instead of rate(100.0)

# 3. Increase Docker resources (Docker Desktop)
# Settings → Resources → Memory: 8GB minimum (12GB+ recommended for large topologies)
# Settings → Resources → CPUs: 4+ cores recommended

# 4. Increase file descriptor limits (Linux/macOS)
ulimit -n 4096

# 5. Close other heavy applications (browsers, IDEs, etc.)
```

---

### 10. Logs Disappear After Run

**Symptoms:**
- Test completes but no logs on disk
- Can't debug failures because logs are gone
- Temporary directories cleaned up automatically

**What you'll see:**

```
$ POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin local_runner
[INFO  runner_examples] Test complete, cleaning up
[INFO  testing_framework_runner_local] Removing temporary directories
$ ls .tmp/
# Empty or missing
```

**Root Cause:** Framework cleans up temporary directories by default to avoid disk bloat.

**Fix:**

```bash
# Persist logs to a specific directory
NOMOS_LOG_DIR=/tmp/test-logs \
NOMOS_TESTS_KEEP_LOGS=1 \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin local_runner

# Logs persist after run
ls /tmp/test-logs/
# nomos-node-0.2024-12-18T14-30-00.log
# nomos-node-1.2024-12-18T14-30-00.log
# ...
```

---

### 11. Consensus Timing Too Tight / Run Duration Too Short

**Symptoms:**
- "Consensus liveness expectation failed"
- Only 1-2 blocks produced (or zero)
- Nodes appear healthy but not making progress

**What you'll see:**

```
$ POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin local_runner
[INFO  testing_framework_core] Starting workloads
[INFO  testing_framework_core] Run window: 10 seconds
[INFO  testing_framework_core] Evaluating expectations
[ERROR testing_framework_core] Consensus liveness expectation failed: expected min 5 blocks, got 1
thread 'main' panicked at 'expectations failed'
```

**Root Cause:** Run duration too short for consensus parameters. If `CONSENSUS_SLOT_TIME=20s` but run duration is only `10s`, you can't produce many blocks.

**Fix:**

```rust
use std::time::Duration;

use testing_framework_core::scenario::ScenarioBuilder;
use testing_framework_workflows::ScenarioBuilderExt;

// Increase run duration to allow more blocks.
let scenario = ScenarioBuilder::topology_with(|t| t.network_star().validators(3).executors(1))
    .expect_consensus_liveness()
    .with_run_duration(Duration::from_secs(120)) // ← Give more time
    .build();
```

**Or adjust consensus timing (if you control node config):**

```bash
# Faster block production (shorter slot time)
CONSENSUS_SLOT_TIME=5 \
CONSENSUS_ACTIVE_SLOT_COEFF=0.9 \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin local_runner
```

---

## Summary: Quick Checklist for Failed Runs

When a test fails, check these in order:

1. ✅ **`POL_PROOF_DEV_MODE=true` is set** (REQUIRED for all runners)
2. ✅ **`versions.env` exists at repo root**
3. ✅ **KZG circuit assets present** (for DA workloads): `testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params`
4. ✅ **Node binaries available** (`NOMOS_NODE_BIN` / `NOMOS_EXECUTOR_BIN` set, or using `run-examples.sh`)
5. ✅ **Docker daemon running** (for compose/k8s)
6. ✅ **Docker image built** (`logos-blockchain-testing:local` exists for compose/k8s)
7. ✅ **No port conflicts** (`lsof -i :18080`, kill orphaned processes)
8. ✅ **Sufficient wallets** (`.wallets(N)` ≥ `.users(M)`)
9. ✅ **Enough resources** (Docker memory 8GB+, ulimit -n 4096)
10. ✅ **Run duration appropriate** (long enough for consensus timing)
11. ✅ **Logs persisted** (`NOMOS_LOG_DIR` + `NOMOS_TESTS_KEEP_LOGS=1` if needed)

**Still stuck?** Check node logs (see [Where to Find Logs](#where-to-find-logs)) for the actual error.

## Where to Find Logs

### Log Location Quick Reference

| Runner | Default Output | With `NOMOS_LOG_DIR` + Flags | Access Command |
|--------|---------------|------------------------------|----------------|
| **Host** (local) | Per-run temporary directories under the current working directory (removed unless `NOMOS_TESTS_KEEP_LOGS=1`) | Per-node files with prefix `nomos-node-{index}` (set `NOMOS_LOG_DIR`) | `cat $NOMOS_LOG_DIR/nomos-node-0*` |
| **Compose** | Docker container stdout/stderr | Set `tracing_settings.logger: !File` in `testing-framework/assets/stack/cfgsync.yaml` (and mount a writable directory) | `docker ps` then `docker logs <container-id>` |
| **K8s** | Pod stdout/stderr | Set `tracing_settings.logger: !File` in `testing-framework/assets/stack/cfgsync.yaml` (and mount a writable directory) | `kubectl logs -l nomos/logical-role=validator` |

**Important Notes:**
- **Host runner** (local processes): Per-run temporary directories are created under the current working directory and removed after the run unless `NOMOS_TESTS_KEEP_LOGS=1`. To write per-node log files to a stable location, set `NOMOS_LOG_DIR=/path/to/logs`.
- **Compose/K8s**: Node log destination is controlled by `testing-framework/assets/stack/cfgsync.yaml` (`tracing_settings.logger`). By default, rely on `docker logs` or `kubectl logs`.
- **File naming**: Log files use prefix `nomos-node-{index}*` or `nomos-executor-{index}*` with timestamps, e.g., `nomos-node-0.2024-12-01T10-30-45.log` (NOT just `.log` suffix).
- **Container names**: Compose containers include project UUID, e.g., `nomos-compose-<uuid>-validator-0-1` where `<uuid>` is randomly generated per run

### Accessing Node Logs by Runner

#### Local Runner

**Console output (default):**
```bash
POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin local_runner 2>&1 | tee test.log
```

**Persistent file output:**
```bash
NOMOS_LOG_DIR=/tmp/debug-logs \
NOMOS_LOG_LEVEL=debug \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin local_runner

# Inspect logs (note: filenames include timestamps):
ls /tmp/debug-logs/
# Example: nomos-node-0.2024-12-01T10-30-45.log
tail -f /tmp/debug-logs/nomos-node-0*  # Use wildcard to match timestamp
```

#### Compose Runner

**Stream live logs:**
```bash
# List running containers (note the UUID prefix in names)
docker ps --filter "name=nomos-compose-"

# Find your container ID or name from the list, then:
docker logs -f <container-id>

# Or filter by name pattern:
docker logs -f $(docker ps --filter "name=nomos-compose-.*-validator-0" -q | head -1)

# Show last 100 lines
docker logs --tail 100 <container-id>
```

**Keep containers for post-mortem debugging:**
```bash
COMPOSE_RUNNER_PRESERVE=1 \
NOMOS_TESTNET_IMAGE=logos-blockchain-testing:local \
POL_PROOF_DEV_MODE=true \
cargo run -p runner-examples --bin compose_runner

# OR: Use run-examples.sh (handles setup automatically)
COMPOSE_RUNNER_PRESERVE=1 scripts/run/run-examples.sh -t 60 -v 1 -e 1 compose

# After test failure, containers remain running:
docker ps --filter "name=nomos-compose-"
docker exec -it <container-id> /bin/sh
docker logs <container-id> > debug.log
```

**Note:** Container names follow the pattern `nomos-compose-{uuid}-validator-{index}-1` or `nomos-compose-{uuid}-executor-{index}-1`, where `{uuid}` is randomly generated per run.

#### K8s Runner

**Important:** Always verify your namespace and use label selectors instead of assuming pod names.

**Stream pod logs (use label selectors):**

```bash
# Check your namespace first
kubectl config view --minify | grep namespace

# All validator pods (add -n <namespace> if not using default)
kubectl logs -l nomos/logical-role=validator -f

# All executor pods
kubectl logs -l nomos/logical-role=executor -f

# Specific pod by name (find exact name first)
kubectl get pods -l nomos/logical-role=validator  # Find the exact pod name
kubectl logs -f <actual-pod-name>        # Then use it

# With explicit namespace
kubectl logs -n my-namespace -l nomos/logical-role=validator -f
```

**Download logs from crashed pods:**

```bash
# Previous logs from crashed pod
kubectl get pods -l nomos/logical-role=validator  # Find crashed pod name first
kubectl logs --previous <actual-pod-name> > crashed-validator.log

# Or use label selector for all crashed validators
for pod in $(kubectl get pods -l nomos/logical-role=validator -o name); do
  kubectl logs --previous $pod > $(basename $pod)-previous.log 2>&1
done
```

**Access logs from all pods:**

```bash
# All pods in current namespace
for pod in $(kubectl get pods -o name); do
  echo "=== $pod ==="
  kubectl logs $pod
done > all-logs.txt

# Or use label selectors (recommended)
kubectl logs -l nomos/logical-role=validator --tail=500 > validators.log
kubectl logs -l nomos/logical-role=executor --tail=500 > executors.log

# With explicit namespace
kubectl logs -n my-namespace -l nomos/logical-role=validator --tail=500 > validators.log
```

## Debugging Workflow

When a test fails, follow this sequence:

### 1. Check Framework Output

Start with the test harness output—did expectations fail? Was there a deployment error?

**Look for:**

- Expectation failure messages
- Timeout errors
- Deployment/readiness failures

### 2. Verify Node Readiness

Ensure all nodes started successfully and became ready before workloads began.

**Commands:**

```bash
# Local: check process list
ps aux | grep nomos

# Compose: check container status (note UUID in names)
docker ps -a --filter "name=nomos-compose-"

# K8s: check pod status (use label selectors, add -n <namespace> if needed)
kubectl get pods -l nomos/logical-role=validator
kubectl get pods -l nomos/logical-role=executor
kubectl describe pod <actual-pod-name>  # Get name from above first
```

### 3. Inspect Node Logs

Focus on the first node that exhibited problems or the node with the highest index (often the last to start).

**Common error patterns:**

- "ERROR: versions.env missing" → missing required `versions.env` file at repository root
- "Failed to bind address" → port conflict
- "Connection refused" → peer not ready or network issue
- "Proof verification failed" or "Proof generation timeout" → missing `POL_PROOF_DEV_MODE=true` (REQUIRED for all runners)
- "Failed to load KZG parameters" or "Circuit file not found" → missing KZG circuit assets at `testing-framework/assets/stack/kzgrs_test_params/`
- "Insufficient funds" → wallet seeding issue (increase `.wallets(N)` or reduce `.users(M)`)

### 4. Check Log Levels

If logs are too sparse, increase verbosity:

```bash
NOMOS_LOG_LEVEL=debug \
NOMOS_LOG_FILTER="cryptarchia=trace,nomos_da_sampling=debug" \
cargo run -p runner-examples --bin local_runner
```

If metric updates are polluting your logs (fields like `counter.*` / `gauge.*`), move those events to a dedicated `tracing` target (e.g. `target: "nomos_metrics"`) and set `NOMOS_LOG_FILTER="nomos_metrics=off,..."` so they don’t get formatted into log output.

### 5. Verify Observability Endpoints

If expectations report observability issues:

**Prometheus (Compose):**
```bash
curl http://localhost:9090/-/healthy
```

**Node HTTP APIs:**
```bash
curl http://localhost:18080/consensus/info  # Adjust port per node
```

### 6. Compare with Known-Good Scenario

Run a minimal baseline test (e.g., 2 validators, consensus liveness only). If it passes, the issue is in your workload or topology configuration.

## Common Error Messages

### "Consensus liveness expectation failed"

- **Cause**: Not enough blocks produced during the run window, missing
  `POL_PROOF_DEV_MODE=true` (causes slow proof generation), or missing KZG
  assets for DA workloads.
- **Fix**:
  1. Verify `POL_PROOF_DEV_MODE=true` is set (REQUIRED for all runners).
  2. Verify KZG assets exist at
     `testing-framework/assets/stack/kzgrs_test_params/` (for DA workloads).
  3. Extend `with_run_duration()` to allow more blocks.
  4. Check node logs for proof generation or DA errors.
  5. Reduce transaction/DA rate if nodes are overwhelmed.

### "Wallet seeding failed"

- **Cause**: Topology doesn't have enough funded wallets for the workload.
- **Fix**: Increase `.wallets(N)` count or reduce `.users(M)` in the transaction
  workload (ensure N ≥ M).

### "Node control not available"

- **Cause**: Runner doesn't support node control (only ComposeDeployer does), or
  `enable_node_control()` wasn't called.
- **Fix**:
  1. Use ComposeDeployer for chaos tests (LocalDeployer and K8sDeployer don't
     support node control).
  2. Ensure `.enable_node_control()` is called in the scenario before `.chaos()`.

### "Readiness timeout"

- **Cause**: Nodes didn't become responsive within expected time (often due to
  missing prerequisites).
- **Fix**:
  1. **Verify `POL_PROOF_DEV_MODE=true` is set** (REQUIRED for all runners—without
     it, proof generation is too slow).
  2. Check node logs for startup errors (port conflicts, missing assets).
  3. Verify network connectivity between nodes.
  4. For DA workloads, ensure KZG circuit assets are present.

### "ERROR: versions.env missing"

- **Cause**: Helper scripts (`run-examples.sh`, `build-bundle.sh`, `setup-circuits-stack.sh`) require `versions.env` file at repository root.
- **Fix**: Ensure you're running from the repository root directory. The `versions.env` file should already exist and contains:
  ```
  VERSION=<circuit release tag>
  NOMOS_NODE_REV=<nomos-node git revision>
  NOMOS_BUNDLE_VERSION=<bundle schema version>
  ```
  Use the checked-in `versions.env` at the repository root as the source of truth.

### "Port already in use"

- **Cause**: Previous test didn't clean up, or another process holds the port.
- **Fix**: Kill orphaned processes (`pkill nomos-node`), wait for Docker cleanup
  (`docker compose down`), or restart Docker.

### "Image not found: logos-blockchain-testing:local"

- **Cause**: Docker image not built for Compose/K8s runners, or KZG assets not
  baked into the image.
- **Fix (recommended)**: Use run-examples.sh which handles everything:
  ```bash
  scripts/run/run-examples.sh -t 60 -v 1 -e 1 compose
  ```
- **Fix (manual)**:
  1. Build bundle: `scripts/build/build-bundle.sh --platform linux`
  2. Set bundle path: `export NOMOS_BINARIES_TAR=.tmp/nomos-binaries-linux-v0.3.1.tar.gz`
  3. Build image: `scripts/build/build_test_image.sh`
  4. **kind/minikube:** load the image into the cluster nodes (e.g. `kind load docker-image logos-blockchain-testing:local`, or `minikube image load ...`), or push to a registry and set `NOMOS_TESTNET_IMAGE` accordingly.

### "Failed to load KZG parameters" or "Circuit file not found"

- **Cause**: DA workload requires KZG circuit assets. The file `testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params` (note repeated filename) must exist. Inside containers, it's at `/kzgrs_test_params/kzgrs_test_params`.
- **Fix (recommended)**: Use run-examples.sh which handles setup:
  ```bash
  scripts/run/run-examples.sh -t 60 -v 1 -e 1 <mode>
  ```
- **Fix (manual)**:
  1. Fetch assets: `scripts/setup/setup-nomos-circuits.sh v0.3.1 /tmp/nomos-circuits`
  2. Copy to expected path: `cp -r /tmp/nomos-circuits/* testing-framework/assets/stack/kzgrs_test_params/`
  3. Verify file exists: `ls -lh testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params`
  4. For Compose/K8s: rebuild image with assets baked in

For detailed logging configuration and observability setup, see [Logging & Observability](logging-observability.md).

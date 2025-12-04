# Troubleshooting Scenarios

**Prerequisites for All Runners:**
- **`POL_PROOF_DEV_MODE=true`** MUST be set for all runners (local, compose, k8s) to avoid expensive Groth16 proof generation that causes timeouts
- **KZG circuit assets** must be present at `testing-framework/assets/stack/kzgrs_test_params/` for DA workloads (fetch via `scripts/setup-nomos-circuits.sh`)

## Quick Symptom Guide

Common symptoms and likely causes:

- **No or slow block progression**: missing `POL_PROOF_DEV_MODE=true`, missing KZG circuit assets for DA workloads, too-short run window, port conflicts, or resource exhaustion—set required env vars, verify assets, extend duration, check node logs for startup errors.
- **Transactions not included**: unfunded or misconfigured wallets (check `.wallets(N)` vs `.users(M)`), transaction rate exceeding block capacity, or rates exceeding block production speed—reduce rate, increase wallet count, verify wallet setup in logs.
- **Chaos stalls the run**: chaos (node control) only works with ComposeDeployer; LocalDeployer and K8sDeployer don't support it (won't "stall", just can't execute chaos workloads). With compose, aggressive restart cadence can prevent consensus recovery—widen restart intervals.
- **Observability gaps**: metrics or logs unreachable because ports clash or services are not exposed—adjust observability ports and confirm runner wiring.
- **Flaky behavior across runs**: mixing chaos with functional smoke tests or inconsistent topology between environments—separate deterministic and chaos scenarios and standardize topology presets.

## Where to Find Logs

### Log Location Quick Reference

| Runner | Default Output | With `NOMOS_LOG_DIR` + Flags | Access Command |
|--------|---------------|------------------------------|----------------|
| **Local** | Temporary directories (cleaned up) | Per-node files with prefix `nomos-node-{index}` (requires `NOMOS_TESTS_TRACING=true`) | `cat $NOMOS_LOG_DIR/nomos-node-0*` |
| **Compose** | Docker container stdout/stderr | Per-node files inside containers (if path is mounted) | `docker ps` then `docker logs <container-id>` |
| **K8s** | Pod stdout/stderr | Per-node files inside pods (if path is mounted) | `kubectl logs -l app=nomos-validator` |

**Important Notes:**
- **Local runner**: Logs go to system temporary directories (NOT in working directory) by default and are automatically cleaned up after tests. To persist logs, you MUST set both `NOMOS_TESTS_TRACING=true` AND `NOMOS_LOG_DIR=/path/to/logs`.
- **Compose/K8s**: Per-node log files only exist inside containers/pods if `NOMOS_LOG_DIR` is set AND the path is writable inside the container/pod. By default, rely on `docker logs` or `kubectl logs`.
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
NOMOS_TESTS_TRACING=true \
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
NOMOS_TESTNET_IMAGE=nomos-testnet:local \
cargo run -p runner-examples --bin compose_runner

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
kubectl logs -l app=nomos-validator -f

# All executor pods
kubectl logs -l app=nomos-executor -f

# Specific pod by name (find exact name first)
kubectl get pods -l app=nomos-validator  # Find the exact pod name
kubectl logs -f <actual-pod-name>        # Then use it

# With explicit namespace
kubectl logs -n my-namespace -l app=nomos-validator -f
```

**Download logs from crashed pods:**

```bash
# Previous logs from crashed pod
kubectl get pods -l app=nomos-validator  # Find crashed pod name first
kubectl logs --previous <actual-pod-name> > crashed-validator.log

# Or use label selector for all crashed validators
for pod in $(kubectl get pods -l app=nomos-validator -o name); do
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
kubectl logs -l app=nomos-validator --tail=500 > validators.log
kubectl logs -l app=nomos-executor --tail=500 > executors.log

# With explicit namespace
kubectl logs -n my-namespace -l app=nomos-validator --tail=500 > validators.log
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
kubectl get pods -l app=nomos-validator
kubectl get pods -l app=nomos-executor
kubectl describe pod <actual-pod-name>  # Get name from above first
```

### 3. Inspect Node Logs

Focus on the first node that exhibited problems or the node with the highest index (often the last to start).

**Common error patterns:**

- "Failed to bind address" → port conflict
- "Connection refused" → peer not ready or network issue
- "Proof verification failed" or "Proof generation timeout" → missing `POL_PROOF_DEV_MODE=true` (REQUIRED for all runners)
- "Failed to load KZG parameters" or "Circuit file not found" → missing KZG circuit assets at `testing-framework/assets/stack/kzgrs_test_params/`
- "Insufficient funds" → wallet seeding issue (increase `.wallets(N)` or reduce `.users(M)`)

### 4. Check Log Levels

If logs are too sparse, increase verbosity:

```bash
NOMOS_LOG_LEVEL=debug \
NOMOS_LOG_FILTER="nomos_consensus=trace,nomos_da_sampling=debug" \
cargo run -p runner-examples --bin local_runner
```

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

### "Port already in use"

- **Cause**: Previous test didn't clean up, or another process holds the port.
- **Fix**: Kill orphaned processes (`pkill nomos-node`), wait for Docker cleanup
  (`docker compose down`), or restart Docker.

### "Image not found: nomos-testnet:local"

- **Cause**: Docker image not built for Compose/K8s runners, or KZG assets not
  baked into the image.
- **Fix**:
  1. Fetch KZG assets: `scripts/setup-nomos-circuits.sh v0.3.1 /tmp/nomos-circuits`.
  2. Copy to assets:
     `cp -r /tmp/nomos-circuits/* testing-framework/assets/stack/kzgrs_test_params/`.
  3. Build image: `testing-framework/assets/stack/scripts/build_test_image.sh`.

### "Failed to load KZG parameters" or "Circuit file not found"

- **Cause**: DA workload requires KZG circuit assets that aren't present.
- **Fix**:
  1. Fetch assets: `scripts/setup-nomos-circuits.sh v0.3.1 /tmp/nomos-circuits`.
  2. Copy to expected path:
     `cp -r /tmp/nomos-circuits/* testing-framework/assets/stack/kzgrs_test_params/`.
  3. For Compose/K8s: rebuild image with assets baked in.

For detailed logging configuration and observability setup, see [Operations](operations.md).

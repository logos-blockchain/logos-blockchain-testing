# Prerequisites & Setup

This page covers everything you need before running your first scenario.

## Required Files

### `versions.env` (Required)

All helper scripts require a `versions.env` file at the repository root:

```bash
VERSION=v0.3.1
NOMOS_NODE_REV=abc123def456789
NOMOS_BUNDLE_VERSION=v1
```

**What it defines:**
- `VERSION` — Circuit release tag for KZG parameters
- `NOMOS_NODE_REV` — Git revision of nomos-node to build/fetch
- `NOMOS_BUNDLE_VERSION` — Bundle schema version

**Where it's used:**
- `scripts/run/run-examples.sh`
- `scripts/build/build-bundle.sh`
- `scripts/setup/setup-nomos-circuits.sh`
- CI workflows

**Error if missing:**
```text
ERROR: versions.env not found at repository root
This file is required and should define:
  VERSION=<circuit release tag>
  NOMOS_NODE_REV=<nomos-node git revision>
  NOMOS_BUNDLE_VERSION=<bundle schema version>
```

**Fix:** Ensure you're in the repository root. The file should already exist in the checked-out repo.

## Node Binaries

Scenarios need compiled `nomos-node` binaries.

### Option 1: Use Helper Scripts (Recommended)

```bash
scripts/run/run-examples.sh -t 60 -v 3 -e 1 host
```

This automatically:
- Clones/updates nomos-node checkout
- Builds required binaries
- Sets `NOMOS_NODE_BIN` 

### Option 2: Manual Build

If you have a sibling `nomos-node` checkout:

```bash
cd ../nomos-node
cargo build --release --bin nomos-node 

# Set environment variables
export NOMOS_NODE_BIN=$PWD/target/release/nomos-node

# Return to testing framework
cd ../nomos-testing
```

### Option 3: Prebuilt Bundles (CI)

CI workflows use prebuilt artifacts:

```yaml
- name: Download nomos binaries
  uses: actions/download-artifact@v3
  with:
    name: nomos-binaries-linux
    path: .tmp/

- name: Extract bundle
  run: |
    tar -xzf .tmp/nomos-binaries-linux-*.tar.gz -C .tmp/
    export NOMOS_NODE_BIN=$PWD/.tmp/nomos-node
```

## Circuit Assets (KZG Parameters)

Data Availability (DA) workloads require KZG cryptographic parameters.

### Asset Location

**Default path:** `testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params`

Note: The directory `kzgrs_test_params/` contains a file named `kzgrs_test_params`. This is the proving key file (~120MB).

**Container path (compose/k8s):** `/kzgrs_test_params/kzgrs_test_params`

### Getting Assets

**Option 1: Use helper script** (recommended):

```bash
# Fetch circuits
scripts/setup/setup-nomos-circuits.sh v0.3.1 /tmp/nomos-circuits

# Copy to default location
mkdir -p testing-framework/assets/stack/kzgrs_test_params
cp -r /tmp/nomos-circuits/* testing-framework/assets/stack/kzgrs_test_params/

# Verify (should be ~120MB)
ls -lh testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params
```

**Option 2: Let `run-examples.sh` handle it**:

```bash
scripts/run/run-examples.sh -t 60 -v 3 -e 1 host
```

This automatically fetches and places assets.

### Override Path

Set `NOMOS_KZGRS_PARAMS_PATH` to use a custom location:

```bash
NOMOS_KZGRS_PARAMS_PATH=/custom/path/to/kzgrs_test_params \
cargo run -p runner-examples --bin local_runner
```

### When Are Assets Needed?

| Runner | When Required |
|--------|---------------|
| **Host (local)** | Always (for DA workloads) |
| **Compose** | During image build (baked into image) |
| **K8s** | During image build + mounted via hostPath |

**Error without assets:**

```text
Error: Custom { kind: NotFound, error: "Circuit file not found at: testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params" }
```

## Platform Requirements

### Host Runner (Local Processes)

**Requires:**
- Rust nightly toolchain
- Node binaries built
- KZG circuit assets (for DA workloads)
- Available ports (18080+, 3100+, etc.)

**No Docker required.**

**Best for:**
- Quick iteration
- Development
- Smoke tests

### Compose Runner (Docker Compose)

**Requires:**
- Docker daemon running
- Docker image built: `logos-blockchain-testing:local`
- KZG assets baked into image
- Docker Desktop (macOS) or Docker Engine (Linux)

**Platform notes (macOS / Apple silicon):**
- Prefer `NOMOS_BUNDLE_DOCKER_PLATFORM=linux/arm64` for native performance
- Use `linux/amd64` only if targeting amd64 environments (slower via emulation)

**Best for:**
- Reproducible environments
- CI testing
- Chaos workloads (node control support)

### K8s Runner (Kubernetes)

**Requires:**
- Kubernetes cluster (Docker Desktop K8s, minikube, kind, or remote)
- `kubectl` configured
- Docker image built and loaded/pushed
- KZG assets baked into image + mounted via hostPath

**Local cluster setup:**

```bash
# Docker Desktop: Enable Kubernetes in settings

# OR: Use kind
kind create cluster
kind load docker-image logos-blockchain-testing:local

# OR: Use minikube
minikube start
minikube image load logos-blockchain-testing:local
```

**Remote cluster:** Push image to registry and set `NOMOS_TESTNET_IMAGE`.

**Best for:**
- Production-like testing
- Resource isolation
- Large topologies

## Critical Environment Variable

**`POL_PROOF_DEV_MODE=true` is REQUIRED for ALL runners!**

Without this, proof generation uses expensive Groth16 proving, causing:
- Tests "hang" for minutes
- CPU spikes to 100%
- Timeouts and failures

**Always set:**

```bash
POL_PROOF_DEV_MODE=true cargo run -p runner-examples --bin local_runner
POL_PROOF_DEV_MODE=true scripts/run/run-examples.sh -t 60 -v 3 -e 1 compose
# etc.
```

**Or add to your shell profile:**

```bash
# ~/.bashrc or ~/.zshrc
export POL_PROOF_DEV_MODE=true
```

## Quick Setup Check

Run this checklist before your first scenario:

```bash
# 1. Verify versions.env exists
cat versions.env

# 2. Check circuit assets (for DA workloads)
ls -lh testing-framework/assets/stack/kzgrs_test_params/kzgrs_test_params

# 3. Verify POL_PROOF_DEV_MODE is set
echo $POL_PROOF_DEV_MODE  # Should print: true

# 4. For compose/k8s: verify Docker is running
docker ps

# 5. For compose/k8s: verify image exists
docker images | grep logos-blockchain-testing

# 6. For host runner: verify node binaries (if not using scripts)
$NOMOS_NODE_BIN --version
```

## Recommended: Use Helper Scripts

The easiest path is to let the helper scripts handle everything:

```bash
# Host runner
scripts/run/run-examples.sh -t 60 -v 3 -e 1 host

# Compose runner
scripts/run/run-examples.sh -t 60 -v 3 -e 1 compose

# K8s runner
scripts/run/run-examples.sh -t 60 -v 3 -e 1 k8s
```

These scripts:
- Verify `versions.env` exists
- Clone/build nomos-node if needed
- Fetch circuit assets if missing
- Build Docker images (compose/k8s)
- Load images into cluster (k8s)
- Run the scenario with proper environment

**Next Steps:**
- [Running Examples](running-examples.md) — Learn how to run scenarios
- [Environment Variables](environment-variables.md) — Full variable reference
- [Troubleshooting](troubleshooting.md) — Common issues and fixes

# Prerequisites & Setup

This page covers everything you need before running your first scenario.

## Required Files

### `versions.env` (Required)

All helper scripts require a `versions.env` file at the repository root:

```bash
VERSION=v0.3.1
LOGOS_BLOCKCHAIN_NODE_REV=abc123def456789
LOGOS_BLOCKCHAIN_BUNDLE_VERSION=v1
```

**What it defines:**
- `VERSION` — Circuit assets release tag
- `LOGOS_BLOCKCHAIN_NODE_REV` — Git revision of logos-blockchain-node to build/fetch
- `LOGOS_BLOCKCHAIN_BUNDLE_VERSION` — Bundle schema version

**Where it's used:**
- `scripts/run/run-examples.sh`
- `scripts/build/build-bundle.sh`
- `scripts/setup/setup-logos-blockchain-circuits.sh`
- CI workflows

**Error if missing:**
```text
ERROR: versions.env not found at repository root
This file is required and should define:
  VERSION=<circuit release tag>
  LOGOS_BLOCKCHAIN_NODE_REV=<logos-blockchain-node git revision>
  LOGOS_BLOCKCHAIN_BUNDLE_VERSION=<bundle schema version>
```

**Fix:** Ensure you're in the repository root. The file should already exist in the checked-out repo.

## Node Binaries

Scenarios need compiled `logos-blockchain-node` binaries.

### Option 1: Use Helper Scripts (Recommended)

```bash
scripts/run/run-examples.sh -t 60 -n 3 host
```

This automatically:
- Clones/updates logos-blockchain-node checkout
- Builds required binaries
- Sets `LOGOS_BLOCKCHAIN_NODE_BIN` 

### Option 2: Manual Build

If you have a sibling `logos-blockchain-node` checkout:

```bash
cd ../logos-blockchain-node
cargo build --release --bin logos-blockchain-node 

# Set environment variables
export LOGOS_BLOCKCHAIN_NODE_BIN=$PWD/target/release/logos-blockchain-node

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
    export LOGOS_BLOCKCHAIN_NODE_BIN=$PWD/.tmp/logos-blockchain-node
```

## Circuit Assets

Nodes require circuit assets for proof generation. The framework expects a
directory containing the circuits, not a single file.

### Asset Location

**Default path:** `~/.logos-blockchain-circuits`

**Container path (compose/k8s):** `/opt/circuits` (set during image build)

### Getting Assets

**Option 1: Use helper script** (recommended):

```bash
scripts/setup/setup-logos-blockchain-circuits.sh v0.3.1 ~/.logos-blockchain-circuits
```

**Option 2: Let `run-examples.sh` handle it**:

```bash
scripts/run/run-examples.sh -t 60 -n 3 host
```

### Override Path

Set `LOGOS_BLOCKCHAIN_CIRCUITS` to use a custom location:

```bash
LOGOS_BLOCKCHAIN_CIRCUITS=/custom/path/to/circuits \
cargo run -p runner-examples --bin local_runner
```

### When Are Assets Needed?

| Runner | When Required |
|--------|---------------|
| **Host (local)** | Always |
| **Compose** | During image build (baked into image) |
| **K8s** | During image build |

**Error without assets:**

```text
Error: circuits directory not found (LOGOS_BLOCKCHAIN_CIRCUITS)
```

## Platform Requirements

### Host Runner (Local Processes)

**Requires:**
- Rust nightly toolchain
- Node binaries built
- Circuit assets for proof generation
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
- Circuit assets baked into image
- Docker Desktop (macOS) or Docker Engine (Linux)

**Platform notes (macOS / Apple silicon):**
- Prefer `LOGOS_BLOCKCHAIN_BUNDLE_DOCKER_PLATFORM=linux/arm64` for native performance
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
- Circuit assets baked into image

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

**Remote cluster:** Push image to registry and set `LOGOS_BLOCKCHAIN_TESTNET_IMAGE`.

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
POL_PROOF_DEV_MODE=true scripts/run/run-examples.sh -t 60 -n 3 compose
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

# 2. Check circuit assets
ls -lh "${HOME}/.logos-blockchain-circuits"

# 3. Verify POL_PROOF_DEV_MODE is set
echo $POL_PROOF_DEV_MODE  # Should print: true

# 4. For compose/k8s: verify Docker is running
docker ps

# 5. For compose/k8s: verify image exists
docker images | grep logos-blockchain-testing

# 6. For host runner: verify node binaries (if not using scripts)
$LOGOS_BLOCKCHAIN_NODE_BIN --version
```

## Recommended: Use Helper Scripts

The easiest path is to let the helper scripts handle everything:

```bash
# Host runner
scripts/run/run-examples.sh -t 60 -n 3 host

# Compose runner
scripts/run/run-examples.sh -t 60 -n 3 compose

# K8s runner
scripts/run/run-examples.sh -t 60 -n 3 k8s
```

These scripts:
- Verify `versions.env` exists
- Clone/build logos-blockchain-node if needed
- Fetch circuit assets if missing
- Build Docker images (compose/k8s)
- Load images into cluster (k8s)
- Run the scenario with proper environment

**Next Steps:**
- [Running Examples](running-examples.md) — Learn how to run scenarios
- [Environment Variables](environment-variables.md) — Full variable reference
- [Troubleshooting](troubleshooting.md) — Common issues and fixes

# Logos Blockchain Testing Framework

A comprehensive testing framework for the Logos blockchain implementation, providing declarative scenario definitions, multiple deployment backends, and production-grade observability.

## Overview

This framework enables you to define, deploy, and execute integration tests for Logos blockchain scenarios across different environments—from local processes to containerized Kubernetes deployments—using a unified API.

**Key capabilities:**
- **Declarative scenario model** — Define topology, workloads, and success criteria using a fluent builder API
- **Multiple deployment backends** — Local processes, Docker Compose, or Kubernetes
- **Built-in workloads** — Transaction injection, DA (Data Availability) traffic, and chaos engineering
- **Observability-first** — Integrated Prometheus metrics, structured logging, and OpenTelemetry support
- **Production-ready** — Used in CI/CD pipelines with reproducible containerized environments

## Quick Start

### Prerequisites

- Rust toolchain (nightly)
- `versions.env` file at repository root (included)
- For Docker Compose: Docker daemon
- For Kubernetes: Cluster access and `kubectl`

### Run Your First Test

```bash
# Host mode (local processes) - fastest iteration
scripts/run-examples.sh -t 60 -v 1 -e 1 host

# Compose mode (Docker containers) - reproducible environment
scripts/run-examples.sh -t 60 -v 1 -e 1 compose

# K8s mode (Kubernetes cluster) - production-like fidelity
scripts/run-examples.sh -t 60 -v 1 -e 1 k8s
```

The script handles circuit setup, binary building, image preparation, and scenario execution automatically.

## Documentation

**Complete documentation available at:** https://logos-blockchain.github.io/logos-blockchain-testing/

### Essential Guides

| Topic | Link |
|-------|------|
| **Getting Started** | [Quickstart Guide](https://logos-blockchain.github.io/logos-blockchain-testing/quickstart.html) |
| **Core Concepts** | [Testing Philosophy](https://logos-blockchain.github.io/logos-blockchain-testing/testing-philosophy.html) |
| **Examples** | [Basic](https://logos-blockchain.github.io/logos-blockchain-testing/examples.html) \| [Advanced](https://logos-blockchain.github.io/logos-blockchain-testing/examples-advanced.html) |
| **Deployment Options** | [Runners Overview](https://logos-blockchain.github.io/logos-blockchain-testing/runners.html) |
| **API Reference** | [Builder API](https://logos-blockchain.github.io/logos-blockchain-testing/dsl-cheat-sheet.html) |
| **Operations** | [Setup & Configuration](https://logos-blockchain.github.io/logos-blockchain-testing/operations.html) |
| **Troubleshooting** | [Common Issues](https://logos-blockchain.github.io/logos-blockchain-testing/troubleshooting.html) |

## Repository Structure

```
logos-blockchain-testing/
├── testing-framework/     # Core library crates
│   ├── core/             # Scenario model, runtime orchestration
│   ├── workflows/        # Workloads (tx, DA, chaos) and expectations
│   ├── configs/          # Node configuration builders
│   ├── runners/          # Deployment backends (local, compose, k8s)
│   └── assets/stack/     # Docker/K8s deployment assets
├── examples/             # Runnable demo binaries
│   └── src/bin/          # local_runner, compose_runner, k8s_runner
├── scripts/              # Helper utilities (run-examples.sh, build-bundle.sh)
└── book/                 # Documentation sources (mdBook)
```

## Architecture

The framework follows a clear separation of concerns:

**Scenario Definition** → **Topology Builder** → **Deployer** → **Runner** → **Workloads** → **Expectations**

- **Scenario**: Declarative description of test intent (topology + workloads + success criteria)
- **Deployer**: Provisions infrastructure on chosen backend (host/compose/k8s)
- **Runner**: Orchestrates execution, manages lifecycle, collects observability
- **Workloads**: Generate traffic and conditions (transactions, DA blobs, chaos)
- **Expectations**: Evaluate success/failure based on observed behavior

## Development

### Building the Documentation

```bash
# Install mdBook
cargo install mdbook mdbook-mermaid

# Build and serve locally
cd book && mdbook serve
# Open http://localhost:3000
```

### Running Tests

```bash
# Run framework unit tests
cargo test

# Run integration examples
scripts/run-examples.sh -t 60 -v 2 -e 1 host
```

### Creating Prebuilt Bundles

For compose/k8s deployments, you can create prebuilt bundles to speed up image builds:

```bash
# Build Linux bundle (required for compose/k8s)
scripts/build-bundle.sh --platform linux

# Use the bundle when building images
export NOMOS_BINARIES_TAR=.tmp/nomos-binaries-linux-v0.3.1.tar.gz
testing-framework/assets/stack/scripts/build_test_image.sh
```

## Environment Variables

Key environment variables for customization:

| Variable | Purpose | Default |
|----------|---------|---------|
| `POL_PROOF_DEV_MODE=true` | **Required** — Disable expensive proof generation | (none) |
| `NOMOS_TESTNET_IMAGE` | Docker image tag for compose/k8s | `logos-blockchain-testing:local` |
| `NOMOS_DEMO_VALIDATORS` | Number of validator nodes | Varies by example |
| `NOMOS_DEMO_EXECUTORS` | Number of executor nodes | Varies by example |
| `NOMOS_LOG_DIR` | Directory for persistent log files | (temporary) |
| `NOMOS_LOG_LEVEL` | Logging verbosity | `info` |

See [Operations Guide](https://logos-blockchain.github.io/logos-blockchain-testing/operations.html) for complete configuration reference.

## CI/CD Integration

The framework is designed for CI/CD pipelines:

- **Host runner**: Fast smoke tests with minimal overhead
- **Compose runner**: Reproducible containerized environment with Prometheus
- **K8s runner**: Production-like cluster validation

Example CI workflow: `.github/workflows/lint.yml` (see `compose_smoke` job)

## License

This project is part of the Logos blockchain implementation.

## Links

- **Documentation**: https://logos-blockchain.github.io/logos-blockchain-testing/
- **Logos Project**: https://github.com/logos-co
- **Nomos Node**: https://github.com/logos-co/nomos-node

## Support

For issues, questions, or contributions, please refer to the [Troubleshooting Guide](https://logos-blockchain.github.io/logos-blockchain-testing/troubleshooting.html) or file an issue in this repository.

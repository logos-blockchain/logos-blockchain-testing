# Runners

Runners turn a scenario plan into a live environment while keeping the plan
unchanged. Choose based on feedback speed, reproducibility, and fidelity. For
environment and operational considerations, see [Operations Overview](operations-overview.md).

**Important:** All runners require `POL_PROOF_DEV_MODE=true` to avoid expensive Groth16 proof generation that causes timeouts.

## Host runner (local processes)
- Launches node processes directly on the host (via `LocalDeployer`).
- Binary: `local_runner.rs`, script mode: `host`
- Fastest feedback loop and minimal orchestration overhead.
- Best for development-time iteration and debugging.
- **Can run in CI** for fast smoke tests.
- **Node control:** Not supported (chaos workloads not available)

**Run with:** `scripts/run/run-examples.sh -t 60 -n 1 host`

## Docker Compose runner
- Starts nodes in containers to provide a reproducible multi-node stack on a
  single machine (via `ComposeDeployer`).
- Binary: `compose_runner.rs`, script mode: `compose`
- Discovers service ports and wires observability for convenient inspection.
- Good balance between fidelity and ease of setup.
- **Recommended for CI pipelines** (isolated environment, reproducible).
- **Node control:** Supported (can restart nodes for chaos testing)

**Run with:** `scripts/run/run-examples.sh -t 60 -n 1 compose`

## Kubernetes runner
- Deploys nodes onto a cluster for higher-fidelity, longer-running scenarios (via `K8sDeployer`).
- Binary: `k8s_runner.rs`, script mode: `k8s`
- Suits CI with cluster access or shared test environments where cluster behavior
  and scheduling matter.
- **Node control:** Not supported yet (chaos workloads not available)

**Run with:** `scripts/run/run-examples.sh -t 60 -n 1 k8s`

### Common expectations
- All runners require at least one node and, for transaction scenarios,
  access to seeded wallets.
- Readiness probes gate workload start so traffic begins only after nodes are
  reachable.
- Environment flags can relax timeouts or increase tracing when diagnostics are
  needed.

## Runner Comparison

```mermaid
flowchart TB
    subgraph Host["Host Runner (Local)"]
        H1["Speed: Fast"]
        H2["Isolation: Shared host"]
        H3["Setup: Minimal"]
        H4["Chaos: Not supported"]
        H5["CI: Quick smoke tests"]
    end
    
    subgraph Compose["Compose Runner (Docker)"]
        C1["Speed: Medium"]
        C2["Isolation: Containerized"]
        C3["Setup: Image build required"]
        C4["Chaos: Supported"]
        C5["CI: Recommended"]
    end
    
    subgraph K8s["K8s Runner (Cluster)"]
        K1["Speed: Slower"]
        K2["Isolation: Pod-level"]
        K3["Setup: Cluster + image"]
        K4["Chaos: Not yet supported"]
        K5["CI: Large-scale tests"]
    end
    
    Decision{Choose Based On}
    Decision -->|Fast iteration| Host
    Decision -->|Reproducibility| Compose
    Decision -->|Production-like| K8s
    
    style Host fill:#e1f5ff
    style Compose fill:#e1ffe1
    style K8s fill:#ffe1f5
```

## Detailed Feature Matrix

| Feature | Host | Compose | K8s |
|---------|------|---------|-----|
| **Speed** | Fastest | Medium | Slowest |
| **Setup Time** | < 1 min | 2-5 min | 5-10 min |
| **Isolation** | Process-level | Container | Pod + namespace |
| **Node Control** | No | Yes | Not yet |
| **Observability** | Basic | External stack | Cluster-wide |
| **CI Integration** | Smoke tests | Recommended | Heavy tests |
| **Resource Usage** | Low | Medium | High |
| **Reproducibility** | Environment-dependent | High | Highest |
| **Network Fidelity** | Localhost only | Virtual network | Real cluster |
| **Parallel Runs** | Port conflicts | Isolated | Namespace isolation |

## Decision Guide

```mermaid
flowchart TD
    Start[Need to run tests?] --> Q1{Local development?}
    Q1 -->|Yes| Q2{Testing chaos?}
    Q1 -->|No| Q5{Have cluster access?}
    
    Q2 -->|Yes| UseCompose[Use Compose]
    Q2 -->|No| Q3{Need isolation?}
    
    Q3 -->|Yes| UseCompose
    Q3 -->|No| UseHost[Use Host]
    
    Q5 -->|Yes| Q6{Large topology?}
    Q5 -->|No| Q7{CI pipeline?}
    
    Q6 -->|Yes| UseK8s[Use K8s]
    Q6 -->|No| UseCompose
    
    Q7 -->|Yes| Q8{Docker available?}
    Q7 -->|No| UseHost
    
    Q8 -->|Yes| UseCompose
    Q8 -->|No| UseHost
    
    style UseHost fill:#e1f5ff
    style UseCompose fill:#e1ffe1
    style UseK8s fill:#ffe1f5
```

### Quick Recommendations

**Use Host Runner when:**
- Iterating rapidly during development
- Running quick smoke tests
- Testing on a laptop with limited resources
- Don't need chaos testing

**Use Compose Runner when:**
- Need reproducible test environments
- Testing chaos scenarios (node restarts)
- Running in CI pipelines
- Want containerized isolation

**Use K8s Runner when:**
- Testing large-scale topologies (10+ nodes)
- Need production-like environment
- Have cluster access in CI
- Testing cluster-specific behaviors

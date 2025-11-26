# Runners

Runners turn a scenario plan into a live environment while keeping the plan
unchanged. Choose based on feedback speed, reproducibility, and fidelity. For
environment and operational considerations, see [Operations](operations.md):

## Local runner
- Launches node processes directly on the host.
- Fastest feedback loop and minimal orchestration overhead.
- Best for development-time iteration and debugging.

## Docker Compose runner
- Starts nodes in containers to provide a reproducible multi-node stack on a
  single machine.
- Discovers service ports and wires observability for convenient inspection.
- Good balance between fidelity and ease of setup.

## Kubernetes runner
- Deploys nodes onto a cluster for higher-fidelity, longer-running scenarios.
- Suits CI or shared environments where cluster behavior and scheduling matter.

### Common expectations
- All runners require at least one validator and, for transaction scenarios,
  access to seeded wallets.
- Readiness probes gate workload start so traffic begins only after nodes are
  reachable.
- Environment flags can relax timeouts or increase tracing when diagnostics are
  needed.

Runner abstraction:
```
Scenario Plan
    │
    ▼
Runner (local | compose | k8s)
    │  provisions env + readiness
    ▼
Runtime + Observability
    │
    ▼
Workloads / Expectations execute
```

Mermaid view:
```mermaid
flowchart TD
    Plan[Scenario Plan] --> RunSel{Runner<br/>(local | compose | k8s)}
    RunSel --> Provision[Provision & readiness]
    Provision --> Runtime[Runtime + observability]
    Runtime --> Exec[Workloads & Expectations execute]
```

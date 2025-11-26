# Core Content: Workloads & Expectations

Workloads describe the activity a scenario generates; expectations describe the
signals that must hold when that activity completes. Both are pluggable so
scenarios stay readable and purpose-driven.

## Workloads
- **Transaction workload**: submits user-level transactions at a configurable
  rate and can limit how many distinct actors participate.
- **Data-availability workload**: drives blob and channel activity to exercise
  data-availability paths.
- **Chaos workload**: triggers controlled node restarts to test resilience and
  recovery behaviors (requires a runner that can control nodes).

## Expectations
- **Consensus liveness**: verifies the system continues to produce blocks in
  line with the planned workload and timing window.
- **Workload-specific checks**: each workload can attach its own success
  criteria (e.g., inclusion of submitted activity) so scenarios remain concise.

Together, workloads and expectations let you express both the pressure applied
to the system and the definition of “healthy” for that run.

Workload pipeline (conceptual):
```
Inputs (topology + wallets + rates)
    │
    ▼
Workload init → Drive traffic → Collect signals
                                   │
                                   ▼
                           Expectations evaluate
```

Mermaid view:
```mermaid
flowchart TD
    I[Inputs<br/>(topology + wallets + rates)] --> Init[Workload init]
    Init --> Drive[Drive traffic]
    Drive --> Collect[Collect signals]
    Collect --> Eval[Expectations evaluate]
```

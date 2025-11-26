# Scenario Lifecycle (Conceptual)

1. **Build the plan**: Declare a topology, attach workloads and expectations, and set the run window. The plan is the single source of truth for what will happen.
2. **Deploy**: Hand the plan to a runner. It provisions the environment on the chosen backend and waits for nodes to signal readiness.
3. **Drive workloads**: Start traffic and behaviors (transactions, data-availability activity, restarts) for the planned duration.
4. **Observe blocks and signals**: Track block progression and other high-level metrics during or after the run window to ground assertions in protocol time.
5. **Evaluate expectations**: Once activity stops (and optional cooldown completes), check liveness and workload-specific outcomes to decide pass or fail.
6. **Cleanup**: Tear down resources so successive runs start fresh and do not inherit leaked state.

Conceptual lifecycle diagram:
```
Plan → Deploy → Readiness → Drive Workloads → Observe → Evaluate → Cleanup
```

Mermaid view:
```mermaid
flowchart LR
    P[Plan<br/>topology + workloads + expectations] --> D[Deploy<br/>runner provisions]
    D --> R[Readiness<br/>wait for nodes]
    R --> W[Drive Workloads]
    W --> O[Observe<br/>blocks/metrics]
    O --> E[Evaluate Expectations]
    E --> C[Cleanup]
```

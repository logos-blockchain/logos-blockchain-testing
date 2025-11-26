# Extending the Framework

## Adding a workload
1) Implement `testing_framework_core::scenario::Workload`:
   - Provide a name and any bundled expectations.
   - In `init`, derive inputs from `GeneratedTopology` and `RunMetrics`; fail
     fast if prerequisites are missing (e.g., wallet data, node addresses).
   - In `start`, drive async traffic using the `RunContext` clients.
2) Expose the workload from a module under `testing-framework/workflows` and
   consider adding a DSL helper for ergonomic wiring.

## Adding an expectation
1) Implement `testing_framework_core::scenario::Expectation`:
   - Use `start_capture` to snapshot baseline metrics.
   - Use `evaluate` to assert outcomes after workloads finish; return all errors
     so the runner can aggregate them.
2) Export it from `testing-framework/workflows` if it is reusable.

## Adding a runner
1) Implement `testing_framework_core::scenario::Deployer` for your backend.
   - Produce a `RunContext` with `NodeClients`, metrics endpoints, and optional
     `NodeControlHandle`.
   - Guard cleanup with `CleanupGuard` to reclaim resources even on failures.
2) Mirror the readiness and block-feed probes used by the existing runners so
   workloads can rely on consistent signals.

## Adding topology helpers
- Extend `testing_framework_core::topology::TopologyBuilder` with new layouts or
  configuration presets (e.g., specialized DA parameters). Keep defaults safe:
  ensure at least one participant and clamp dispersal factors as the current
  helpers do.

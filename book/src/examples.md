# Examples

Concrete scenario shapes that illustrate how to combine topologies, workloads,
and expectations. Adjust counts, rates, and durations to fit your environment.

## Simple 2-validator transaction workload
- **Topology**: two validators.
- **Workload**: transaction submissions at a modest per-block rate with a small
  set of wallet actors.
- **Expectations**: consensus liveness and inclusion of submitted activity.
- **When to use**: smoke tests for consensus and transaction flow on minimal
  hardware.

## DA + transaction workload
- **Topology**: validators plus executors if available.
- **Workloads**: data-availability blobs/channels and transactions running
  together to stress both paths.
- **Expectations**: consensus liveness and workload-level inclusion/availability
  checks.
- **When to use**: end-to-end coverage of transaction and DA layers in one run.

## Chaos + liveness check
- **Topology**: validators (optionally executors) with node control enabled.
- **Workloads**: baseline traffic (transactions or DA) plus chaos restarts on
  selected roles.
- **Expectations**: consensus liveness to confirm the system keeps progressing
  despite restarts; workload-specific inclusion if traffic is present.
- **When to use**: resilience validation and operational readiness drills.

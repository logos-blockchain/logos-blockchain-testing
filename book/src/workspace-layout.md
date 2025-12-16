# Workspace Layout

The workspace focuses on multi-node integration testing and sits alongside a
`nomos-node` checkout. Its crates separate concerns to keep scenarios
repeatable and portable:

- **Configs**: prepares high-level node, network, tracing, and wallet settings
  used across test environments.
- **Core scenario orchestration**: the engine that holds topology descriptions,
  scenario plans, runtimes, workloads, and expectations.
- **Workflows**: ready-made workloads (transactions, data-availability, chaos)
  and reusable expectations assembled into a user-facing DSL.
- **Runners**: deployment backends for local processes, Docker Compose, and
  Kubernetes, all consuming the same scenario plan.
- **Runner Examples** (crate name: `runner-examples`, path: `examples/`):
  runnable binaries (`examples/src/bin/local_runner.rs`,
  `examples/src/bin/compose_runner.rs`, `examples/src/bin/k8s_runner.rs`) that
  demonstrate complete scenario execution with each deployer.

This split keeps configuration, orchestration, reusable traffic patterns, and
deployment adapters loosely coupled while sharing one mental model for tests.

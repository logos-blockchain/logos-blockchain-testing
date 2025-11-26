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
- **Test workflows**: example scenarios and integration checks that show how
  the pieces fit together.

This split keeps configuration, orchestration, reusable traffic patterns, and
deployment adapters loosely coupled while sharing one mental model for tests.

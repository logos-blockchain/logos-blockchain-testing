# Internal Crate Reference

High-level roles of the crates that make up the framework:

- **Configs**: prepares reusable configuration primitives for nodes, networking,
  tracing, data availability, and wallets, shared by all scenarios and runners.
- **Core scenario orchestration**: houses the topology and scenario model,
  runtime coordination, node clients, and readiness/health probes.
- **Workflows**: packages workloads and expectations into reusable building
  blocks and offers a fluent DSL to assemble them.
- **Runners**: implements deployment backends (local host, Docker Compose,
  Kubernetes) that all consume the same scenario plan.
- **Test workflows**: example scenarios and integration checks that exercise the
  framework end to end and serve as living documentation.

Use this map to locate where to add new capabilities: configuration primitives
in configs, orchestration changes in core, reusable traffic/assertions in
workflows, environment adapters in runners, and demonstrations in tests.

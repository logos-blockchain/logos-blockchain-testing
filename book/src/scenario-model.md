# Scenario Model (Developer Level)

The scenario model defines clear, composable responsibilities:

- **Topology**: a declarative description of the cluster—how many nodes, their
  roles, and the broad network and data-availability characteristics. It
  represents the intended shape of the system under test.
- **Scenario**: a plan combining topology, workloads, expectations, and a run
  window. Building a scenario validates prerequisites (like seeded wallets) and
  ensures the run lasts long enough to observe meaningful block progression.
- **Workloads**: asynchronous tasks that generate traffic or conditions. They
  use shared context to interact with the deployed cluster and may bundle
  default expectations.
- **Expectations**: post-run assertions. They can capture baselines before
  workloads start and evaluate success once activity stops.
- **Runtime**: coordinates workloads and expectations for the configured
  duration, enforces cooldowns when control actions occur, and ensures cleanup
  so runs do not leak resources.

Developers extending the model should keep these boundaries strict: topology
describes, scenarios assemble, runners deploy, workloads drive, and expectations
judge outcomes. For guidance on adding new capabilities, see
[Extending the Framework](extending.md).

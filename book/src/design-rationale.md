# Design Rationale

- **Modular crates** keep configuration, orchestration, workloads, and runners decoupled so each can evolve without breaking the others.
- **Pluggable runners** let the same scenario run on a laptop, a Docker host, or a Kubernetes cluster, making validation portable across environments.
- **Separated workloads and expectations** clarify intent: what traffic to generate versus how to judge success. This simplifies review and reuse.
- **Declarative topology** makes cluster shape explicit and repeatable, reducing surprise when moving between CI and developer machines.
- **Maintainability through predictability**: a clear flow from plan to deployment to verification lowers the cost of extending the framework and interpreting failures.

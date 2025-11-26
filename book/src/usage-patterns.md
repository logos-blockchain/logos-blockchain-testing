# Usage Patterns

- **Shape a topology, pick a runner**: choose local for quick iteration, compose for reproducible multi-node stacks with observability, or k8s for cluster-grade validation.
- **Compose workloads deliberately**: pair transactions and data-availability traffic for end-to-end coverage; add chaos only when assessing recovery and resilience.
- **Align expectations with goals**: use liveness-style checks to confirm the system keeps up with planned activity, and add workload-specific assertions for inclusion or availability.
- **Reuse plans across environments**: keep the scenario constant while swapping runners to compare behavior between developer machines and CI clusters.
- **Iterate with clear signals**: treat expectation outcomes as the primary pass/fail indicator, and adjust topology or workloads based on what those signals reveal.

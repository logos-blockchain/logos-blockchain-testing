# Part IV — Uniform Clusters and Configuration

This part shows how to put your own node behind the framework and control how clusters are configured and driven.

It covers the uniform-cluster entry pattern in depth: implementing `Application`, describing topologies, generating per-node configuration, and driving nodes imperatively outside the scenario runtime.

- [Implementing Application](implementing-application.md) — the environment contract for your node
- [Topology and Deployment Plans](topology.md) — describing cluster shape
- [Ports, Peers, Node Config, and Readiness](node-config.md) — per-node configuration mechanics
- [Static Artifacts and cfgsync](cfgsync.md) — typed app config to per-node artifacts to backend rendering
- [Seeds and Reproducibility](seeds.md) — deterministic deployments
- [ManualCluster: Imperative Node Control](manual-cluster.md) — direct node lifecycle control
- [Persistence, Snapshots, and Recovery Testing](persistence.md) — state across restarts

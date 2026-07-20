# Ownership and Design Boundaries

This chapter lists the responsibilities of the framework and of an application repository.

---

## The Boundary

The scenario engine never names a concrete application. Its only coupling point is the `Application` trait: a bundle of associated types (`Deployment`, `NodeClient`, `NodeConfig`) that the engine plumbs around generically. Everything that knows what your system *is* (its binary, its config format, its client, its notion of "healthy") lives on your side of that trait.

| Concern | Owner |
|---|---|
| Process lifetime (spawn, stop, restart, PIDs) | Framework |
| Working directories and temp dirs | Framework |
| Cleanup guards and teardown ordering | Framework |
| Topology mechanics (ports, peers, node names) | Framework |
| Readiness probing, retry, and gating | Framework |
| Handle storage and lookup (`HandleRegistry`, `AppRuntime`, `RunContext`) | Framework |
| Workload/expectation scheduling and aggregation | Framework |
| Node binaries and how to obtain them | Application repo |
| `NodeConfig` shape and rendering | Application repo |
| Typed node clients | Application repo |
| Readiness endpoints and app-specific checks | Application repo |
| Domain handles (`StoreHandle`, `WalletHandle`, ...) | Application repo |
| Meaningful workloads, expectations, scenarios | Application repo |

---

## What the Framework Owns

**Process lifetime and working directories.** Deployers spawn node processes into per-run working directories, track PIDs, and stop everything on teardown. Artifact retention is policy (`CleanupPolicy::preserve_artifacts`), not something scenarios hand-roll.

**Cleanup.** Teardown is guard-based and automatic: cleanup guards chain and run in reverse registration order when the `RunHandle` drops, and the same guards run on the failure path. App-layer adapters register managed resources in a LIFO cleanup stack so dependants stop before dependencies, independently of exposed handle clones ([Handle Ownership and Teardown](handles-teardown.md)).

**Topology mechanics.** Port allocation, peer wiring, node naming, and readiness gating with retry are all generic over `E: Application`. The engine asks your environment *what* to render and probe, never *why*.

**Handle storage and lookup.** `DeployContext` collects typed handles during preparation; `AppRuntime` carries them into the run; `AppRunContextExt` returns clones to workloads. Duplicate exposure of a type/name pair is an error, never a silent replacement.

---

## What the Application Repository Owns

The kvstore example is the template. Its integration crate supplies, in its own repository:

- **The binary and how to get it**: a `FallbackBinaryProvider` chain that uses `KVSTORE_NODE_BIN` if set and otherwise builds `kvstore-node` with cargo ([Binary Providers](binary-providers.md)).
- **Config**: `KvNodeConfig`, built per node from the framework's port/peer views and rendered to YAML.
- **Client**: `KvHttpClient`, constructed in `Application::build_node_client`.
- **Readiness**: `node_readiness_path()` returning `/health/ready`.
- **Domain handles and presets**: `KvStoreCluster`, `KvLocalApp`, `KvExistingClusterApp`.
- **Scenarios that mean something**: write workloads, convergence expectations, runnable bins.

```rust,ignore
// Application side: domain knowledge, no orchestration.
fn node_readiness_path() -> &'static str {
    "/health/ready"
}

// Framework side: orchestration, no domain knowledge.
// It only ever sees E::NodeClient, E::NodeConfig, E::Deployment.
```

Sources: `examples/kvstore/testing/integration/src/{app,local_env}.rs`, `testing-framework/app/src/lib.rs`.

---

## How the Boundary Is Enforced

**Unsupported defaults.** `Application::build_node_client` and `external_node_client` return an "unsupported" error by default. Capabilities are available only when the environment implements them.

**Generic application types.** There is no global list of known applications or framework config file naming their binaries. `testing-framework-core` compiles against `E: Application`, so it does not depend on adopter types or their dependencies. The same runtime can therefore be instantiated with kvstore, openraft_kv, nats, or an application from another repository.

**Application-owned composition.** Application repositories implement `AppDeployment`, compose children through `DeployContext`, and expose typed handles. The framework supplies the context, registry, and lifecycle without defining the application stack.

**CI boundary check.** `scripts/run/check-boundaries.sh` checks an application-side topology crate for framework-extension symbols (`cfgsync`, `ComposeDeployEnv`, `K8sDeployEnv`, `runner-compose`, `runner-k8s`). This detects backend dependencies in topology code. The compiler enforces the reverse direction because core crates do not reference concrete application types.

> **External example:** the current boundary script targets logos-blockchain's `lb-topology` crate (in its own checkout), which keeps that adopter's topology code local/topology-focused. The pattern generalizes: point the same grep at your own integration crates.

Application-specific config formats and startup rules belong in the environment implementation or an `AppDeployment`, not in framework crates.

---

## Where to Go Next

- [Application, AppDeployment, and Environments](application-model.md): the trait that defines the boundary.
- [Implementing Application](implementing-application.md): building your side of it.
- [Framework vs Application Boundaries](tf-boundaries.md): the reference treatment in Part VI.
- [Public Extension Points](extension-points.md): the sanctioned ways to extend the framework itself.

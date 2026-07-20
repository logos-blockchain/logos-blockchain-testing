# Uniform Child Clusters: LocalAppCluster

`LocalAppCluster<E>` runs an additional uniform cluster of local processes as one child of a composed stack.

For N identical nodes of one binary with peer wiring and per-node clients, use `ScenarioBuilder<E>` when the cluster is the system under test. When the cluster is one component of a larger stack, deploy it as a `LocalAppCluster` inside the root deployment.

The environment `E` must implement `LocalDeployerEnv` (config rendering, ports, process spec; see [Local Deployer](deployer-local.md)). That work is the same whether the app runs standalone or as a child, so a cluster env written for uniform scenarios is reusable here unchanged.

---

## Starting a Child Cluster

Inside an `AppDeployment`, `DeployContext::deploy_local_cluster` launches every node described by the deployment (`node-0`, `node-1`, ...), waits for network readiness, registers cleanup, and returns the cluster handle:

```rust,ignore
// examples/kvstore/testing/integration/src/app.rs
#[derive(Clone)]
pub struct KvLocalApp {
    deployment: KvTopology,
}

impl KvLocalApp {
    #[must_use]
    pub fn nodes(nodes: usize) -> Self {
        Self { deployment: KvTopology::new(nodes) }
    }
}

#[async_trait]
impl AppDeployment<AppHostEnv> for KvLocalApp {
    type Handle = LocalAppCluster<KvEnv>;

    async fn deploy(self, ctx: &mut DeployContext<AppHostEnv>) -> Result<Self::Handle, DynError> {
        ctx.deploy_local_cluster::<KvEnv>(self.deployment).await
    }
}
```

The kvstore preset delegates cluster provisioning to `deploy_local_cluster`, which registers a cleanup guard and returns a cloneable access and control handle. Scenario cleanup stops any remaining nodes independently of handle clones.

---

## The Handle API

<details>
<summary>LocalAppCluster handle method reference</summary>

| Method | Purpose |
|--------|---------|
| `deployment()` / `node_count()` | The cluster's deployment descriptor and node count. |
| `node_clients()` | Shared `NodeClients<E>` collection. |
| `clients()` | Snapshot of all currently available clients. |
| `first_client()` | First available client, if any. |
| `node_client(name)` | Client for one node, if started. |
| `node_pid(name)` | OS process id for one node, if running. |
| `start_node(name)` / `start_node_with(name, options)` | Start a node, optionally with `StartNodeOptions` (config overrides, persist/snapshot dirs, args). |
| `stop_node(name)` | Stop a node. |
| `restart_node(name)` / `restart_node_with(name, options)` | Restart with existing or explicit options. |
| `wait_network_ready()` | Wait for the cluster-level readiness condition. |
| `wait_node_ready(name)` | Wait for one node to report ready. |

</details>

Node names follow the `node-{index}` convention used at startup. `LocalAppCluster<E>` is the backend-independent `ClusterHandle<E>` alias; it exposes the supported common control surface rather than an underlying `ManualCluster`.

Per-node control is provided by the cluster handle. A workload restarting a child-cluster node does not need the scenario-level `with_node_control()` capability.

---

## Worked Example: kvstore Convergence Across a Restart

The `kvstore_app_host_convergence` bin runs a three-node kv cluster as an app, then exercises convergence across a node restart:

```rust,ignore
// examples/kvstore/examples/src/bin/app_host_convergence.rs
let mut scenario = AppHost::scenario()
    .with_app(KvLocalApp::nodes(3))
    .with_run_duration(Duration::from_secs(5))
    .with_workload(KvAppHostConvergence::new(3))
    .build()?;

let deployer = AppHostLocalDeployer::default();
let runner = deployer.deploy(&scenario).await?;
runner.run(&mut scenario).await?;
```

The workload requires the cluster handle and drives it directly:

```rust,ignore
async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
    let cluster = ctx.require_app::<LocalAppCluster<KvEnv>>()?;

    ensure_cluster_shape(&cluster, self.expected_nodes)?;
    put_value(&cluster, "before-restart").await?;
    cluster.restart_node("node-0").await?;
    cluster.wait_node_ready("node-0").await?;
    put_value(&cluster, "after-restart").await?;

    Ok(())
}
```

`put_value` writes through `cluster.first_client()`; `ensure_cluster_shape` checks `node_count()`, `clients()`, `node_client("node-0")`, and `node_pid("node-0")`. Run it with:

```bash
cargo run -p kvstore-examples --bin kvstore_app_host_convergence
```

The kvstore environment resolves its node binary through a fallback provider chain, so this example does not require a manually configured binary path (see [Binary Providers](binary-providers.md)).

---

## Exposing the Cluster to Workloads

`KvLocalApp` returns the raw `LocalAppCluster<KvEnv>` as its handle, and the factory auto-exposes it, so workloads request `LocalAppCluster<KvEnv>` directly. In a composed stack you can either expose the raw cluster handle (as the job stack does), wrap it in a domain newtype (`StoreHandle`) for clearer requirements, or use named handles when two child clusters share an environment type (see [Composing Heterogeneous Stacks](composing-stacks.md)).

---

## See Also

- [One Binary: LocalProcessApp](local-process-app.md): the single-process counterpart.
- [Backend Scope](app-backend-scope.md): why child clusters are local-only today.

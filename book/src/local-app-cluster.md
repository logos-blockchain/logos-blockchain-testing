# Uniform Clusters: ClusterApp and ClusterHandle

`ClusterApp<E>` deploys a uniform cluster of an application's nodes as one child of a composed stack, and `ClusterHandle<E>` is the backend-neutral handle it exposes.

For N identical nodes of one binary with peer wiring and per-node clients, register a `ClusterApp` with `with_app` (local processes) or `with_app_using` (Compose or Kubernetes). A scenario over one uniform cluster is simply the composition with a single child.

For local runs the environment `E` must implement `LocalDeployerEnv` (config rendering, ports, process spec; see [Local Deployer](deployer-local.md)). That work is the same whether the cluster is the whole system or one component of a larger stack, so a cluster env is reusable in both shapes unchanged.

---

## Deploying a Cluster

The prebuilt `ClusterApp<E>` covers the common case directly on the builder:

```rust,ignore
use testing_framework_app::{AppHost, AppScenarioBuilderExt as _, ClusterApp};

let builder = AppHost::scenario()
    .with_app(ClusterApp::<KvEnv>::new(KvTopology::new(3)));
```

`ClusterApp` builds a managed `ClusterRequest` for the deployment: nodes start eagerly, full node control is requested, and the backend registers cleanup with the scenario. Optional modifiers adjust the request:

| Modifier | Effect |
|----------|--------|
| `with_policy(policy)` | Per-cluster [`DeploymentPolicy`](deployment-policies.md) override. |
| `with_start_mode(ClusterStartMode::OnDemand)` | Provision capacity without starting nodes; tests start them through the handle. |
| `with_control(ClusterControlRequest::None)` | Do not request node control. |
| `with_external_nodes(nodes)` | Join external endpoints to the managed inventory. |
| `with_observability(inputs)` | Supply [`ObservabilityInputs`](observation.md) to the backend. |

Inside a custom `AppDeployment`, the same operation is available as `DeployContext::deploy_cluster`, which launches every node described by the deployment (`node-0`, `node-1`, ...), waits for readiness per policy, registers cleanup, and returns the handle:

```rust,ignore
// examples/kvstore/testing/integration/src/app.rs
#[async_trait]
impl AppDeployment<AppHostEnv> for KvLocalApp {
    type Handle = ClusterHandle<KvEnv>;

    async fn deploy(self, ctx: &mut DeployContext<AppHostEnv>) -> Result<Self::Handle, DynError> {
        ctx.deploy_cluster(ClusterRequest::<KvEnv>::managed(self.deployment))
            .await
    }
}
```

Scenario cleanup stops any remaining nodes independently of handle clones.

---

## The Handle API

<details>
<summary>ClusterHandle method reference</summary>

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
| `observability()` / `metrics()` | The cluster's observability inputs and a telemetry handle over them (see [Telemetry](telemetry.md)). |

</details>

Node names follow the `node-{index}` convention used at startup. `ClusterHandle<E>` is backend-independent: the same methods work whether the cluster was provisioned on local processes, Compose, or Kubernetes, and control methods error at call time when the backend did not grant the corresponding control.

Per-node control is provided by the cluster handle; nothing needs to be enabled on the scenario for a workload to restart a cluster node.

---

## Worked Example: kvstore Convergence

The `kvstore_basic_convergence` bin runs a three-node kv cluster as an app and exercises convergence under write load:

```rust,ignore
// examples/kvstore/examples/src/bin/basic_convergence.rs
let mut scenario = AppHost::scenario()
    .with_app(ClusterApp::<KvEnv>::new(KvTopology::new(3)))
    .with_run_duration(Duration::from_secs(30))
    .with_workload(KvClusterAccessible::new(3))
    .with_workload(KvWriteWorkload::new().operations(300).key_count(30))
    .with_expectation(KvConverges::new("demo", 30))
    .build()?;

let runner = AppHostDeployer.deploy(&scenario).await?;
runner.run(&mut scenario).await?;
```

Workloads require the cluster handle and drive it directly:

```rust,ignore
async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
    let cluster = ctx.require_app::<ClusterHandle<KvEnv>>()?;

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
cargo run -p kvstore-examples --bin kvstore_basic_convergence
```

The kvstore environment resolves its node binary through a fallback provider chain, so this example does not require a manually configured binary path (see [Binary Providers](binary-providers.md)).

---

## Exposing the Cluster to Workloads

`ClusterApp` returns the raw `ClusterHandle<E>` as its handle, and the factory auto-exposes it, so workloads request `ClusterHandle<KvEnv>` directly. In a composed stack you can either expose the raw cluster handle (as the job stack does), wrap it in a domain newtype (`StoreHandle`) for clearer requirements, or use named handles when two child clusters share an environment type (see [Composing Heterogeneous Stacks](composing-stacks.md)).

---

## See Also

- [One Binary: LocalProcessApp](local-process-app.md): the single-process counterpart.
- [Backend Scope](app-backend-scope.md): what each backend provisioner supports.

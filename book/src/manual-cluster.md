# ManualCluster: Imperative Node Control

`ManualCluster` provides imperative node lifecycle control. Your code starts, stops, and restarts nodes directly without using the scenario runner.

---

## When to Use It

Use `ManualCluster` when orchestration lives outside the scenario runtime. Scenarios can also start and restart nodes from workloads through the `ClusterHandle`; see [Chaos and Controlled Failure](chaos.md).

- **Step-driven flows**: an external driver decides when each node starts and what happens next.
- **BDD harnesses**: Gherkin steps map naturally onto imperative start/stop/wait calls.
- **Exploratory debugging**: poke at a live cluster from a `main` function without writing workloads or expectations.

There are no workloads, expectations, or `RunContext`; you call methods and assert with your own code.

---

## Creating a Cluster

On the local backend (`testing-framework/deployers/local/src/manual/mod.rs`), construct one directly from a deployment descriptor:

```rust,ignore
use testing_framework_runner_local::ManualCluster;

let cluster = ManualCluster::<KvEnv>::from_topology(KvTopology::new(3));
```

The descriptor defines capacity and indexing, not initial state: no processes exist until you call `start_node`. `E` must implement `LocalDeployerEnv` (see [Implementing Application](implementing-application.md)).

On Kubernetes, `testing_framework_runner_k8s::ManualCluster` is constructed asynchronously because it installs the Helm stack first:

```rust,ignore
use testing_framework_core::scenario::{ClusterStartMode, DeploymentPolicy, ObservabilityInputs};
use testing_framework_runner_k8s::ManualCluster;

// Install the stack with no nodes running…
let cluster = ManualCluster::<KvEnv>::from_topology(KvTopology::new(3)).await?;

// …or spell out the start mode, policy, and observability inputs
let cluster = ManualCluster::<KvEnv>::provision(
    KvTopology::new(3),
    ClusterStartMode::OnDemand,
    DeploymentPolicy::default(),
    &ObservabilityInputs::default(),
)
.await?;
```

Manual clusters are the same machinery the managed path uses: a `ClusterApp` built with `.with_start_mode(ClusterStartMode::OnDemand)` provisions the identical capacity inside a scenario, leaving node starts to workloads driving the `ClusterHandle`.

**Naming:** requested names are normalized to a `node-` prefix: `start_node("a")` registers `node-a`; names already starting with `node-` pass through; an empty name becomes `node-<index>`. Each started node needs a fresh name; reusing a registered name is an error.

---

## API

| Method | What it does |
|---|---|
| `start_node(name)` | Start a node with default options |
| `start_node_with(name, options)` | Start with `StartNodeOptions` (below); returns `StartedNode { name, client }` |
| `stop_node(name)` | Kill the process; the node stays registered |
| `stop_all()` | Stop every node and reset registration state (also runs on drop) |
| `restart_node(name)` | Stop and respawn in the same working directory |
| `restart_node_with(name, options)` | Restart with extra `args` / `runtime`; other overrides rejected |
| `wait_network_ready()` | Poll every started node's readiness endpoint (`AllNodesReady`) |
| `wait_node_ready(name)` | Poll one node, honoring its `start_timeout` |
| `node_client(name)` / `node_clients()` | Look up one client / the shared `NodeClients<E>` collection |
| `node_pid(name)` | OS pid, `None` if the process is not running |
| `add_external_sources(sources)` | Build clients for `ExternalNodeSource`s and add them to the client set |
| `add_external_clients(clients)` | Add prebuilt clients to the client set |

`ManualCluster` also implements the core `NodeControlHandle<E>` and `ClusterWaitHandle<E>` traits, so it can stand behind code written against those abstractions. An app-layer child cluster exposes the same common operations through `ClusterHandle<E>`, without exposing the backend-specific `ManualCluster` object.

---

## StartNodeOptions

The full options struct (`core/src/scenario/control.rs`):

| Field | Type | Builder | Meaning |
|---|---|---|---|
| `peers` | `Option<PeerSelection>` | `with_peers(sel)` | `DefaultLayout`, `None`, or `Named(names)` — see [node-config.md](node-config.md) for where each path honors it |
| `config_override` | `Option<E::NodeConfig>` | `with_config_override(cfg)` | Replace the generated config wholesale |
| `config_patch` | patch closure | `create_patch(fn)` | Transform the generated config before spawn |
| `persist_dir` | `Option<PathBuf>` | `with_persist_dir(path)` | Place the working directory predictably — see [Persistence](persistence.md) |
| `snapshot_dir` | `Option<PathBuf>` | `with_snapshot_dir(path)` | Seed the working directory from saved state — see [Persistence](persistence.md) |
| `args` | `Vec<String>` | `with_args(args)` | Extra CLI args appended on launch |
| `runtime` | `NodeRuntimeOptions` | `with_runtime(opts)` / `with_start_timeout(dur)` | Per-node readiness timeout |

`restart_node_with` accepts only `args` and `runtime`. Passing `peers`, `config_override`, `config_patch`, `persist_dir`, or `snapshot_dir` to a restart returns an `InvalidArgument` error, because a restart reuses the node's existing config and working directory. To change those, stop the node and start a new one.

---

## Example: Convergence Under Restart

Adapted from the in-repo example `cargo run -p kvstore-examples --bin kvstore_k8s_manual_convergence` (`examples/kvstore/examples/src/bin/k8s_manual_convergence.rs`):

```rust,ignore
let cluster = ManualCluster::<KvEnv>::from_topology(KvTopology::new(3)).await?;

let node0 = cluster.start_node("node-0").await?.client;
let node1 = cluster.start_node("node-1").await?.client;
let node2 = cluster.start_node("node-2").await?.client;
cluster.wait_network_ready().await?;

write_keys(&node0, "kv-manual", 12).await?;
wait_for_convergence(&[node0.clone(), node1.clone(), node2.clone()], "kv-manual", 12).await?;

cluster.restart_node("node-2").await?;
cluster.wait_network_ready().await?;

let node2 = cluster.node_client("node-2").expect("client after restart");
wait_for_convergence(&[node0, node1, node2], "kv-manual", 12).await?;

cluster.stop_all();
```

The driver determines which nodes exist, when writes happen, what convergence means, and when to inject the restart. `write_keys` and `wait_for_convergence` are plain functions over the application's HTTP client.

That example runs on Kubernetes: the k8s `ManualCluster` offers the same method surface over pods in a per-run namespace. The local `ManualCluster` starts processes directly and needs no external infrastructure.

---

## Lifecycle and Cleanup

Dropping the `ManualCluster` calls `stop_all()`: every child process is killed and waited on. Node working directories are temporary and removed with the processes unless retained. Set `TF_KEEP_LOGS=1` (or `true`/`yes`) to keep them for inspection, and see [Persistence](persistence.md) for deliberate state retention.

> **External example:** logos-blockchain's cucumber suite drives `ManualCluster` from Gherkin steps in its own repository, including dependency-ordered starts, targeted restarts, snapshot-on-stop, and restore-from-snapshot.

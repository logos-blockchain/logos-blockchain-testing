# ManualCluster: Imperative Node Control

`ManualCluster` provides imperative node lifecycle control. Your code starts, stops, and restarts nodes directly without using the scenario runner.

---

## When to Use It

Use `ManualCluster` when orchestration lives outside the scenario runtime. Scenarios can also start and restart nodes from workloads by requesting `with_node_control()`; see [Scenario Capabilities](capabilities.md) and [Chaos and Controlled Failure](chaos.md).

- **Step-driven flows**: an external driver decides when each node starts and what happens next.
- **BDD harnesses**: Gherkin steps map naturally onto imperative start/stop/wait calls.
- **Exploratory debugging**: poke at a live cluster from a `main` function without writing workloads or expectations.

There are no workloads, expectations, or `RunContext`; you call methods and assert with your own code.

---

## Creating a Cluster

Two equivalent entry points on the local backend (`testing-framework/deployers/local/src/manual/mod.rs`):

```rust,ignore
use testing_framework_runner_local::{ManualCluster, ProcessDeployer};

// Directly from a deployment descriptor…
let cluster = ManualCluster::<KvEnv>::from_topology(KvTopology::new(3));

// …or via the deployer
let deployer = ProcessDeployer::<KvEnv>::new();
let cluster = deployer.manual_cluster_from_descriptors(KvTopology::new(3));
```

The descriptor defines capacity and indexing, not initial state: no processes exist until you call `start_node`. `E` must implement `LocalDeployerEnv` (see [Implementing Application](implementing-application.md)).

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

The full options struct (`core/src/scenario/capabilities.rs`):

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
let deployer = KvK8sDeployer::new();
let cluster = deployer
    .manual_cluster_from_descriptors(KvTopology::new(3))
    .await?;

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

That example runs on Kubernetes: the Kubernetes deployer supplies a manual cluster with the same method surface (`manual_cluster_from_descriptors` there is `async` and fallible because it must install the stack first). The local `ManualCluster` documented in this chapter starts processes directly and needs no external infrastructure.

---

## Lifecycle and Cleanup

Dropping the `ManualCluster` calls `stop_all()`: every child process is killed and waited on. Node working directories are temporary and removed with the processes unless retained. Set `TF_KEEP_LOGS=1` (or `true`/`yes`) to keep them for inspection, and see [Persistence](persistence.md) for deliberate state retention.

> **External example:** logos-blockchain's cucumber suite drives `ManualCluster` from Gherkin steps in its own repository, including dependency-ordered starts, targeted restarts, snapshot-on-stop, and restore-from-snapshot.

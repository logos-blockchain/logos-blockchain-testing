# Local Deployer

`ProcessDeployer` runs every node as a local OS process. It is the default backend.

The local deployer lives in the `testing-framework-runner-local` crate. It requires no Docker daemon and no cluster: it resolves a node binary, writes each node's config into a private working directory, spawns the processes, probes readiness, and hands the running cluster to the scenario runner.

```rust,ignore
use kvstore_runtime_ext::KvLocalDeployer; // = ProcessDeployer<KvEnv>
use testing_framework_core::scenario::Deployer;

let deployer = KvLocalDeployer::default();
let runner = deployer.deploy(&scenario).await?;
runner.run(&mut scenario).await?;
```

Run the demonstration binary with `cargo run -p kvstore-examples --bin kvstore_basic_convergence`. No manual binary setup is needed, because kvstore's fallback provider chain builds the node binary on first use (see [Binary Providers](binary-providers.md)).

---

## What deploy Does

For a managed scenario, `ProcessDeployer::deploy`:

1. Validates the cluster mode: the local deployer rejects `ClusterMode::ExistingCluster` (attach is a compose/k8s feature, see [Existing and External Clusters](external-clusters.md)).
2. Builds the source orchestration plan and spawns one `ProcessNode` per topology entry.
3. Probes readiness and retries the whole spawn on failure (see below).
4. Merges external node clients into the managed set.
5. Assembles the runtime and returns a `Runner<E>` whose cleanup guard owns the node processes.

The main runtime types are:

- **`ProcessDeployer<E>`**: the deployer. `E` implements `LocalDeployerEnv` (full-control hooks) or the compact `LocalBinaryApp` trait (one binary + one config file + one HTTP port per node).
- **`LaunchSpec`**: the launch plan for one process: binary path, files to materialize, CLI args, env vars.
- **`ProcessNode`**: a spawned child process plus its tempdir, endpoints, and typed client.

---

## Working Directories and Logs

Each node gets its own temporary working directory, created under the current directory (or under a caller-supplied persist path). Config files and any other `LaunchFile` entries are written there before spawn, and the process starts with that directory as its cwd.

Node stdout and stderr are inherited from the test process, so node logs interleave with your test output; control verbosity with the `RUST_LOG` value configured on the app's `LocalProcessSpec` (for example `.with_rust_log("kvstore_node=info")`).

On drop, each `ProcessNode` kills its child and removes the tempdir. Two things preserve working directories instead of deleting them:

- `DeploymentPolicy` with `cleanup_policy.preserve_artifacts = true` (see [Readiness, Retry, and Artifact Preservation](deployment-policies.md)), or the `TF_KEEP_LOGS=1` env var.
- A panicking test thread, in which case directories are kept automatically for inspection.

Nodes started with a `persist_dir` or seeded from a `snapshot_dir` (via `StartNodeOptions`) copy or place state accordingly before spawn (see [Persistence, Snapshots, and Recovery Testing](persistence.md)).

---

## Ports

The deployer reserves real OS ports up front: `allocate_available_port()` binds an ephemeral listener and releases it, and `reserve_local_node_ports` reserves the network port plus any app-named extra ports for each node. Endpoints are surfaced as `NodeEndpoints` (an API socket address plus named extra ports), from which the app builds its typed `NodeClient`.

---

## Readiness and Retry

Readiness is governed by the scenario's `DeploymentPolicy` combined with the deployer's own switch:

| Control | Effect |
|---|---|
| `ProcessDeployer::with_membership_check(false)` | Disables local readiness probing entirely |
| `DeploymentPolicy.readiness_enabled` | Must also be true for probes to run |
| `DeploymentPolicy.readiness_requirement` | `AllNodesReady`, `AnyNodeReady`, or `AtLeast(n)` |
| `DeploymentPolicy.retry_policy` | Attempts and backoff; defaults to 3 attempts, 250 ms base, 2 s max |

The probe shape comes from the environment: `LocalReadinessProbe::HttpGet { path }` (default, using `Application::node_readiness_path()`) or `LocalReadinessProbe::Tcp`. If spawn or readiness fails, all nodes from that attempt are dropped and the entire cluster is respawned with exponential backoff and jitter, up to the retry budget.

---

## Node Control

The local deployer supports the complete node-control surface. Building the scenario with `with_node_control()` deploys through `Deployer<E, NodeControlCapability>`, which wraps the spawned nodes in a `NodeManager`. Workloads can then start, stop, and restart nodes by name, with full `StartNodeOptions` support (peer selection, config overrides and patches, persist and snapshot directories, extra args, start timeouts). The openraft_kv failover scenario uses this path:

```bash
cargo run -p openraft-kv-examples --bin openraft_kv_basic_failover
```

See [Scenario Capabilities](capabilities.md) for the capability-gated builder.

---

## Manual Clusters

For orchestration outside the scenario runner, such as Cucumber steps or another test harness, the deployer provides an imperative cluster:

```rust,ignore
let deployer = ProcessDeployer::<KvEnv>::new();
let cluster = deployer.manual_cluster_from_descriptors(descriptors);

cluster.start_node("node-0").await?;
cluster.wait_network_ready().await?;
cluster.stop_all();
```

`ManualCluster` exposes `start_node(_with)`, `stop_node`, `restart_node(_with)`, `wait_node_ready`, `wait_network_ready`, `node_client`, `node_pid`, `node_clients`, and `add_external_sources` / `add_external_clients`. It is covered in depth in [ManualCluster: Imperative Node Control](manual-cluster.md).

---

## Binary Resolution

Every local node needs an executable. `LocalProcessSpec::new("MY_NODE_BIN")` defaults to an env-var provider; `with_binary_provider` swaps in any `BinaryProvider`, including fallback chains that try an env override first and build with Cargo otherwise. Resolution is cached per process and locked across processes. Full detail in [Binary Providers](binary-providers.md).

---

The local deployer supports external node sources (`with_external_node`) but not attached existing clusters. If `Application::external_node_client` is not implemented, it falls back to parsing the endpoint (`http://host:port`) and building a client from the resolved socket address.

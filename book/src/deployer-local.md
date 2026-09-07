# Local Backend

`LocalClusterProvisioner` runs every node as a local OS process. It is the default backend.

The local backend lives in the `testing-framework-runner-local` crate. It requires no Docker daemon and no cluster: it resolves a node binary, writes each node's config into a private working directory, spawns the processes, probes readiness, and returns a `ClusterHandle` backed by the running cluster. `with_app(...)` uses it implicitly; naming it is equivalent:

```rust,ignore
use testing_framework_runner_local::LocalClusterProvisioner;

let mut scenario = AppHost::scenario()
    .with_app_using(
        ClusterApp::<KvEnv>::new(KvTopology::new(3)),
        LocalClusterProvisioner,
    )
    .build()?;

let runner = AppHostDeployer.deploy(&scenario).await?;
runner.run(&mut scenario).await?;
```

Run the demonstration binary with `cargo run -p kvstore-examples --bin kvstore_basic_convergence`. No manual binary setup is needed, because kvstore's fallback provider chain builds the node binary on first use (see [Binary Providers](binary-providers.md)).

---

## What provision_cluster Does

For a managed request, `LocalClusterProvisioner::provision_cluster`:

1. Creates an empty `LocalCluster` for the deployment (honoring artifact-preservation policy).
2. With the default `ClusterStartMode::Eager`, spawns one process per topology entry, probes readiness, and retries the whole spawn on failure (see below). With `OnDemand`, no node starts and the handle's profile is `ManualControlled`.
3. Merges any external node sources into the client inventory.
4. Attaches the readiness wait, the cleanup guard, and — when the request asks for `ClusterControlRequest::Full` — the full node-control surface.

Attached requests are rejected (`AttachedUnsupported`; attach is a compose/k8s feature, see [Existing and External Clusters](external-clusters.md)); external requests produce a clients-only `ExternalUncontrolled` unit.

The main runtime types are:

- **`LocalClusterProvisioner`**: the provisioner. `E` implements `LocalDeployerEnv` (full-control hooks) or the compact `LocalBinaryApp` trait (one binary + one config file + one HTTP port per node).
- **`LaunchSpec`**: the launch plan for one process: binary path, files to materialize, CLI args, env vars.
- **`LocalCluster<E>`**: the concrete cluster: spawned processes, node manager, clients, and cleanup.

---

## Working Directories and Logs

Each node gets its own temporary working directory, created under the current directory (or under a caller-supplied persist path). Config files and any other `LaunchFile` entries are written there before spawn, and the process starts with that directory as its cwd.

Node stdout and stderr are inherited from the test process, so node logs interleave with your test output; control verbosity with the `RUST_LOG` value configured on the app's `LocalProcessSpec` (for example `.with_rust_log("kvstore_node=info")`).

On drop, each node process is killed and its tempdir removed. Two things preserve working directories instead of deleting them:

- `DeploymentPolicy` with `cleanup_policy.preserve_artifacts = true` (see [Readiness, Retry, and Artifact Preservation](deployment-policies.md)), or the `TF_KEEP_LOGS=1` env var.
- A panicking test thread, in which case directories are kept automatically for inspection.

Nodes started with a `persist_dir` or seeded from a `snapshot_dir` (via `StartNodeOptions`) copy or place state accordingly before spawn (see [Persistence, Snapshots, and Recovery Testing](persistence.md)).

---

## Ports

The backend reserves real OS ports up front: `allocate_available_port()` binds an ephemeral listener and releases it, and `reserve_local_node_ports` reserves the network port plus any app-named extra ports for each node. Endpoints are surfaced as `NodeEndpoints` (an API socket address plus named extra ports), from which the app builds its typed `NodeClient`.

---

## Readiness and Retry

Readiness is governed by the `DeploymentPolicy` carried on the cluster request (`ClusterApp::with_policy`):

| Control | Effect |
|---|---|
| `DeploymentPolicy.readiness_enabled` | Probes run only when true |
| `DeploymentPolicy.readiness_requirement` | `AllNodesReady`, `AnyNodeReady`, or `AtLeast(n)` |
| `DeploymentPolicy.retry_policy` | Attempts and backoff; defaults to 3 attempts, 250 ms base, 2 s max |

The probe shape comes from the environment: `LocalReadinessProbe::HttpGet { path }` (default, using `Application::node_readiness_path()`) or `LocalReadinessProbe::Tcp`. If spawn or readiness fails, all nodes from that attempt are dropped and the entire cluster is respawned with exponential backoff and jitter, up to the retry budget.

---

## Node Control

The local backend supports the complete node-control surface. When the request asks for `ClusterControlRequest::Full` — the `ClusterApp` default — the handle's `start_node`, `stop_node`, and `restart_node` operate on nodes by name, with full `StartNodeOptions` support (peer selection, config overrides and patches, persist and snapshot directories, extra args, start timeouts). The openraft_kv failover scenario uses this path:

```bash
cargo run -p openraft-kv-examples --bin openraft_kv_basic_failover
```

See [Cluster Control and Observability](capabilities.md) for the request fields and handle surface.

---

## Manual Clusters

For orchestration outside the scenario runner, such as Cucumber steps or another test harness, the crate provides an imperative cluster:

```rust,ignore
use testing_framework_runner_local::ManualCluster;

let cluster = ManualCluster::<KvEnv>::from_topology(descriptors);

cluster.start_node("node-0").await?;
cluster.wait_network_ready().await?;
cluster.stop_all();
```

`ManualCluster` exposes `start_node(_with)`, `stop_node`, `restart_node(_with)`, `wait_node_ready`, `wait_network_ready`, `node_client`, `node_pid`, `node_clients`, and `add_external_sources` / `add_external_clients`. It is covered in depth in [ManualCluster: Imperative Node Control](manual-cluster.md). The same on-demand lifecycle is available inside a scenario via `ClusterApp::with_start_mode(ClusterStartMode::OnDemand)`.

---

## Binary Resolution

Every local node needs an executable. `LocalProcessSpec::new("MY_NODE_BIN")` defaults to an env-var provider; `with_binary_provider` swaps in any `BinaryProvider`, including fallback chains that try an env override first and build with Cargo otherwise. Resolution is cached per process and locked across processes. Full detail in [Binary Providers](binary-providers.md).

---

The local backend supports external node sources but not attached existing clusters. If `Application::external_node_client` is not implemented, it falls back to parsing the endpoint (`http://host:port`) and building a client from the resolved socket address.

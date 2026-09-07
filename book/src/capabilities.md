# Cluster Control and Observability

Node control and telemetry endpoints are per-cluster request data. Each `ClusterRequest` states what the test needs from that cluster; the backend provisioner wires the matching runtime surfaces into the returned `ClusterHandle`. Operations a cluster does not have fail with a clear error at the call site instead of somewhere inside a workload.

---

## Control as Request Data

`ClusterControlRequest` (`testing-framework/core/src/scenario/provisioning.rs`) has two values:

| Value | Meaning |
|---|---|
| `ClusterControlRequest::None` | Clients only; no lifecycle operations on the handle |
| `ClusterControlRequest::Full` | The provisioner attaches its node-control surface to the handle |

A raw `ClusterRequest` defaults to `None`. `ClusterApp` defaults to `Full`, because a cluster deployed as the system under test usually wants restarts available; opt out with `.with_control(ClusterControlRequest::None)`:

```rust,ignore
let scenario = AppHost::scenario()
    .with_app(
        ClusterApp::<QueueEnv>::new(QueueTopology::new(3))
            .with_control(ClusterControlRequest::None),   // clients only
    )
    .build()?;
```

What `Full` buys depends on the backend: local grants the complete surface including `StartNodeOptions`, compose grants restart (plus stop for attached clusters), and k8s grants start/stop/restart by scaling per-node deployments. The [Backend Capability Matrix](capability-matrix.md) records the details.

Every handle also reports a `ClusterControlProfile` describing who owns the lifecycle: `FrameworkManaged` (provisioned eagerly, torn down by the framework), `ManualControlled` (provisioned on demand, your code starts nodes), `ExistingClusterAttached`, or `ExternalUncontrolled`. `framework_owns_lifecycle()` is true only for `FrameworkManaged`.

---

## Node Control Without ManualCluster

Restarting nodes from a declarative workload does **not** require `ManualCluster`. The handle carries the control surface:

```rust,ignore
async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
    let cluster = ctx.require_app::<ClusterHandle<QueueEnv>>()?;

    cluster.restart_node("node-1").await?;
    cluster.wait_node_ready("node-1").await?;
    Ok(())
}
```

When control was not requested (or the backend cannot provide an operation), the call returns an error such as `"cluster node control is not available"` — explicit partial support at run time. [Chaos and Controlled Failure](chaos.md) shows full restart scenarios built this way, including the reusable `ClusterRestartChaos` workload. `ManualCluster` remains the imperative API for tests that control the entire node lifecycle themselves; see [ManualCluster: Imperative Node Control](manual-cluster.md).

### The Handle's Control Surface

`ClusterHandle<E>` forwards these operations to the backend's `NodeControlHandle` (`testing-framework/core/src/scenario/control.rs`):

| Method | Effect |
|--------|--------|
| `restart_node(name)` | Stop and start a named node |
| `restart_node_with(name, options)` | Restart with `StartNodeOptions` overrides |
| `start_node(name)` | Start a node, returning `StartedNode<E>` |
| `start_node_with(name, options)` | Start with overrides |
| `stop_node(name)` | Stop a named node |
| `wait_node_ready(name)` | Wait for one named node's readiness gate |
| `node_client(name)` | Current client for a node, if any |
| `node_pid(name)` | OS pid where applicable |

`StartedNode<E>` is a plain pair: the node `name` and a fresh `E::NodeClient`.

`wait_network_ready()` is the matching cluster-wide wait, backed by the provisioner's `ClusterWaitHandle`. Inside workloads, prefer waiting on observed application state instead.

### Start Modes

`ClusterStartMode` selects when managed nodes start:

- `Eager` (default): the provisioner starts and readiness-gates every node before the handle is returned.
- `OnDemand`: the cluster is prepared but no node runs; the handle's profile is `ManualControlled` and your code calls `start_node` explicitly. Supported by the local and k8s provisioners; the compose provisioner rejects it.

```rust,ignore
ClusterApp::<QueueEnv>::new(QueueTopology::new(3))
    .with_start_mode(ClusterStartMode::OnDemand)
```

### StartNodeOptions

`StartNodeOptions<E>` customizes a dynamic start or restart. Overview (full treatment in Part IV: [Ports, Peers, Node Config, and Readiness](node-config.md) and [Persistence, Snapshots, and Recovery Testing](persistence.md)):

| Field | Builder method | Purpose |
|-------|----------------|---------|
| `peers: Option<PeerSelection>` | `with_peers` | `DefaultLayout`, `None`, or `Named(vec)` |
| `config_override: Option<E::NodeConfig>` | `with_config_override` | Replace the generated config |
| `config_patch` | `create_patch(fn)` | Transform the generated config before spawn |
| `persist_dir: Option<PathBuf>` | `with_persist_dir` | Place the working directory at a findable location ([Persistence](persistence.md)) |
| `snapshot_dir: Option<PathBuf>` | `with_snapshot_dir` | Seed the working dir from a snapshot |
| `args: Vec<String>` | `with_args` | Extra process arguments |
| `runtime.start_timeout` | `with_runtime` / `with_start_timeout` | Readiness timeout override |

The local backend honors all of them; k8s rejects persist/snapshot dirs and routes config overrides through cfgsync where the environment supports it ([Kubernetes Backend](deployer-k8s.md)).

---

## Observability as Request Data

`ObservabilityInputs` (`testing-framework/core/src/scenario/observability.rs`) carries optional telemetry endpoints:

| Field | Purpose |
|---|---|
| `metrics_query_url` | Base URL the run uses to query Prometheus |
| `metrics_otlp_ingest_url` | OTLP HTTP endpoint nodes export metrics to |
| `grafana_url` | Grafana URL surfaced in logs and endpoint printouts |

Set it per cluster:

```rust,ignore
let observability = ObservabilityInputs {
    metrics_query_url: Some(Url::parse("http://127.0.0.1:19091")?),
    ..ObservabilityInputs::default()
};

let app = ClusterApp::<MetricsCounterEnv>::new(MetricsCounterTopology::new(3))
    .with_observability(observability);
```

The compose and k8s provisioners resolve the effective inputs by reading the `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL`, `LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL`, and `LOGOS_BLOCKCHAIN_GRAFANA_URL` environment variables and applying the request's values as overrides (request wins). The OTLP ingest URL flows into node config preparation; the resolved inputs are stored on the handle. The local provisioner does not populate observability, so local handles report empty metrics.

Expectations read metrics through the handle:

```rust,ignore
let cluster = ctx.require_app::<ClusterHandle<MetricsCounterEnv>>()?;
let metrics = cluster.metrics()?;             // Prometheus-backed when configured
let total = metrics.counter_value("sum(metrics_counter_increments_total)")?;
```

`metrics()` builds a Prometheus-backed `Metrics` handle from `metrics_query_url`, or an empty handle when none is configured (`is_configured()` distinguishes the two). Details, including what telemetry is and is not, are in [Telemetry and External Observability](telemetry.md).

---

## See Also

- [Chaos and Controlled Failure](chaos.md) — node control from workloads
- [ManualCluster: Imperative Node Control](manual-cluster.md) — the imperative alternative
- [Telemetry and External Observability](telemetry.md) — observability inputs in use
- [Backend Capability Matrix](capability-matrix.md) — backend support by feature

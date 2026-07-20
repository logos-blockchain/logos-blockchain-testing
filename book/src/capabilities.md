# Scenario Capabilities

Capabilities record, in the type system, which deployer services a scenario requests, such as node control or external telemetry. Unsupported combinations fail during construction, compilation, or deployment rather than during a workload.

---

## The Capability Type Parameter

The core builder is generic over a capability marker: `Builder<E, Caps>` with `Caps = ()` by default. Building a scenario produces `Scenario<E, Caps>`, and deployers are typed as `Deployer<E, Caps>`, so a deployer that cannot provide a capability does not accept scenarios that demand it.

The public wrappers (`testing-framework/core/src/scenario/definition/builder.rs`):

| Builder type | Capability | Entered via |
|--------------|-----------|-------------|
| `ScenarioBuilder<E>` | `()` | `ScenarioBuilder::with_deployment(...)` / `::new(provider)` |
| `NodeControlScenarioBuilder<E>` | `NodeControlCapability` | `.with_node_control()` (alias `.enable_node_control()`) |
| `ObservabilityScenarioBuilder<E>` | `ObservabilityCapability` | `.with_observability()` or any `ObservabilityBuilderExt` method |

All three expose the same fluent surface (`with_workload`, `with_expectation`, `with_run_duration`, ...), so the capability switch can happen anywhere in the chain:

```rust,ignore
let scenario = ScenarioBuilder::with_deployment(topology)
    .with_node_control()                 // () -> NodeControlCapability
    .with_workload(my_restart_workload)
    .with_run_duration(Duration::from_secs(60))
    .build()?;
```

`RequiresNodeControl` (`testing-framework/core/src/scenario/capabilities.rs`) is how `build()` and deployers reason about the marker:

```rust,ignore
pub trait RequiresNodeControl {
    const REQUIRED: bool;
}
// (): false    NodeControlCapability: true    ObservabilityCapability: false
```

`build()` uses it to validate the source configuration: a scenario that requires node control but only has external, uncontrolled nodes fails with a `SourceConfiguration` error ("node control is not available for cluster mode 'external-only' ..."). See [Existing and External Clusters](external-clusters.md).

---

## Node Control Without ManualCluster

Restarting nodes from a declarative workload does **not** require `ManualCluster`. The node-control capability provides access instead:

1. Call `.with_node_control()` on the builder.
2. Deploy with a deployer that supports the capability (local ships full node control, compose supports restart; the k8s deployer wires no node control handle into managed scenarios, so use its `ManualCluster` mode instead).
3. Inside a workload, take the handle from the context:

```rust,ignore
let Some(control) = ctx.node_control() else {
    return Err("this workload requires node control".into());
};

control.restart_node("node-1").await?;
```

`ManualCluster` is the imperative API for tests that control the entire node lifecycle themselves; see [ManualCluster: Imperative Node Control](manual-cluster.md). The scenario form above runs workloads, expectations, and teardown through the scenario runtime. [Chaos and Controlled Failure](chaos.md) shows a full failover scenario built this way.

### NodeControlHandle

`NodeControlHandle` (`testing-framework/core/src/scenario/control.rs`) is the deployer-agnostic control surface. Every method has a default implementation returning a "not supported by this deployer" error, so partial support is explicit at run time:

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

`ClusterWaitHandle<E>` is the matching wait surface: a single `wait_network_ready()` used for readiness gates. It is exposed publicly on the runner as `Runner::wait_network_ready()` (before `run` starts) and on `ManualCluster`; inside workloads, prefer waiting on observed application state instead.

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

---

## The Observability Capability

`ObservabilityCapability` carries optional telemetry endpoints (Prometheus query URL, OTLP ingest URL, Grafana URL). It does not require node control and is populated through `ObservabilityBuilderExt` (`testing-framework/core/src/scenario/builder_ext.rs`):

```rust,ignore
use testing_framework_core::scenario::ObservabilityBuilderExt;

let builder = ScenarioBuilder::with_deployment(topology)
    .with_metrics_query_url_str("http://127.0.0.1:9090");
```

Each method transitions `ScenarioBuilder<E>` into `ObservabilityScenarioBuilder<E>` (and is a plain setter if you are already there). `Url`-typed, `_str` (panicking), and `try_..._str` (fallible) variants exist for all three endpoints. Deployers merge these values with environment variables; the details, including what telemetry is and is not, are in [Telemetry and External Observability](telemetry.md).

Capabilities use one marker per scenario, not a set. Choosing `with_node_control()` gives the scenario node control; choosing an observability method supplies telemetry endpoints. Each deployer declares which `Caps` it supports; the [Capability Matrix](capability-matrix.md) lists the available combinations.

---

## See Also

- [Chaos and Controlled Failure](chaos.md) — node control from workloads
- [ManualCluster: Imperative Node Control](manual-cluster.md) — the imperative alternative
- [Telemetry and External Observability](telemetry.md) — the observability capability in use
- [Capability Matrix](capability-matrix.md) — deployer support by capability

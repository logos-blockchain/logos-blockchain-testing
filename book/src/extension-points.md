# Public Extension Points

This chapter lists every trait you implement to plug your application into the framework.

The framework never imports your application. It defines public traits that your integration crate implements and calls them at defined points in the run lifecycle. Each entry below links to the corresponding chapter.

| Trait | Defined in | You implement it to... | Taught in |
|---|---|---|---|
| `Application` | `testing-framework-core` (`env.rs`) | Bundle your deployment, client, and config types | [Implementing Application](implementing-application.md) |
| `DeploymentProvider<D>` | `testing-framework-core` (`topology`) | Build a deployment plan, optionally from a seed | [Topology and Deployment Plans](topology.md) |
| `Workload<E>` | `testing-framework-core` (`scenario`) | Drive traffic against the running system | [Workloads and Concurrency](workloads.md) |
| `Expectation<E>` | `testing-framework-core` (`scenario`) | Define what success means | [Expectations and Evaluation](expectations.md) |
| `RuntimeExtensionFactory<E>` | `testing-framework-core` (`scenario`) | Prepare a shared runtime value before workloads start | [Runtime Extensions](runtime-extensions.md) |
| `Observer` | `testing-framework-core` (`observation`) | Continuously materialize app state | [Continuous Observation](observation.md) |
| `SourceProvider<S>` | `testing-framework-core` (`observation`) | Supply the current observation source set | [Continuous Observation](observation.md) |
| `SourceProviderFactory<E, S>` | `testing-framework-core` (`observation`) | Build a source provider once node clients exist | [Continuous Observation](observation.md) |
| `AppDeployment<E, P>` | `testing-framework-app` | Prepare one composable application preset | [AppDeployment and DeployContext](app-deployment.md) |
| `Deployer<E, Caps>` | `testing-framework-core` (`scenario::runtime`) | Provision a scenario into a target environment | [Part V](part-v.md) |
| `NodeControlHandle<E>` | `testing-framework-core` (`scenario`) | Expose start/stop/restart of nodes at runtime | [Scenario Capabilities](capabilities.md) |
| `ClusterWaitHandle<E>` | `testing-framework-core` (`scenario`) | Expose cluster readiness waits | [Scenario Capabilities](capabilities.md) |
| `ObservabilityCapabilityProvider` | `testing-framework-core` (`scenario`) | Surface telemetry endpoints from capability markers | [Telemetry and External Observability](telemetry.md) |
| `BinaryProvider` | `testing-framework-runner-local` (`binary`) | Resolve the node executable for local processes | [Binary Providers](binary-providers.md) |
| `DownloadProcessor` | `testing-framework-runner-local` (`binary`) | Turn a downloaded artifact into an executable | [Binary Providers](binary-providers.md) |
| `IntoExistingCluster` | `testing-framework-core` (`scenario::sources`) | Convert a value into an existing-cluster descriptor | [Existing and External Clusters](external-clusters.md) |

---

## Environment and Topology

**`Application`** is the root of every integration. It bundles the backend-specific types the scenario engine is generic over: a deployment descriptor, a node client, and a node config. The three methods have working defaults: override `external_node_client` to support external sources, `build_node_client` to support deployer-discovered nodes, and `node_readiness_path` when your health endpoint is not `/`.

```rust,ignore
#[async_trait]
pub trait Application: Send + Sync + 'static {
    type Deployment: DeploymentDescriptor + Clone + 'static;
    type NodeClient: Clone + Send + Sync + 'static;
    type NodeConfig: Clone + Send + Sync + 'static;

    fn external_node_client(source: &ExternalNodeSource) -> Result<Self::NodeClient, DynError>;
    fn build_node_client(access: &NodeAccess) -> Result<Self::NodeClient, DynError>;
    fn node_readiness_path() -> &'static str; // default "/"
}
```

Plugs in as the `E` type parameter of `ScenarioBuilder<E>`, `Workload<E>`, `Expectation<E>`, and every deployer.

**`DeploymentProvider<D>`** builds the deployment descriptor a scenario runs against, optionally driven by a `DeploymentSeed` for reproducible generation. `ScenarioBuilder::new` accepts one; `ScenarioBuilder::with_deployment` wraps a fixed value in the built-in `FixedDeploymentProvider`.

```rust,ignore
pub trait DeploymentProvider<D: DeploymentDescriptor>: Send + Sync {
    fn build(&self, seed: Option<&DeploymentSeed>) -> Result<D, DynTopologyError>;
}
```

---

## Scenario Behavior

**`Workload<E>`** describes an action sequence executed during the run. `start` receives the `RunContext<E>` (node clients, extensions, run metrics) and runs concurrently with other workloads. A workload can bundle its own checks via `expectations()`.

```rust,ignore
#[async_trait]
pub trait Workload<E: Application>: Send + Sync {
    fn name(&self) -> &str;
    fn expectations(&self) -> Vec<Box<dyn Expectation<E>>> { Vec::new() }
    fn init(&mut self, descriptors: &E::Deployment, metrics: &RunMetrics) -> Result<(), DynError> { Ok(()) }
    async fn start(&self, ctx: &RunContext<E>) -> Result<(), DynError>;
}
```

Registered with `with_workload` / `with_workload_boxed` on the builder.

**`Expectation<E>`** defines a check evaluated during or after the run. `start_capture` records a baseline, `check_during_capture` is the optional fail-fast hook polled during the run, and `evaluate` delivers the verdict at the end.

```rust,ignore
#[async_trait]
pub trait Expectation<E: Application>: Send + Sync {
    fn name(&self) -> &str;
    fn init(&mut self, descriptors: &E::Deployment, metrics: &RunMetrics) -> Result<(), DynError> { Ok(()) }
    async fn start_capture(&mut self, ctx: &RunContext<E>) -> Result<(), DynError> { Ok(()) }
    async fn check_during_capture(&mut self, ctx: &RunContext<E>) -> Result<(), DynError> { Ok(()) }
    async fn evaluate(&mut self, ctx: &RunContext<E>) -> Result<(), DynError>;
}
```

Registered with `with_expectation` / `with_expectation_boxed`.

**`RuntimeExtensionFactory<E>`** prepares one typed value after deployment (node clients are available) and before workloads start. The value is stored by `TypeId` and retrieved in workloads via `ctx.extension::<T>()` / `ctx.require_extension::<T>()`. Return `PreparedRuntimeExtension::new(value)`, `::with_cleanup(value, guard)`, or `::from_task(value, join_handle)` to tie a background task's lifetime to the run.

```rust,ignore
#[async_trait]
pub trait RuntimeExtensionFactory<E: Application>: Send + Sync {
    async fn prepare(
        &self,
        deployment: &E::Deployment,
        node_clients: NodeClients<E>,
    ) -> Result<PreparedRuntimeExtension, DynError>;
}
```

Registered with `with_runtime_extension_factory`. Registering two factories that produce the same extension type fails at prepare time with `duplicate runtime extension type registered`.

---

## Observation

**`Observer`** owns the app-side logic of the continuous observation runtime: `init` builds retained state from the source set, `poll` advances it each cycle and emits delta events, `snapshot` renders the current view. The runtime handles scheduling, history, and error tracking.

```rust,ignore
#[async_trait]
pub trait Observer: Send + Sync + 'static {
    type Source: Clone + Send + Sync + 'static;
    type State: Send + Sync + 'static;
    type Snapshot: Clone + Send + Sync + 'static;
    type Event: Clone + Send + Sync + 'static;

    async fn init(&self, sources: &[ObservedSource<Self::Source>]) -> Result<Self::State, DynError>;
    async fn poll(&self, sources: &[ObservedSource<Self::Source>], state: &mut Self::State)
        -> Result<Vec<Self::Event>, DynError>;
    fn snapshot(&self, state: &Self::State) -> Self::Snapshot;
}
```

**`SourceProvider<S>`** returns the current source set before each cycle, which lets the observed population change mid-run. Use `StaticSourceProvider` for a fixed set.

```rust,ignore
#[async_trait]
pub trait SourceProvider<S>: Send + Sync + 'static {
    async fn sources(&self) -> Result<Vec<ObservedSource<S>>, DynError>;
}
```

**`SourceProviderFactory<E, S>`** builds the provider once node clients exist. Any `Fn(&E::Deployment, NodeClients<E>) -> Result<BoxedSourceProvider<S>, DynError>` closure implements it. All three plug into a scenario through `ObservationExtensionFactory<E, O>`, which is itself a `RuntimeExtensionFactory`; see `examples/openraft_kv/testing/integration/src/observation.rs` for a complete implementation.

---

## Application Composition

**`AppDeployment<E, P>`** prepares one reusable application preset, such as a process, child cluster, or composed stack, and returns a typed access or control handle. Framework adapters register managed resource lifetime separately with scenario cleanup. `AppHandle` is blanket-implemented for any `Clone + Send + Sync + 'static` type.

```rust,ignore
#[async_trait]
pub trait AppDeployment<E, P = LocalClusterProvisioner>: Send + 'static
where
    E: Application,
{
    type Handle: AppHandle;
    async fn deploy(self, ctx: &mut DeployContext<E, P>) -> Result<Self::Handle, DynError>;
}
```

Registered with `AppScenarioBuilderExt::with_app`, which wraps it in an `AppDeploymentFactory` (a `RuntimeExtensionFactory`). Compose children inside `deploy` via `ctx.deploy(...)` / `ctx.deploy_and_expose(...)`. See [Handle Ownership and Teardown](handles-teardown.md) for handle access and cleanup semantics.

---

## Deployment Backends

**`Deployer<E, Caps>`** is the contract every backend implements: turn a built `Scenario` into a `Runner<E>`. `ProcessDeployer` (local), `ComposeDeployer`, and `K8sDeployer` are the in-repo implementations; `Caps` carries capability markers such as `NodeControlCapability`.

```rust,ignore
#[async_trait]
pub trait Deployer<E: Application, Caps = ()>: Send + Sync {
    type Error;
    async fn deploy(&self, scenario: &Scenario<E, Caps>) -> Result<Runner<E>, Self::Error>;
}
```

**`NodeControlHandle<E>`** is the deployer-agnostic control surface behind node-control scenarios: `start_node(_with)`, `stop_node`, `restart_node(_with)`, `wait_node_ready`, `node_client`, and `node_pid`. Every method has a default that returns a "not supported by this deployer" error, so backends implement only what they support. **`ClusterWaitHandle<E>`** provides the cluster-wide `wait_network_ready` operation. Both are combined by `ManualClusterHandle<E>` in `core::runtime::manual`, the interface behind [ManualCluster](manual-cluster.md).

**`ObservabilityCapabilityProvider`** lets deployers read telemetry endpoints out of whatever capability marker a scenario was built with; it is implemented for `()`, `NodeControlCapability`, and `ObservabilityCapability`. You only implement it when defining a new capability marker type.

---

## Local Binary Resolution

**`BinaryProvider`** resolves the executable path for a locally spawned node process. Implementations return `Ok(None)` when valid but unable to resolve, which is how `FallbackBinaryProvider` chains providers. The default `resolve` caches per process by `cache_key`.

```rust,ignore
pub trait BinaryProvider: Send + Sync {
    fn try_resolve(&self) -> Result<Option<PathBuf>, BinaryProviderError>;
    fn display(&self) -> String;
    fn cache_key(&self) -> String;
    // provided: resolve(), resolve_uncached()
}
```

Built-in implementations: `PathBinaryProvider`, `EnvBinaryProvider`, `BuildBinaryProvider`, `DownloadBinaryProvider`, `FallbackBinaryProvider`. **`DownloadProcessor`** post-processes a checksum-verified download (for example, unpacking an archive) into the executable; `DownloadProcessorFn` adapts a closure with a stable `cache_key` so changed preparation logic invalidates the cache.

```rust,ignore
pub trait DownloadProcessor: Send + Sync {
    fn process(&self, artifact: &Path, output: &Path) -> Result<(), DownloadProcessorError>;
    fn cache_key(&self) -> &str;
}
```

---

## Attaching Sources

**`IntoExistingCluster`** converts a value into the typed `ExistingCluster` descriptor accepted by `with_existing_cluster_from`. It is implemented for `ExistingCluster` and `&ExistingCluster`; implement it for your own environment-selection types to keep attach logic in one place. External endpoints use `ExternalNodeSource` values directly and pair with `Application::external_node_client`.

The required extension points depend on the entry pattern: a uniform managed cluster needs `Application` and the scenario traits, an AppHost stack adds `AppDeployment`, and attached clusters add the source traits. See [Choosing an Entry Pattern](entry-patterns.md). For where each implementation should live, see [Framework vs Application Boundaries](tf-boundaries.md) and the crate-level view in [Crate and API Map](crate-map.md).

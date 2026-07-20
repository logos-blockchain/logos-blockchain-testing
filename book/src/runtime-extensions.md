# Runtime Extensions

Runtime extensions are typed values prepared once per run (after nodes exist, before workloads start) and handed to workloads and expectations through the `RunContext`. The app layer and the observation runtime are built on them.

---

## The Mechanism

The implementation is in `testing-framework/core/src/scenario/runtime/extensions.rs` and has three parts:

**1. A factory registered on the builder.** `RuntimeExtensionFactory<E>` is called by the deployer during preparation, when the deployment is resolved and node clients are available:

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

Register it with `.with_runtime_extension_factory(Box::new(factory))`. Factories run in registration order; any `prepare` error aborts the deployment.

The factory runs after the deployment is resolved and node clients exist. It prepares the value once so every workload and expectation can share it instead of rebuilding the same clients or polling loops.

**2. A prepared value, optionally with cleanup.** `PreparedRuntimeExtension` wraps one value of any `Clone + Send + Sync + 'static` type, with three constructors:

| Constructor | Use when |
|-------------|----------|
| `PreparedRuntimeExtension::new(value)` | The value needs no teardown |
| `PreparedRuntimeExtension::with_cleanup(value, guard)` | Custom teardown via a `CleanupGuard` |
| `PreparedRuntimeExtension::from_task(value, join_handle)` | The value is fed by a background Tokio task; the task is aborted at teardown |

Cleanup guards are collected into the run's cleanup chain and execute at teardown in reverse registration order; see [Handle Ownership and Teardown](handles-teardown.md) for how the app layer separates those guards from handle access.

**3. Typed retrieval from the context.** The prepared values land in a type-indexed store inside `RunContext`:

```rust,ignore
// Somewhere in a workload or expectation:
let handle: MyHandle = ctx.require_extension::<MyHandle>()?;
// or, tolerating absence:
let maybe: Option<MyHandle> = ctx.extension::<MyHandle>();
```

`extension::<T>()` returns a *clone* of the stored value. Extension values should therefore be cheap to clone, typically by wrapping shared state in an `Arc`.

---

## One Value Per Type

The store is keyed by `TypeId`. Registering two extensions that prepare the same type is a hard error at prepare time:

```text
duplicate runtime extension type registered: <type name>
```

Because `ctx.extension::<T>()` returns one value by type, each type may be registered only once. To register several values with the same underlying shape, wrap them in distinct newtypes or, in the app layer, use named handles instead (see [AppDeployment and DeployContext](app-deployment.md)).

This rule is why a scenario allows only one `with_app(...)`: the app layer registers its `AppRuntime` extension per call, and a second registration collides. Compose multiple applications inside one root `AppDeployment` instead; see [AppHost and with_app](app-host.md).

---

## Writing a Factory

A minimal factory that shares a client wrapper with all workloads:

```rust,ignore
use async_trait::async_trait;
use testing_framework_core::scenario::{
    DynError, NodeClients, PreparedRuntimeExtension, RuntimeExtensionFactory,
};

#[derive(Clone)]
struct FrontDoor(MyNodeClient);

struct FrontDoorFactory;

#[async_trait]
impl RuntimeExtensionFactory<MyEnv> for FrontDoorFactory {
    async fn prepare(
        &self,
        _deployment: &<MyEnv as Application>::Deployment,
        node_clients: NodeClients<MyEnv>,
    ) -> Result<PreparedRuntimeExtension, DynError> {
        let client = node_clients
            .snapshot()
            .first()
            .cloned()
            .ok_or("no nodes available")?;

        Ok(PreparedRuntimeExtension::new(FrontDoor(client)))
    }
}

// Registration:
let builder = builder.with_runtime_extension_factory(Box::new(FrontDoorFactory));
```

For an extension backed by a polling loop, spawn the task in `prepare` and return `from_task(handle, join_handle)`. The runner aborts the task when the run tears down, so the loop cannot outlive the cluster. The observation runtime works this way. The pubsub example registers its feed this way (`examples/pubsub/testing/integration/src/scenario.rs`):

```rust,ignore
self.with_runtime_extension_factory(Box::new(PubSubTopicFeedFactory::new(topic)))
```

---

## What Is Built on This

The following layers use runtime extension factories:

| Layer | Factory | Extension value in `RunContext` |
|-------|---------|--------------------------------|
| App layer | `AppDeploymentFactory` (via `with_app`) | `AppRuntime` + exposed app handles |
| Observation | `ObservationExtensionFactory` (via `with_observer`) | `ObservationHandle<O>` |

So when a workload calls `ctx.require_app::<KvStoreCluster>()` or `ctx.require_extension::<ObservationHandle<OpenRaftClusterObserver>>()`, it is walking the same type-indexed store described above.

- App layer: [AppHost and with_app](app-host.md)
- Observation runtime: [Continuous Observation](observation.md)

---

## See Also

- [Workloads and Concurrency](workloads.md) — consuming extensions from workloads
- [Continuous Observation](observation.md) — an extension backed by a polling task
- [Handle Ownership and Teardown](handles-teardown.md) — cleanup ordering in depth

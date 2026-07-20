# AppDeployment and DeployContext

Application repositories implement `AppDeployment` for deployable components. The implementation uses `DeployContext` to deploy children, expose handles, and register managed resources with scenario cleanup.

The framework runs deployments, stores handles, and performs teardown without defining application binaries or clients. The application crate decides which components start and which typed handles workloads receive.

---

## The Trait Contract

```rust,ignore
#[async_trait]
pub trait AppDeployment<E: Application, P = LocalClusterProvisioner>: Send + 'static {
    type Handle: AppHandle;

    async fn deploy(self, ctx: &mut DeployContext<E, P>) -> Result<Self::Handle, DynError>;
}
```

The trait has these properties:

- **`deploy` consumes `self`.** A deployment value describes one preparation attempt and returns its runtime access handle.
- **The handle is typed.** `Handle` can be any `Clone + Send + Sync + 'static` type. Managed lifetime is registered separately; see [Handle Ownership and Teardown](handles-teardown.md).
- **`Clone` is required by the factory.** `with_app` needs `A: AppDeployment<E> + Clone + Sync` because `AppDeploymentFactory` clones the description on each `prepare`. Deployment structs should contain configuration such as node counts, ports, and paths rather than live resources.

A minimal implementation, from the kvstore example:

```rust,ignore
// examples/kvstore/testing/integration/src/app.rs
#[derive(Clone)]
pub struct KvLocalApp {
    deployment: KvTopology,
}

#[async_trait]
impl AppDeployment<AppHostEnv> for KvLocalApp {
    type Handle = LocalAppCluster<KvEnv>;

    async fn deploy(self, ctx: &mut DeployContext<AppHostEnv>) -> Result<Self::Handle, DynError> {
        ctx.deploy_local_cluster::<KvEnv>(self.deployment).await
    }
}
```

---

## DeployContext API

One context belongs to one scenario preparation. It carries the active cluster provisioner, outer deployment and clients, exposed handles, and a cleanup stack. Routing every managed child through this context registers cleanup as soon as the resource is acquired, including when deployment fails partway.

| Method | Purpose |
|--------|---------|
| `deploy(app)` | Runs a child deployment, returns its handle. Does **not** expose it. |
| `deploy_and_expose(app)` | Runs a child deployment and exposes a clone of its handle. |
| `expose(handle)` | Registers the default (unnamed) handle for its concrete type. |
| `expose_named(name, handle)` | Registers a named handle; allows several instances of one type. |
| `get::<T>()` / `get_named::<T>(name)` | `Option<T>` clone of an exposed handle. |
| `require::<T>()` / `require_named::<T>(name)` | `Result<T, AppDeployError>` — typed missing-handle error. |
| `contains::<T>()` | Whether a default handle for `T` is exposed. |
| `handles()` | Borrows the registry of handles exposed so far. |
| `deployment()` | The outer scenario deployment descriptor (`E::Deployment`). |
| `node_clients()` | Clients for nodes owned by the outer scenario (`NodeClients<E>`). |
| `deploy_cluster::<App>(request)` | Provisions a managed, attached, or external cluster through the active provisioner. |
| `deploy_local_cluster::<App>(deployment)` | Convenience for an eager managed cluster with the active provisioner. |

`deploy` does not expose its returned handle. Use it when only the parent needs the child handle. Use `deploy_and_expose` when workloads should also be able to request the child directly. Both `expose` and `expose_named` return `AppDeployError::DuplicateHandle` if the type or type/name pair is already registered.

---

## Nested Deployments

A deployment composes children by calling `ctx.deploy(...)` or `ctx.deploy_and_expose(...)` on other `AppDeployment` values. The parent decides what is visible:

```rust,ignore
#[async_trait]
impl AppDeployment<TestEnv> for ParentApp {
    type Handle = ParentHandle;

    async fn deploy(self, ctx: &mut DeployContext<TestEnv>) -> Result<Self::Handle, DynError> {
        let child = ctx.deploy(ChildApp).await?; // child handle NOT exposed
        let parent = ParentHandle { child };

        ctx.expose(parent.clone())?; // only the parent is visible

        Ok(parent)
    }
}
```

Workloads can then require `ParentHandle` but not `ChildHandle`: the child stays an implementation detail. Its managed resources remain registered with scenario cleanup whether or not the returned handle is exposed or embedded. Expose the child too when workloads legitimately need it.

```mermaid
flowchart TD
    Root[Root AppDeployment] -->|deploy| C1[Child A]
    Root -->|deploy_and_expose| C2[Child B]
    Root -->|expose| RH[Root handle]
    C2 --> BH[Child B handle]
    RH --> W[Workloads]
    BH --> W
    RH:::hd
    BH:::hd
    W:::sc
    classDef hd stroke:#4caf7d,stroke-width:2.5px;
    classDef sc stroke:#9b6dd6,stroke-width:2.5px;
```

---

## The Outer Scenario: deployment() and node_clients()

For `AppHost` scenarios, `deployment()` is the empty `AppHostTopology` and `node_clients()` is empty; everything lives in your handles. On a regular uniform-cluster scenario, they are how an app preset wraps the managed cluster itself:

```rust,ignore
// examples/kvstore/testing/integration/src/app.rs
#[async_trait]
impl AppDeployment<KvEnv> for KvExistingClusterApp {
    type Handle = KvStoreCluster;

    async fn deploy(self, ctx: &mut DeployContext<KvEnv>) -> Result<Self::Handle, DynError> {
        Ok(KvStoreCluster::new(
            ctx.deployment().clone(),
            ctx.node_clients().clone(),
        ))
    }
}
```

This preset does not launch nodes. It returns typed access to the nodes already managed by the scenario.

---

## Root-Handle Auto-Exposure

After the root deployment returns, `AppDeploymentFactory` checks `ctx.contains::<A::Handle>()`. If the root handle type is not already exposed, the factory exposes the returned handle as the default for its type. So:

- A simple root app can just `return Ok(handle)` and workloads can `require_app::<Handle>()` with no explicit `expose`.
- A root app that already exposed its own handle (like the stack apps in [Composing Heterogeneous Stacks](composing-stacks.md)) is left alone, so there is no duplicate error.

---

## See Also

- [AppHost and with_app](app-host.md): how a deployment gets registered and prepared.
- [Handle Ownership and Teardown](handles-teardown.md): what exposure means for resource lifetime.
- [One Binary: LocalProcessApp](local-process-app.md), [Uniform Child Clusters: LocalAppCluster](local-app-cluster.md): ready-made deployments to compose.

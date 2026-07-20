# Shared Cluster Provisioning

App composition and uniform scenarios share a cluster-provisioning model. A request describes the cluster source and required behavior. A provisioner returns a backend-independent `ClusterHandle<E>` and registers any managed lifetime with the app cleanup stack.

---

## One Request, Three Sources

`ClusterRequest<E>` separates what the test needs from how a backend supplies it:

```rust,ignore
let managed = ClusterRequest::<QueueEnv>::managed(QueueTopology::new(3));
let attached = ClusterRequest::<QueueEnv>::attached(existing_cluster);
let external = ClusterRequest::<QueueEnv>::external(node_sources);
```

| Source | Nodes started by the framework | Clients | Node control | Framework teardown |
|---|---:|---:|---:|---:|
| `Managed` | Yes, unless start mode is on demand | Yes | When requested and supported | Yes |
| `Attached` | No | Yes | According to the attached cluster's control profile | Only resources the framework itself acquires |
| `External` | No | Yes | No | No |

Managed and attached sources can also include external nodes with `with_external_nodes(...)`. This is useful when one logical cluster combines framework-visible nodes from more than one source.

---

## Requesting Behavior

The request carries requirements that are meaningful across backends:

| Method | Meaning |
|---|---|
| `with_policy(policy)` | Apply readiness, retry, cleanup, and network-control policy. |
| `with_start_mode(Eager)` | Start managed nodes while provisioning. This is the default. |
| `with_start_mode(OnDemand)` | Prepare a managed cluster but let test code start nodes explicitly. |
| `with_control(Full)` | Require the node-control surface on the returned handle. |
| `with_network_control()` | Require backend network control. |
| `with_network_recovery(recovery)` | Register application recovery after a network effect is released; also requests network control. |

Backends may support different combinations of these requirements. The [Capability Matrix](capability-matrix.md) records current coverage.

---

## Provisioning Inside an AppDeployment

`DeployContext` is parameterized by a `ClusterProvisioner`. Its `deploy_cluster` method is the app-layer entry point:

```rust,ignore
#[async_trait]
impl AppDeployment<AppHostEnv> for QueueApp {
    type Handle = ClusterHandle<QueueEnv>;

    async fn deploy(
        self,
        ctx: &mut DeployContext<AppHostEnv>,
    ) -> Result<Self::Handle, DynError> {
        ctx.deploy_cluster(ClusterRequest::<QueueEnv>::managed(self.topology))
            .await
    }
}
```

`DeployContext::deploy_cluster` requests the full common node-control surface because app workloads receive the returned cluster handle directly. The local convenience `deploy_local_cluster` expresses the common managed, eager case. Use `deploy_cluster` when ownership mode, start mode, or policy must be visible in the app definition.

`with_app(app)` selects the default local provisioner. `with_app_using(app, provisioner)` supplies another provisioner. The root deployment must implement `AppDeployment<E, P>` for that provisioner type; code written only as `AppDeployment<E>` uses the default local type.

---

## The Returned Handle

`ClusterHandle<E>` presents the common runtime surface:

- clients: `node_clients`, `clients`, `first_client`, `node_client`;
- cluster description: `deployment`, `node_count`, `control_profile`;
- node operations when present: `start_node`, `stop_node`, `restart_node`, `wait_node_ready`;
- cluster readiness: `wait_network_ready`;
- network effects when present: `network_control`.

Unavailable operations return an error or `None`; callers can inspect `control_profile()` when behavior depends on ownership mode.

The handle does not own managed lifetime. The provisioner returns a private cleanup guard alongside the runtime surfaces. `DeployContext` moves that guard into the scenario cleanup stack, which runs in reverse acquisition order on normal completion and partial deployment failure.

---

## Backend Boundary

`ClusterProvisioner<E>` has one operation:

```rust,ignore
#[async_trait]
pub trait ClusterProvisioner<E: Application>: Clone + Send + Sync + 'static {
    async fn provision_cluster(
        &self,
        request: ClusterRequest<E>,
    ) -> Result<ClusterUnit<E>, DynError>;
}
```

A backend implementation translates the request into concrete resources, clients, control adapters, readiness, and cleanup. `ClusterUnit<E>` carries these values from the provisioner; applications normally use the resulting `ClusterHandle<E>`.

The local provisioner currently supports managed and external sources. Attached support and equivalent Compose/Kubernetes app provisioners require backend implementations, but not another application-composition model.

---

## Relation to Other Entry Patterns

- A uniform scenario asks its deployer to provision the scenario's primary cluster.
- A composed stack asks its `DeployContext` to provision one or more child clusters.
- An attached or external test changes `ClusterSource`, while workloads keep using clients and available controls.
- `ManualCluster` uses local provisioning machinery directly and gives imperative code responsibility for sequencing.

The entry patterns differ in who describes and drives the test. They do not need separate definitions of what a cluster is, which controls it exposes, or who tears it down.

---

## See Also

- [Existing and External Clusters](external-clusters.md): declaring non-managed sources in ordinary scenarios.
- [AppDeployment and DeployContext](app-deployment.md): composing cluster and process children.
- [Handle Ownership and Teardown](handles-teardown.md): the lifetime boundary in detail.
- [Readiness, Retry, and Cleanup](deployment-policies.md): the policies carried by a request.

# Cluster Provisioning

Every node cluster in the framework is provisioned through one model. A `ClusterRequest<E>` describes the cluster source and required behavior; a backend `ClusterProvisioner<E>` turns it into a `ClusterUnit<E>`; applications and workloads use the resulting backend-neutral `ClusterHandle<E>`. `ClusterApp` is a thin builder over this model, and `ManualCluster` reuses the same lifecycle machinery imperatively.

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
| `Attached` | No | Yes | Backend-dependent (compose: restart + stop; k8s: none) | Only resources the framework itself acquires |
| `External` | No | Yes | No | No |

Managed and attached sources can also include external nodes with `with_external_nodes(...)`. This is useful when one logical cluster combines framework-visible nodes from more than one source. See [Existing and External Clusters](external-clusters.md) for the attach and external arms in detail.

---

## Requesting Behavior

The request carries requirements that are meaningful across backends:

| Method | Meaning |
|---|---|
| `with_policy(policy)` | Apply readiness, retry, and cleanup policy ([Readiness, Retry, and Artifact Preservation](deployment-policies.md)). |
| `with_start_mode(Eager)` | Start managed nodes while provisioning. This is the default. |
| `with_start_mode(OnDemand)` | Prepare a managed cluster but let test code start nodes explicitly. |
| `with_control(Full)` | Require the node-control surface on the returned handle. Requests default to `None`; `ClusterApp` requests `Full`. |
| `with_observability(inputs)` | Per-cluster telemetry endpoints; provisioners merge them over `LOGOS_BLOCKCHAIN_*` env vars ([Cluster Control and Observability](capabilities.md)). |

Backends support different combinations of these requirements — for example, compose rejects `OnDemand`. The [Backend Capability Matrix](capability-matrix.md) records current coverage.

---

## Provisioning Inside an AppDeployment

`DeployContext` is parameterized by a provisioner type. Its `deploy_cluster` method is the app-layer entry point:

```rust,ignore
#[async_trait]
impl AppDeployment<AppHostEnv> for QueueLocalApp {
    type Handle = ClusterHandle<QueueEnv>;

    async fn deploy(
        self,
        ctx: &mut DeployContext<AppHostEnv>,
    ) -> Result<Self::Handle, DynError> {
        ctx.deploy_cluster(ClusterRequest::<QueueEnv>::managed(self.deployment))
            .await
    }
}
```

The request is forwarded unchanged; runtime control is granted only when the request asks for it. `deploy_cluster` also moves the unit's cleanup guard into the scenario cleanup stack before returning the handle.

For the common cases, `ClusterApp<E>` builds the request for you — `ClusterApp::new(topology)` is a managed, eager, full-control request, with `with_policy`, `with_start_mode`, `with_control`, `with_external_nodes`, and `with_observability` as overrides — and is itself an `AppDeployment` whose handle is `ClusterHandle<E>`, so single-cluster scenarios need no custom deployment at all.

`with_app(app)` selects the default `LocalClusterProvisioner`. `with_app_using(app, provisioner)` supplies another provisioner; the root deployment must implement `AppDeployment<E, P>` for that provisioner type (code written only as `AppDeployment<E>` uses the default local type, while `ClusterApp` is generic over any `ClusterProvisioner<E>`).

---

## The Returned Handle

`ClusterHandle<E>` presents the common runtime surface:

- clients: `node_clients`, `clients`, `first_client`, `node_client(name)`, `node_pid(name)`;
- cluster description: `deployment`, `node_count`, `control_profile`;
- node operations when present: `start_node(_with)`, `stop_node`, `restart_node(_with)`, `wait_node_ready`;
- cluster readiness: `wait_network_ready`;
- telemetry: `observability()` and `metrics()`, which builds the Prometheus-backed `Metrics` handle from the cluster's observability inputs.

Unavailable operations return an error ("cluster node control is not available", "cluster readiness is not available") or `None`; callers can inspect `control_profile()` when behavior depends on ownership mode.

The handle does not own managed lifetime. The provisioner returns a cleanup guard inside the unit; `DeployContext` moves that guard into the scenario cleanup stack, which runs in reverse acquisition order on normal completion and partial deployment failure.

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

A backend implementation translates the request into concrete resources, clients, control adapters, readiness, and cleanup. `ClusterUnit<E>` carries those values from the provisioner — the node clients, a `ClusterControlProfile`, and optional node control, cluster wait, cleanup guard, and observability inputs — and `unit.handle()` derives the `ClusterHandle` applications actually use.

Three implementations ship:

| Provisioner | Crate | Notes |
|---|---|---|
| `LocalClusterProvisioner` | `testing-framework-runner-local` | The default; full node control, no attach |
| `ComposeProvisioner` | `testing-framework-runner-compose` | Also implements `ContainerStackProvisioner`, so one compose project holds node clusters and container services |
| `K8sClusterProvisioner` | `testing-framework-runner-k8s` | Managed units backed by the k8s `ManualCluster`; no container-stack support yet |

Heterogeneous container services use the separate `ContainerStackProvisioner` contract (`DeployContext::deploy_container_stack`) because their units are mixed containers, not one typed uniform cluster.

---

## Relation to Entry Patterns

- A single-cluster scenario wraps one request in a `ClusterApp`.
- A composed stack asks its `DeployContext` to provision one or more child clusters, each with its own request.
- An attached or external test changes the request's `ClusterSource`, while workloads keep using clients and available controls.
- `ManualCluster` uses the same provisioning machinery — on k8s it *is* the managed unit — and gives imperative code responsibility for sequencing; inside the model, `ClusterStartMode::OnDemand` provides the same lifecycle behind a `ClusterHandle`.

The entry patterns differ in who describes and drives the test. They do not need separate definitions of what a cluster is, which controls it exposes, or who tears it down.

---

## See Also

- [Existing and External Clusters](external-clusters.md): the attach and external request arms.
- [AppDeployment and DeployContext](app-deployment.md): composing cluster and process children.
- [Handle Ownership and Teardown](handles-teardown.md): the lifetime boundary in detail.
- [Readiness, Retry, and Artifact Preservation](deployment-policies.md): the policies carried by a request.

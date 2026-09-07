# Existing and External Clusters

Scenarios can run against nodes the framework did not deploy: an attached existing cluster, standalone external endpoints, or a mix.

Where a cluster's nodes come from is one field of its `ClusterRequest`. The three arms of `ClusterSource` cover every case: **managed** nodes the provisioner spawns, **attached** nodes discovered in an existing cluster, and **external** nodes named by static endpoints. Workloads and expectations see the same `ClusterHandle` either way.

---

## The Source Model

The source model uses these types from `testing-framework-core`:

| Type | Shape |
|---|---|
| `ClusterSource<E>` | `Managed { deployment, external }`, `Attached { cluster, external }`, or `External { nodes }` |
| `ExistingCluster` | Typed descriptor of a cluster to attach to — a k8s label selector (optionally namespaced) or a compose project (optionally with explicit services) |
| `IntoExistingCluster` | Conversion trait for building attach descriptors from other values |
| `ExternalNodeSource` | A label plus an endpoint string, e.g. `http://10.0.0.5:8080` |
| `ClusterControlProfile` | `FrameworkManaged`, `ExistingClusterAttached`, `ExternalUncontrolled`, `ManualControlled` |

`ExistingCluster` is constructed with `for_k8s_selector(selector)`, `for_k8s_selector_in_namespace(namespace, selector)`, `for_compose_project(project)`, or `for_compose_services(project, services)`. `ExternalNodeSource::new(label, endpoint)` wraps a plain endpoint string.

Each source arm maps to a `ClusterControlProfile` on the returned handle, which workloads can consult to know whether the framework owns node lifecycles (`framework_owns_lifecycle()` is true only for `FrameworkManaged`).

---

## Building the Requests

```rust,ignore
use testing_framework_core::scenario::{ClusterRequest, ExistingCluster, ExternalNodeSource};

// Attach to a running compose project instead of deploying nodes.
let attached = ClusterRequest::<KvEnv>::attached(
    ExistingCluster::for_compose_project("compose-stack-1234".into()),
);

// A cluster made only of static endpoints.
let external = ClusterRequest::<KvEnv>::external(vec![ExternalNodeSource::new(
    "staging-gateway".into(),
    "http://staging.example.net:8080".into(),
)]);

// A managed cluster with one extra external endpoint in its inventory.
let hybrid = ClusterApp::<KvEnv>::new(KvTopology::new(2)).with_external_nodes(vec![
    ExternalNodeSource::new("fixed-peer".into(), "http://10.0.0.5:8080".into()),
]);
```

| Constructor / method | Effect |
|---|---|
| `ClusterRequest::managed(deployment)` | Provision and own N nodes |
| `ClusterRequest::attached(cluster)` | Discover nodes in an existing cluster |
| `ClusterRequest::external(nodes)` | Endpoint-only cluster, no discovery |
| `.with_external_nodes(nodes)` | Add external endpoints to any arm |
| `ClusterApp::with_external_nodes(nodes)` | The same, on the app-level builder |

External nodes compose with every arm: managed + external and attached + external are both valid hybrids, useful when one logical cluster combines framework-visible nodes from more than one source.

Inside an `AppDeployment`, any of these requests goes through `ctx.deploy_cluster(request)`; the simplest scenarios wrap them via `ClusterApp` and `with_app_using` with the matching backend provisioner.

---

## From Source to Typed Client

External and attached sources become typed clients through one hook on the `Application` trait:

```rust,ignore
fn external_node_client(source: &ExternalNodeSource) -> Result<Self::NodeClient, DynError>;
```

The default implementation errors with "external node sources are not supported"; an application opts in by parsing `source.endpoint()` and constructing its client. The local provisioner additionally falls back to a generic parser that resolves `http://host:port` endpoints and builds the client from the socket address when the app has not overridden the hook.

---

## Per-Backend Support

- **Local** (`LocalClusterProvisioner`): no attach — attached requests are rejected with `AttachedUnsupported`. External nodes are supported.
- **Compose** (`ComposeProvisioner`): attach requires a compose descriptor. Services are taken from the descriptor or discovered from the running project; each container's labeled API port is inspected and turned into an `ExternalNodeSource` fed to `external_node_client`. The handle gets a readiness wait, and — when the request asks for `ClusterControlRequest::Full` — node control supporting `restart_node` and `stop_node` against the discovered container IDs. An attach that resolves zero clients fails the deploy. See [Compose Backend](deployer-compose.md).
- **K8s** (`K8sClusterProvisioner`): attach requires a k8s descriptor. Services matching the label selector are listed in the namespace (default `default`); each service's single TCP NodePort (preferring ports named `http` or `api`) becomes the endpoint. The handle gets a readiness wait but **no node control** — that is a current gap; drive lifecycle on k8s through managed clusters or `ManualCluster` instead. See [Kubernetes Backend](deployer-k8s.md).
- **External** requests are supported by all three provisioners and always produce an `ExternalUncontrolled` handle: clients only, no control, no readiness wait, no teardown.

---

## Use Cases

- **Staging and live networks.** Point `ClusterRequest::external` at long-lived endpoints and run workloads and expectations against them; the framework never touches their lifecycle (`ExternalUncontrolled`).
- **Shared test stacks.** Deploy a compose or k8s stack once, keep it alive with the backend preserve env vars (see [Readiness, Retry, and Artifact Preservation](deployment-policies.md)), and attach many fast scenarios to it by project name or label selector.
- **Hybrid clusters.** Combine managed nodes with an external dependency, for example a locally deployed cluster that must interoperate with a fixed remote peer, via `with_external_nodes`.

Manual clusters have their own external hooks: `add_external_sources` and `add_external_clients` on `ManualCluster` merge external endpoints into an imperatively driven cluster (see [ManualCluster](manual-cluster.md)).

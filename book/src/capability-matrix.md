# Backend Capability Matrix

This page records what each backend provisioner currently supports, based on the provisioner implementations.

The framework ships three cluster provisioners: `LocalClusterProvisioner` (local processes), `ComposeProvisioner` (Docker Compose), and `K8sClusterProvisioner` (Kubernetes/Helm). All three implement the same `ClusterProvisioner<E>` operation and feed the same scenario runtime; they differ in where nodes run and which runtime surfaces they wire onto the returned `ClusterHandle`.

| Feature | Local | Compose | K8s |
|---|---|---|---|
| Managed clusters (`ClusterRequest::managed`) | Yes | Yes | Yes |
| On-demand start (`ClusterStartMode::OnDemand`) | Yes | No — rejected (`OnDemandUnsupported`) | Yes |
| Node control (`ClusterControlRequest::Full`) | Yes — start, stop, restart, full `StartNodeOptions` | Restart only (managed); restart + stop (attached) | Managed only — start, stop, restart, per-node readiness via deployment replica scaling; default start options only (`persist_dir`/`snapshot_dir` rejected, config overrides need cfgsync artifacts); works over NodePorts and `kubectl port-forward` (forwards are respawned after restarts); attached clusters get restart with per-call deployment resolution |
| Attached clusters (`ClusterRequest::attached`) | No — rejected | Yes — compose project/services | Yes — label selector; clients, readiness waits, port-forward fallback, and restart control |
| External nodes (`ClusterRequest::external`, `with_external_nodes`) | Yes | Yes | Yes |
| Container stacks (`ContainerStackProvisioner`) | No | Yes — same `ComposeProvisioner` type | No — provisioner pending |
| Observability inputs / metrics | No — handle metrics are empty | Yes | Yes |
| Imperative `ManualCluster` | Yes | No | Yes |
| Deploy retry | Yes — defaults to 3 attempts with backoff | Only when `retry_policy` is set; default is one attempt | No |
| Binary providers | Yes | No — container images | No — container images |
| cfgsync artifacts | No — direct config files | Yes | Yes |

### Example Application Coverage

This table records concrete adapters and runnable example binaries, rather than what the generic framework could theoretically support.

| Example application system | Local | Compose | K8s |
|---|---|---|---|
| `kvstore` | Yes — bin + smoke test | Yes | Yes, including manual cluster and managed restart chaos |
| `openraft_kv` | Yes | Yes | Yes — imperative manual failover |
| `queue` | Yes, including verb DSL and restart chaos | Yes | No |
| `pubsub` | Yes | Yes | Yes, including manual cluster |
| `nats` | Yes | Yes | No |
| `metrics_counter` | No | Yes | Yes, including manual cluster |
| `redis_streams` | No | Yes | No |

---

## Row-by-Row

**Managed clusters.** All three provisioners accept `ClusterRequest::managed(deployment)` — usually built through `ClusterApp` — and return a `ClusterUnit` whose handle carries clients, readiness waits, and cleanup. This is the common path shown in the [Local](deployer-local.md), [Compose](deployer-compose.md), and [Kubernetes](deployer-k8s.md) chapters.

**On-demand start.** Local and k8s prepare the cluster without starting nodes and return a `ManualControlled` handle whose `start_node` calls bring nodes up. The compose provisioner rejects on-demand requests with `ComposeRunnerError::OnDemandUnsupported`.

**Node control.** The local provisioner attaches the full `LocalCluster` control surface when the request asks for `Full`: start, stop, and restart by name, with complete `StartNodeOptions` support (peer selection, config overrides and patches, persist/snapshot dirs, extra args, timeouts). The compose provisioner wires a `ComposeNodeControl` handle that supports `restart_node` via `docker compose restart`; for attached clusters, `ComposeAttachedNodeControl` adds `stop_node` via `docker container stop`. The k8s provisioner backs control with its `ManualCluster`, which scales the per-node Deployments between 0 and 1 replicas for `start_node`, `stop_node`, `restart_node`, and `wait_node_ready`; only default start options are accepted unless the environment implements cfgsync override artifacts. Restarts respawn any `kubectl port-forward` tunnels on their original local ports so existing clients keep working. Attached k8s clusters get the same scaling-based restart, resolving each node name to its Deployment in the attached namespace at call time; unreachable endpoints are reached through automatically spawned port-forwards.

**Attached clusters.** `ClusterRequest::attached(...)` takes descriptors built with `ExistingCluster::for_compose_project` / `for_compose_services` (compose) or `for_k8s_selector` / `for_k8s_selector_in_namespace` (k8s). The local provisioner rejects attached requests outright. Details in [Existing and External Clusters](external-clusters.md).

**External nodes.** All three provisioners resolve `ExternalNodeSource` values into node clients through `Application::external_node_client`. The local provisioner additionally falls back to a generic endpoint parser (`build_external_client`) when the application does not override that hook.

**Container stacks.** `ComposeProvisioner` also implements `ContainerStackProvisioner`, so one compose project can hold node clusters *and* heterogeneous container services declared as `ContainerServiceSpec` values. Each `ContainerServiceHandle` supports individual start, stop, restart, readiness, and running-state operations. Kubernetes does not yet implement `ContainerStackProvisioner`. See [Backend Scope](app-backend-scope.md).

**Observability.** Compose and k8s resolve `ObservabilityInputs` from the `LOGOS_BLOCKCHAIN_*` env vars merged with the request's per-cluster values (request wins), pass the OTLP ingest URL into workspace/chart preparation, and store the result on the handle so `handle.metrics()` returns a Prometheus-backed telemetry handle. The local provisioner leaves observability empty. See [Cluster Control and Observability](capabilities.md) and [Telemetry and External Observability](telemetry.md).

**Manual clusters.** `testing_framework_runner_local::ManualCluster::from_topology` and `testing_framework_runner_k8s::ManualCluster::from_topology` give imperative code the node-control surface without the scenario runtime. On k8s the same `ManualCluster` type also backs the managed provisioning path. See [ManualCluster](manual-cluster.md).

**Binary providers.** Binary resolution (`PathBinaryProvider`, `EnvBinaryProvider`, `BuildBinaryProvider`, `DownloadBinaryProvider`, `FallbackBinaryProvider`) lives in the local crate and feeds local process launches. Compose and k8s nodes run container images instead, so image selection happens through descriptor specs and env-var overrides, not binary providers. See [Binary Providers](binary-providers.md).

**cfgsync artifacts.** The compose provisioner writes a `cfgsync.yaml` into its workspace and can launch a Docker-backed cfgsync config server sidecar; the k8s provisioner supports cfgsync-backed config overrides and cfgsync-rendered bootstrap assets in chart values. The local backend materializes rendered config files directly into each node's working directory with no cfgsync involvement. See [Static Artifacts and cfgsync](cfgsync.md).

---

## Backend Selection

The local backend runs node processes directly and provides full node control. It requires no infrastructure beyond the node binary, which a [binary provider](binary-providers.md) can build. It is the default: `with_app(...)` uses `LocalClusterProvisioner` unless `with_app_using(...)` names another provisioner.

Use Compose for container images, container networking, container-stack composition, or telemetry endpoints. Use Kubernetes to exercise charts, NodePort or port-forward access paths, and cluster infrastructure. Attach to an already-running stack when the cluster outlives the test (see [Existing and External Clusters](external-clusters.md)).

Readiness gating, deploy retries, and artifact preservation are controlled uniformly through the `DeploymentPolicy` carried on each `ClusterRequest`; see [Readiness, Retry, and Artifact Preservation](deployment-policies.md).

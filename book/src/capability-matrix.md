# Capability Matrix

This page records what each deployer backend currently supports, based on the deployer implementations.

The framework ships three deployers: `ProcessDeployer` (local processes), `ComposeDeployer` (Docker Compose), and `K8sDeployer` (Kubernetes/Helm). All three drive the same scenario runtime; they differ in where nodes run and which capabilities they wire into it.

| Feature | Local | Compose | K8s |
|---|---|---|---|
| Uniform managed scenarios | Yes | Yes | Yes |
| Node control (`with_node_control`) | Yes — start, stop, restart | Restart only (managed); restart + stop (attached) | Managed only — start, stop, restart, per-node readiness; default start options only (no config/persist/snapshot/args overrides); works over NodePorts and `kubectl port-forward` (forwards are respawned after restarts); attached mode rejected — use `ManualCluster` |
| Observability / telemetry inputs | No — telemetry is empty | Yes | Yes |
| Attach / existing clusters | No — rejected | Yes — compose project/services | Yes — label selector |
| External nodes | Yes | Yes | Yes |
| App layer / AppHost composition | Yes (only backend) | No | No |
| Binary providers | Yes | No — container images | No — container images |
| cfgsync artifacts | No — direct config files | Yes | Yes |

## Deployer and App-System Coverage

`testing-framework-app` is not a fourth deployer. It is a scenario runtime
extension for composing typed application units. Its built-in process and
cluster adapters currently use the local deployer primitives.

Legend: **Yes** = implemented, **Partial** = implemented with the stated
limits, **No** = no implementation in that subsystem, and **Inherited** = the
app layer uses the enclosing scenario/deployer behavior.

| Feature | Local deployer | Compose deployer | K8s deployer | App system (`testing-framework-app`) |
|---|---|---|---|---|
| Managed uniform cluster | Yes | Yes | Yes | Inherited — apps can wrap the outer deployment, but do not replace its deployer |
| Heterogeneous composed stack | No — deploys one environment topology | No | No | Yes — nested `AppDeployment`s with typed handles |
| Deploy additional child cluster | Yes — `LocalClusterProvisioner` | No provisioner | No provisioner | Yes — local clusters through `DeployContext::deploy_cluster` / `deploy_local_cluster` |
| Deploy standalone component | Yes — local process primitives | No per-component API | No per-component API | Yes — `LocalProcessApp` |
| Managed node lifecycle | Yes — start, stop, restart, readiness, custom start options | Partial — restart; attached mode also stops | Partial — start, stop, restart, per-node readiness via deployment replica scaling; default start options only; port-forwards respawned after lifecycle operations | Yes — local cluster handles provide full control; local process handles start, stop, and restart |
| Imperative `ManualCluster` | Yes | No | Yes — start, stop, restart, readiness; start-option limits apply | Partial — provisioner abstraction exists, built-in/default integration is local |
| Attach existing cluster | No | Yes — project or services | Yes — label selector and namespace | Partial — handle-only presets can wrap outer attached deployments; app units cannot attach independently |
| External node clients | Yes | Yes | Yes | Inherited from the outer scenario through `DeployContext::node_clients` |
| Observability inputs / metrics | No | Yes | Yes | Inherited; the built-in `AppHostLocalDeployer` has no observability capability |
| Binary selection | Yes — path, env, build, download, fallback providers | Container image descriptors | Container image/chart descriptors | Yes — local component launch specs and local child-cluster binary providers |
| cfgsync-backed artifacts | No — writes direct config files | Yes | Yes | No app-specific adapter; child/outer deployer behavior applies |
| Typed application handles | No app registry | No app registry | No app registry | Yes — default and named handles, exposed to workloads |
| Nested deployment and reverse cleanup | Deployer-owned cluster cleanup | Deployer-owned stack cleanup | Deployer-owned release cleanup | Yes — child deployments compose and app resources clean up in reverse registration order |

### Example Application Coverage

This table records concrete adapters and runnable example binaries, rather
than what the generic framework could theoretically support.

| Example application system | Local | Compose | K8s | AppHost / composed app |
|---|---|---|---|---|
| `kvstore` | Yes | Yes | Yes, including manual cluster | Yes — `KvLocalApp` |
| `openraft_kv` | Yes | Yes | Yes, including manual failover | Yes — `OpenRaftKvLocalApp` |
| `queue` | Yes | Yes | No | Yes — `QueueLocalApp` |
| `pubsub` | Yes | Yes | Yes, including manual cluster | No |
| `nats` | Yes | Yes | No | No |
| `metrics_counter` | No | Yes | Yes, including manual cluster | No |
| `redis_streams` | No | Yes | No | No |

---

## Row-by-Row

**Uniform managed scenarios.** All three deployers implement the `Deployer` trait for scenarios built with `ScenarioBuilder<E>` over a topology: `deployer.deploy(&scenario).await` returns a `Runner<E>`. This is the common path shown in the [Local](deployer-local.md), [Compose](deployer-compose.md), and [Kubernetes](deployer-k8s.md) chapters.

**Node control.** The local deployer implements `Deployer<E, NodeControlCapability>` and backs it with a `NodeManager` that can start, stop, and restart node processes, including `StartNodeOptions` (peer selection, config overrides, persist/snapshot dirs). The compose deployer wires a `ComposeNodeControl` handle that supports `restart_node` via `docker compose restart`; in attached (existing-cluster) mode it also supports `stop_node` via `docker container stop`. The k8s deployer wires a `K8sNodeControl` handle into managed scenario deployments that supports `start_node`, `stop_node`, `restart_node`, and `wait_node_ready` by scaling the per-node deployments; only default start options are accepted (config overrides, persist/snapshot dirs, extra args, and timeout overrides are rejected). It works over both direct NodePorts and `kubectl port-forward` fallback — after a restart or start the node's forwards are respawned on their original local ports, so existing clients keep working. Attached (existing-cluster) mode rejects node control. Config-override lifecycle control on Kubernetes goes through the k8s `ManualCluster` (see [Kubernetes Deployer](deployer-k8s.md) and [ManualCluster](manual-cluster.md)).

**Observability / telemetry inputs.** Compose and k8s resolve `ObservabilityInputs` from `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL` / `LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL` / `LOGOS_BLOCKCHAIN_GRAFANA_URL` env vars merged with the scenario's `ObservabilityCapability`, pass the OTLP ingest URL into workspace preparation, and build the run's `Metrics` telemetry handle from the query URL. The local orchestrator constructs its runtime with `Metrics::empty()` and never resolves observability inputs. See [Telemetry and External Observability](telemetry.md).

**Attach / existing clusters.** `with_existing_cluster(...)` switches the scenario to `ClusterMode::ExistingCluster`. Compose accepts descriptors built with `ExistingCluster::for_compose_project` / `for_compose_services`; k8s accepts `for_k8s_selector` / `for_k8s_selector_in_namespace`. The local deployer explicitly rejects existing-cluster mode with a source-orchestration error. Details in [Existing and External Clusters](external-clusters.md).

**External nodes.** All three deployers resolve `with_external_node(s)` sources into node clients through `Application::external_node_client`. The local deployer additionally falls back to a generic endpoint parser (`build_external_client`) when the application does not override that hook.

**App layer / AppHost composition.** The app layer is local-only today: `AppHostLocalDeployer` is a type alias for `ProcessDeployer<AppHostEnv>`. There is no compose or k8s AppHost deployer. See [Backend Scope](app-backend-scope.md).

**Binary providers.** Binary resolution (`PathBinaryProvider`, `EnvBinaryProvider`, `BuildBinaryProvider`, `DownloadBinaryProvider`, `FallbackBinaryProvider`) lives in the local deployer crate and feeds `LocalProcessSpec`. Compose and k8s nodes run container images instead, so image selection happens through descriptor specs and env-var overrides, not binary providers. See [Binary Providers](binary-providers.md).

**cfgsync artifacts.** The compose deployer writes a `cfgsync.yaml` into its workspace and can launch a Docker-backed cfgsync config server sidecar (`ComposeConfigServerMode::Docker`); the k8s deployer supports cfgsync-backed config overrides in manual-cluster flows and cfgsync-rendered bootstrap assets in chart values. The local deployer materializes rendered config files directly into each node's working directory with no cfgsync involvement. See [Static Artifacts and cfgsync](cfgsync.md).

---

## Backend Selection

The local backend runs node processes directly and provides full node control. It requires no infrastructure beyond the node binary, which a [binary provider](binary-providers.md) can build.

Use Compose for container images, container networking, or telemetry endpoints. Use Kubernetes to exercise charts, NodePort or port-forward access paths, and cluster infrastructure. Attach to an already-running stack when the cluster outlives the test (see [Existing and External Clusters](external-clusters.md)).

Readiness gating, deploy retries, and artifact preservation are controlled uniformly through `DeploymentPolicy`; see [Readiness, Retry, and Artifact Preservation](deployment-policies.md).

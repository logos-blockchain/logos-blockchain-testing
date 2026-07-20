# Capability Matrix

This page records what each deployer backend currently supports, based on the deployer implementations.

The framework ships three deployers: `ProcessDeployer` (local processes), `ComposeDeployer` (Docker Compose), and `K8sDeployer` (Kubernetes/Helm). All three drive the same scenario runtime; they differ in where nodes run and which capabilities they wire into it.

| Feature | Local | Compose | K8s |
|---|---|---|---|
| Uniform managed scenarios | Yes | Yes | Yes |
| Node control (`with_node_control`) | Yes — start, stop, restart | Restart only (managed); restart + stop (attached) | No — use `ManualCluster` |
| Observability / telemetry inputs | No — telemetry is empty | Yes | Yes |
| Attach / existing clusters | No — rejected | Yes — compose project/services | Yes — label selector |
| External nodes | Yes | Yes | Yes |
| App layer / AppHost composition | Yes (only backend) | No | No |
| Binary providers | Yes | No — container images | No — container images |
| cfgsync artifacts | No — direct config files | Yes | Yes |

---

## Row-by-Row

**Uniform managed scenarios.** All three deployers implement the `Deployer` trait for scenarios built with `ScenarioBuilder<E>` over a topology: `deployer.deploy(&scenario).await` returns a `Runner<E>`. This is the common path shown in the [Local](deployer-local.md), [Compose](deployer-compose.md), and [Kubernetes](deployer-k8s.md) chapters.

**Node control.** The local deployer implements `Deployer<E, NodeControlCapability>` and backs it with a `NodeManager` that can start, stop, and restart node processes, including `StartNodeOptions` (peer selection, config overrides, persist/snapshot dirs). The compose deployer wires a `ComposeNodeControl` handle that supports `restart_node` via `docker compose restart`; in attached (existing-cluster) mode it also supports `stop_node` via `docker container stop`. The k8s deployer does not wire a node control handle into managed scenario deployments at all; node lifecycle control on Kubernetes goes through the k8s `ManualCluster` (see [Kubernetes Deployer](deployer-k8s.md) and [ManualCluster](manual-cluster.md)).

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

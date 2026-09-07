# Crate and API Map

This chapter maps which crate owns which concept, what each one exports, and how they depend on each other.

The workspace splits into the app-agnostic core, portable resource contracts,
the application-composition layer, deployment backends, and the cfgsync
configuration pipeline. Example applications live under `examples/` and
depend on the framework, never the other way around.

```mermaid
graph BT
    art[cfgsync-artifacts]
    cc[cfgsync-core] --> art
    ca[cfgsync-adapter] --> cc
    ca --> art
    cr[cfgsync-runtime] --> ca
    core[testing-framework-core] --> ca
    container[testing-framework-container] --> core
    local[testing-framework-runner-local] --> core
    compose[testing-framework-runner-compose] --> core
    compose --> container
    k8s[testing-framework-runner-k8s] --> core
    k8s --> cc
    k8s --> art
    app[testing-framework-app] --> core
    app --> container
    app --> local
```

---

## testing-framework-core

Path: `testing-framework/core`. The scenario engine and everything app-agnostic: builder, runtime, topology, observation, sources, capabilities. Every other framework crate depends on it.

| Module | Contents |
|---|---|
| `env` | The `Application` trait (re-exported from `scenario`) |
| `scenario` | `ScenarioBuilder`, `Scenario`, `Workload`, `Expectation`, `RunContext`, `RunHandle`, `Runner`, `RuntimeExtensionFactory`, `DeploymentPolicy`, cluster provisioning (`ClusterRequest`, `ClusterSource`, `ClusterHandle`, `ClusterProvisioner`, `ClusterStartMode`, `ClusterControlRequest`), control traits (`NodeControlHandle`, `ClusterWaitHandle`), sources (`ExistingCluster`, `ExternalNodeSource`), `ObservabilityInputs`, snapshots |
| `topology` | `DeploymentDescriptor`, `DeploymentProvider`, `FixedDeploymentProvider`, `DeploymentSeed`, `DeploymentPlan`, `TopologyShapeBuilder`, `ClusterTopology`, `NodeCountTopology` |
| `observation` | `Observer`, `SourceProvider`, `StaticSourceProvider`, `SourceProviderFactory`, `ObservationExtensionFactory`, `ObservationRuntime`, `ObservationHandle`, `ObservationConfig` |
| `runtime` | `manual` (the `ManualClusterHandle` interface), `process`, `retry` |
| `cfgsync` | Bridges deployments to the cfgsync pipeline (re-exports `cfgsync-adapter`, rendering output types) |

Key builder entry points: `ScenarioBuilder::with_deployment` and `::new(provider)`. The shared fluent methods (`with_workload`, `with_expectation`, `with_observer`, policy setters, ...) live on `CoreBuilderExt`.

---

## testing-framework-container

Path: `testing-framework/container`. The portable contract for containerized
workloads. It owns `ContainerServiceSpec`, `ContainerStackRequest`,
`ContainerStackProvisioner`, resolved endpoints, cleanup ownership, and
per-container lifecycle handles. It contains no application composition,
Docker commands, Compose descriptors, Helm values, Kubernetes resources, or
Local-process implementation.

The app crate consumes this contract when an `AppDeployment` provisions
container resources. Compose implements it today; Kubernetes can implement the
same contract without either backend depending on the app crate.

---

## testing-framework-app

Path: `testing-framework/app`. The composition layer for heterogeneous stacks:
singleton processes, extra clusters, container stacks, or several applications
composed into one system. It consumes Local and container contracts without
owning either backend model (see [Backend Scope](app-backend-scope.md)).

| Export | Role |
|---|---|
| `AppHost`, `AppHostEnv`, `AppHostTopology`, `AppHostScenarioBuilder`, `AppHostDeployer` | Scenario entry point (`AppHost::scenario().with_app(...)`) and the backend-neutral deployer (`AppHostDeployer.deploy(&scenario)`) |
| `AppDeployment`, `AppHandle` | The composition trait and its blanket handle bound |
| `ClusterApp` | Uniform cluster as an app: builds a managed `ClusterRequest`, exposes a `ClusterHandle` |
| `ClusterRestartChaos` | Restart-chaos workload driving a `ClusterHandle`'s node control |
| `DeployContext` | Deploy children, expose typed/named handles, provision clusters (`deploy_cluster`) and container stacks (`deploy_container_stack`), register cleanup (`defer_cleanup`) |
| `AppDeploymentFactory`, `AppScenarioBuilderExt`, `AppRunContextExt` | Builder registration (`with_app`, `with_app_using`) and workload-side handle lookup (`app`, `require_app`, ...) |
| `LocalProcessApp`, `LocalProcessHandle` | One supervised local process as an app |
| `AppRuntime`, `HandleRegistry`, `AppDeployError` | Runtime handle storage and errors; managed cleanup is kept separately |

---

## Deployment Backends

Each backend implements `ClusterProvisioner<E>` for its environment trait; the provisioner is passed per app through `with_app_using` (local is the `with_app` default).

**`testing-framework-runner-local`** (`testing-framework/deployers/local`) spawns nodes as local processes. Exports `LocalClusterProvisioner`, `LocalCluster`, `ManualCluster`, `NodeManager`, the `LocalDeployerEnv` / `LocalBinaryApp` environment traits with config/port helpers (`LocalProcessSpec`, `LocalNodePorts`, `build_local_cluster_node_config`, ...), process primitives (`LaunchSpec`, `NodeEndpoints`, `ProcessNode`), and the whole `binary` module (`BinaryProvider` and its implementations). Honors `TF_KEEP_LOGS` for tempdir retention.

**`testing-framework-runner-compose`** (`.../compose`) renders Docker Compose stacks. Exports `ComposeProvisioner` (which implements both `ClusterProvisioner` and `ContainerStackProvisioner`), `ComposeDeployEnv`, descriptor builders (`ComposeDescriptor`, `NodeDescriptor`), compose lifecycle commands (`compose_up`, `compose_down`, `dump_compose_logs`), and the Docker config-server support used to serve cfgsync artifacts to containers. The cluster and container-stack roles are adapters over one internal managed-project runtime rather than independent Compose implementations.

**`testing-framework-runner-k8s`** (`.../k8s`) installs a Helm release. Exports `K8sClusterProvisioner`, `K8sDeployEnv`, `ManualCluster` (K8s variant), Helm/chart-value infrastructure (`HelmInstallSpec`, `RunnerChartValues`, `render_binary_config_node_chart_assets`, ...), and wait/cleanup helpers. Depends directly on `cfgsync-core` and `cfgsync-artifacts` for artifact delivery.

---

## cfgsync

cfgsync is the typed pipeline that turns app config into per-node files: app config → registration snapshot → per-node artifact sets → backend rendering. Consumed by the compose and k8s deployers (locally, configs are written straight to disk). See [Static Artifacts and cfgsync](cfgsync.md).

| Crate | Responsibility | Key exports |
|---|---|---|
| `cfgsync-artifacts` | App-agnostic artifact model | `ArtifactFile`, `ArtifactSet` |
| `cfgsync-core` | Protocol, client/server, template rendering, bundles | `Client`, `serve_cfgsync`, `NodeRegistration`, `NodeArtifactsPayload`, `RenderedCfgsync`, `NodeArtifactsBundle`, config sources |
| `cfgsync-adapter` | Materializing registration snapshots into artifacts | `RegistrationSnapshotMaterializer`, `CachedSnapshotMaterializer`, `PersistingSnapshotMaterializer`, `MaterializedArtifacts`, `RegistrationConfigSource` |
| `cfgsync-runtime` | Standalone server/client binaries-facing runtime | `serve_from_config`, `run_client_from_env`, `ServerConfig` |

---

## Examples Workspace Layout

Every example app follows the same four-part shape under `examples/<app>/`:

```text
examples/kvstore/
├── kvstore-node/            # the application binary under test
├── testing/
│   ├── integration/         # crate kvstore-runtime-ext: Application impl,
│   │                        #   local/compose/k8s env impls, observation
│   └── workloads/           # crate kvstore-runtime-workloads: Workloads + Expectations
└── examples/                # crate kvstore-examples: runnable bins
```

The naming is uniform: `<app>-runtime-ext`, `<app>-runtime-workloads`, `<app>-examples`. `nats` and `redis_streams` have no node crate because they run upstream binaries or images. `multi_app` uses an acceptance-suite layout instead: a `job-worker/` binary crate, a `fixture/` crate (`multi-app-fixture`: the stack deployment, handles, workload, and expectation), and an `e2e/` crate (`multi-app-e2e`) whose integration tests drive the fixture. It demonstrates application composition.

Run any example bin with:

```bash
cargo run -p kvstore-examples --bin kvstore_basic_convergence
```

**Note:** the dependency arrows only ever point from examples toward the framework and from backends toward core. If you find yourself wanting an arrow in the other direction, read [Framework vs Application Boundaries](tf-boundaries.md). The trait-level view of the same surface is in [Public Extension Points](extension-points.md).

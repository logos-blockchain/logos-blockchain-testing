# Glossary

This glossary gives short definitions of the terms used throughout this book, with a link to the chapter that owns each.

---

**AppDeployment**: the trait a composable application implements: `deploy(self, ctx)` builds the application (processes, clusters, wiring) and returns its handle. Deployments consume themselves and must be `Clone` so the factory can re-run them. See [AppDeployment and DeployContext](app-deployment.md).

**AppHost**: the app-layer entry point. `AppHost::scenario()` returns a scenario builder over a zero-node environment (`AppHostEnv`) so the composed application, not a managed topology, is the system under test. See [AppHost and with_app](app-host.md).

**Application (environment)**: the trait that defines one application's deployment descriptor, node client, node config, and readiness path for the framework. Often called the environment; implemented once per application. See [Application, AppDeployment, and Environments](application-model.md).

**Binary Provider**: the local deployer's strategy for producing a node executable: explicit path, env-var override, build command, checksum-verified download, or an ordered fallback chain. Resolution is cached and cross-process locked. See [Binary Providers](binary-providers.md).

**cfgsync artifact**: a per-node configuration file rendered from typed app config by the cfgsync pipeline and served to nodes at container startup; how the compose and k8s deployers get configs into containers. See [Static Artifacts and cfgsync](cfgsync.md).

**Cleanup Guard**: the core runner's teardown hook (`CleanupGuard`). Guards are registered as resources are acquired and run when the scenario runtime is released; the app layer groups its managed resources in a LIFO cleanup stack. See [Handle Ownership and Teardown](handles-teardown.md).

**Deployer**: the object that turns a scenario definition into running infrastructure (`deployer.deploy(&scenario)` → runner): local processes, a compose stack, or a Kubernetes namespace. See [Capability Matrix](capability-matrix.md).

**Deployment Plan / Topology**: the application-defined descriptor of what to deploy (node count and layout), owned by the `Application::Deployment` type and consumed by every backend. See [Topology and Deployment Plans](topology.md).

**Deployment Policy**: per-scenario knobs for deploy behavior: readiness on/off and requirement, optional retry with backoff, and artifact preservation (`CleanupPolicy`). Set with `with_deployment_policy`. See [Readiness, Retry, and Artifact Preservation](deployment-policies.md).

**Entry Pattern**: one of the three declarative ways into the scenario runtime (uniform managed cluster, AppHost composed stack, attached/external nodes), or imperative control through `ManualCluster`. See [Choosing an Entry Pattern](entry-patterns.md).

**Existing Cluster / External Node**: sources that plug already-running infrastructure into a scenario instead of deploying it: `ExistingCluster` for a whole cluster, `ExternalNodeSource` for a single endpoint. See [Existing and External Clusters](external-clusters.md).

**Expectation**: a post-run (and cooldown-aware) assertion about the system's end state, registered with `with_expectation`; expectations decide whether the scenario passed. See [Expectations and Evaluation](expectations.md).

**Handle (typed / named)**: a cheaply clonable access or control value exposed by a deployment and fetched by workloads (`require_app::<T>()`), keyed by concrete type plus an optional instance name. Managed lifetime belongs to scenario cleanup rather than handle clones. See [Handle Ownership and Teardown](handles-teardown.md).

**Cluster Provisioner**: a backend adapter that turns a managed, attached, or external `ClusterRequest<E>` into common clients, controls, readiness, and optional cleanup. See [Shared Cluster Provisioning](cluster-provisioning.md).

**Verb Layer**: optional typed syntax that expands domain actions into ordinary workloads, expectations, and capability requests. See [The Verb Layer](verb-layer.md).

**ManualCluster**: imperative node orchestration that bypasses the scenario runner: start, stop, restart, and probe named nodes directly. Use it for interactive debugging and bespoke lifecycles. See [ManualCluster: Imperative Node Control](manual-cluster.md).

**Observation**: the continuous observation runtime: named `ObservedSource`s polled into snapshots and history that workloads and expectations read through an `ObservationHandle`. Test-visible application state, as opposed to Telemetry. See [Continuous Observation](observation.md).

**Runner**: what a deployer returns after a successful deploy; `runner.run(&mut scenario)` executes workloads, evaluates expectations, and tears the run down. See [Scenario Model and Lifecycle](scenario-model.md).

**Runtime Extension**: a typed value prepared before workloads start and shared through the `RunContext` (one instance per type). The app layer's `AppRuntime` is a runtime extension. See [Runtime Extensions](runtime-extensions.md).

**Scenario**: the complete declarative test definition produced by a `ScenarioBuilder`: deployment, workloads, expectations, run duration, policies, and extensions, all evaluated by one runtime regardless of entry pattern. See [Scenario Model and Lifecycle](scenario-model.md).

**Seed**: the value (`DeploymentSeed`, set via `with_deployment_seed`) that makes generated deployments deterministic, so a failing run can be replayed exactly. See [Seeds and Reproducibility](seeds.md).

**Telemetry**: metrics, logs, and tracing reached through external endpoints (`ObservabilityInputs`, Prometheus, Grafana); operational visibility, as opposed to the Observation runtime's test-visible state. See [Telemetry and External Observability](telemetry.md).

**Workload**: active behavior during the run: a named task (`trait Workload`) started against the `RunContext` that drives traffic or chaos while the scenario clock runs. See [Workloads and Concurrency](workloads.md).

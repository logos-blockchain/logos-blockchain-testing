# Environment Variables

This chapter is the complete, audited list of environment variables the framework reads, and where each read happens.

This chapter was produced by auditing the source (`grep -rn "env::var" testing-framework/ cfgsync/ --include="*.rs"`), not by convention. If a variable is not listed here, the framework does not read it. Re-run the grep after upgrading.

---

## Core (`testing-framework-core`)

| Variable | Purpose | Read in | When unset |
|---|---|---|---|
| `SLOW_TEST_ENV` | When exactly `true`, `adjust_timeout` doubles framework timeouts (slow CI runners) | `core/src/lib.rs` | normal timeouts |
| `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL` | Prometheus-compatible query endpoint for `ObservabilityInputs::from_env` | `core/src/scenario/observability.rs` | metrics queries disabled (`Metrics::empty()`) |
| `LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL` | OTLP metrics ingest endpoint | `core/src/scenario/observability.rs` | none |
| `LOGOS_BLOCKCHAIN_GRAFANA_URL` | Grafana base URL surfaced alongside run output | `core/src/scenario/observability.rs` | none |

The three `LOGOS_BLOCKCHAIN_*` names are historical; they are only consulted when telemetry inputs come from the environment rather than from an `ObservabilityCapability`; see [Telemetry and External Observability](telemetry.md).

---

## Local Deployer (`testing-framework-runner-local`)

| Variable | Purpose | Read in | When unset |
|---|---|---|---|
| `TF_KEEP_LOGS` | Preserve per-node working directories (`1`/`true`/`yes`) | `deployers/local/src/lib.rs`, honored by the orchestrator and `ManualCluster` | directories deleted at teardown (unless the deployment policy preserves them) |

Two provider types read **caller-named** variables, where the framework defines the mechanism and the application names the variable:

- `EnvBinaryProvider::new("MY_NODE_BIN")` reads that variable as an explicit executable path. Unset or not-a-file counts as unresolved, letting a `FallbackBinaryProvider` continue to the next provider.
- `DownloadUrl::Env(var)` / `DownloadChecksum::Env(var)` on `DownloadBinaryProvider` read the download URL and expected SHA-256 from the named variables. A missing URL variable is a hard error (`MissingDownloadUrl`); a missing checksum variable disables verification.

See [Binary Providers](binary-providers.md).

---

## Compose Deployer (`testing-framework-runner-compose`)

| Variable | Purpose | Read in | When unset |
|---|---|---|---|
| `COMPOSE_RUNNER_PRESERVE` | Skip `docker compose down`, keep the workspace | `lifecycle/cleanup.rs` | full teardown |
| `TESTNET_RUNNER_PRESERVE` | Alias for the above | `lifecycle/cleanup.rs` | full teardown |
| `COMPOSE_RUNNER_HOST` | Host used to reach published container ports | `infrastructure/ports.rs` | `127.0.0.1` |
| `COMPOSE_RUNNER_HOST_GATEWAY` | Explicit `extra_hosts` gateway entry; `disable` or empty removes it | `docker/platform.rs` | falls through to `DOCKER_HOST_GATEWAY` |
| `DOCKER_HOST_GATEWAY` | Gateway IP mapped as `host.docker.internal:<ip>` | `docker/platform.rs` | `host.docker.internal:host-gateway` |
| `TESTNET_PRINT_ENDPOINTS` | If set (any value), print discovered endpoints after deploy | `deployer/orchestrator.rs` | silent |
| `REPO_ROOT_OVERRIDE_DIR` | Override repository-root detection for stack assets | `docker/workspace.rs` | falls through to `CARGO_WORKSPACE_DIR`, then manifest-relative detection |
| `CARGO_WORKSPACE_DIR` | Workspace root override (also used by template rendering) | `docker/workspace.rs`, `infrastructure/template.rs` | manifest-relative detection |
| `REL_ASSETS_STACK_DIR` | Alternative stack-assets directory (absolute, or relative to repo root) | `docker/workspace.rs` | bundled default assets |

Per-application image selection is again a mechanism with caller-derived names: `BinaryConfigNodeSpec::conventional("/usr/local/bin/kvstore-node", ...)` derives the prefix `KVSTORE` and reads `KVSTORE_IMAGE` (default `kvstore-node:local`) and `KVSTORE_PLATFORM` (`descriptor/node.rs`).

---

## Kubernetes Deployer (`testing-framework-runner-k8s`)

| Variable | Purpose | Read in | When unset |
|---|---|---|---|
| `K8S_RUNNER_NODE_HOST` | Host used to reach NodePort services | `host.rs` | `KUBERNETES_SERVICE_HOST`, then `127.0.0.1` |
| `KUBERNETES_SERVICE_HOST` | Standard fallback for the above (e.g. Docker Desktop) | `host.rs` | `127.0.0.1` |
| `K8S_RUNNER_PRESERVE` | Skip Helm uninstall and namespace deletion | `env.rs` | full teardown |
| `K8S_RUNNER_DEBUG` | Log Helm install stdout/stderr | `infrastructure/helm.rs` | Helm output suppressed |
| `K8S_RUNNER_DEPLOYMENT_TIMEOUT_SECS` | Deployment readiness timeout (integer seconds) | `lifecycle/wait/mod.rs` | built-in default |
| `K8S_RUNNER_HTTP_TIMEOUT_SECS` | Node HTTP readiness timeout | `lifecycle/wait/mod.rs` | built-in default |
| `K8S_RUNNER_HTTP_PROBE_TIMEOUT_SECS` | Per-probe HTTP timeout | `lifecycle/wait/mod.rs` | built-in default |
| `K8S_RUNNER_HTTP_POLL_INTERVAL_SECS` | Readiness poll interval | `lifecycle/wait/mod.rs` | built-in default |
| `TESTNET_PRINT_ENDPOINTS` | If set, print Prometheus/Grafana/pprof endpoints after deploy | `deployer/orchestrator.rs` | silent |

Image selection mirrors compose with a k8s-specific override first: `BinaryConfigK8sSpec::conventional` reads `<PREFIX>_K8S_IMAGE`, then `<PREFIX>_IMAGE`, then the `<binary-name>:local` default (`env.rs`). `workspace.rs` additionally exposes `resolve_workspace_root` / `resolve_optional_relative_dir` helpers that read a variable **named by the caller**.

---

## cfgsync Runtime (`cfgsync-runtime`)

These are read by the cfgsync **client inside node containers** at startup, not by your test process; the deployers set them when rendering the stack. See [Static Artifacts and cfgsync](cfgsync.md).

| Variable | Purpose | When unset |
|---|---|---|
| `CFG_SERVER_ADDR` | cfgsync server URL | `http://127.0.0.1:<default port>` |
| `CFG_HOST_IP` | This node's IPv4 address for registration | `127.0.0.1` |
| `CFG_HOST_IDENTIFIER` | Node identifier for registration | `unidentified-node` |
| `CFG_REGISTRATION_METADATA_JSON` | Extra registration payload (JSON) | empty payload |
| `CFG_FILE_PATH` | Where to write the fetched `config.yaml` | config output not routed |
| `CFG_DEPLOYMENT_PATH` | Where to write the fetched deployment settings | deployment output not routed |
| `LOGOS_BLOCKCHAIN_CFGSYNC_PORT` | Default server port for the `cfgsync-client` binary | `4400` |

---

## Example-App Variables (Not Framework Variables)

The example applications define their own variables through the mechanisms above. **These belong to the examples**: `KVSTORE_NODE_BIN` is defined by the kvstore example's environment implementation, not by the framework; your application will define its own equivalents. Found by auditing `examples/`:

| Variable | Example | Purpose |
|---|---|---|
| `KVSTORE_NODE_BIN`, `OPENRAFT_KV_NODE_BIN` | kvstore, openraft_kv | optional binary override (fallback builds with Cargo) |
| `QUEUE_NODE_BIN`, `PUBSUB_NODE_BIN`, `METRICS_COUNTER_NODE_BIN` | queue, pubsub, metrics_counter | required node binary path for local runs |
| `NATS_SERVER_BIN` | nats | path to an upstream `nats-server` executable |
| `NATS_IMAGE` / `NATS_PLATFORM` | nats | compose image override (default `nats:2.10`) |
| `REDIS_STREAMS_IMAGE` / `REDIS_STREAMS_PLATFORM` | redis_streams | compose image override (default `redis:7`) |
| `KVSTORE_IMAGE`, `QUEUE_IMAGE`, … (`<PREFIX>_IMAGE`/`<PREFIX>_PLATFORM`/`<PREFIX>_K8S_IMAGE`) | all node apps | derived image overrides via the conventional specs |
| `METRICS_COUNTER_K8S_PROMETHEUS_NODE_PORT` | metrics_counter | fixed NodePort for the Prometheus service |
| `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL` | metrics_counter | also consulted by the example to locate Prometheus |

See [Running the Examples](running-examples.md) for how these fit each binary.

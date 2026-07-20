# Running the Examples

This chapter lists every runnable example binary, the exact command to launch it, and what it needs from your machine.

---

## Conventions

All examples are ordinary binaries run with:

```bash
cargo run -p <package> --bin <bin>
```

Naming encodes the backend: `*_basic_*` and `*_app_host_*` run as local processes, `*_compose_*` need a running Docker daemon, and `*_k8s_*` need a reachable Kubernetes cluster context (the k8s deployer drives Helm and the cluster API). Compose binaries exit gracefully with a warning when Docker is unavailable, and the k8s binaries skip when the cluster cannot be reached (`K8sRunnerError::ClientInit`).

Logging uses `tracing_subscriber` with an env filter; set `RUST_LOG` to adjust verbosity.

---

## Summary

| Binary | Package | Backend | Requirements |
|---|---|---|---|
| `kvstore_app_host_convergence` | `kvstore-examples` | local (AppHost) | none — node auto-built |
| `kvstore_basic_convergence` | `kvstore-examples` | local | none — node auto-built |
| `kvstore_compose_convergence` | `kvstore-examples` | compose | Docker + `kvstore-node:local` image |
| `kvstore_k8s_convergence` | `kvstore-examples` | k8s | cluster context, Helm, image |
| `kvstore_k8s_manual_convergence` | `kvstore-examples` | k8s (manual) | cluster context, Helm, image |
| `openraft_kv_app_host_smoke` | `openraft-kv-examples` | local (AppHost) | none — node auto-built |
| `openraft_kv_basic_failover` | `openraft-kv-examples` | local | none — node auto-built |
| `openraft_kv_compose_failover` | `openraft-kv-examples` | compose | Docker + `openraft-kv-node:local` image |
| `openraft_kv_k8s_failover` | `openraft-kv-examples` | k8s | cluster context, Helm, image |
| `processes_queued_jobs_and_converges_results` | `multi-app-e2e` (test, not a bin) | local (AppHost) | none — nodes and worker auto-built |
| `nats_basic_roundtrip` | `nats-examples` | local | `nats-server` binary via `NATS_SERVER_BIN` |
| `nats_compose_roundtrip` | `nats-examples` | compose | Docker + `nats:2.10` image present |
| `nats_parity_check` | `nats-examples` | compose + local | Docker; local leg needs `nats-server` |
| `redis_streams_compose_roundtrip` | `redis-streams-examples` | compose | Docker + `redis:7` image present |
| `redis_streams_compose_failover` | `redis-streams-examples` | compose | Docker + `redis:7` image present |
| `pubsub_basic_ws_roundtrip` | `pubsub-examples` | local | `PUBSUB_NODE_BIN` |
| `pubsub_basic_ws_reconnect` | `pubsub-examples` | local | `PUBSUB_NODE_BIN` |
| `pubsub_compose_ws_roundtrip` | `pubsub-examples` | compose | Docker + `pubsub-node:local` image |
| `pubsub_compose_ws_reconnect` | `pubsub-examples` | compose | Docker + `pubsub-node:local` image |
| `pubsub_k8s_ws_roundtrip` | `pubsub-examples` | k8s | cluster context, Helm, image |
| `pubsub_k8s_manual_ws_roundtrip` | `pubsub-examples` | k8s (manual) | cluster context, Helm, image |
| `queue_basic_convergence` | `queue-examples` | local | `QUEUE_NODE_BIN` |
| `queue_basic_restart_chaos` | `queue-examples` | local | `QUEUE_NODE_BIN` |
| `queue_basic_roundtrip` | `queue-examples` | local | `QUEUE_NODE_BIN` |
| `queue_compose_convergence` | `queue-examples` | compose | Docker + `queue-node:local` image |
| `queue_compose_roundtrip` | `queue-examples` | compose | Docker + `queue-node:local` image |
| `metrics_counter_compose_prometheus_expectation` | `metrics-counter-examples` | compose | Docker + `metrics-counter-node:local` image |
| `metrics_counter_k8s_prometheus_expectation` | `metrics-counter-examples` | k8s | cluster context, Helm, image |
| `metrics_counter_k8s_manual_prometheus` | `metrics-counter-examples` | k8s (manual) | cluster context, Helm, image |

---

## Binary Resolution for Local Runs

Local examples resolve their node binary through a [Binary Provider](binary-providers.md):

- **kvstore and openraft_kv** use a `FallbackBinaryProvider`: an explicit `KVSTORE_NODE_BIN` / `OPENRAFT_KV_NODE_BIN` override wins, otherwise a `BuildBinaryProvider` runs `cargo build -p <node-crate>` for you. No setup needed.
- **queue, pubsub, and metrics_counter** use a plain `EnvBinaryProvider`: you must build the node and point the env var at it:

```bash
cargo build -p queue-node
QUEUE_NODE_BIN=target/debug/queue-node cargo run -p queue-examples --bin queue_basic_convergence
```

- **nats** launches the upstream `nats-server` executable. Point `NATS_SERVER_BIN` at one (for example from a package manager install). `nats_parity_check` probes for it (env var or `PATH`) and skips the local leg when it is missing.

---

## Compose Images

The compose deployer checks images with `docker image inspect` and does **not** build or pull them (`MissingImage` error otherwise; see [Troubleshooting](troubleshooting.md)):

- In-repo node apps default to `<binary-name>:local` (override via `<APP>_IMAGE`). Build them from the repository root, e.g.:

```bash
docker build -f examples/kvstore/Dockerfile -t kvstore-node:local .
```

Dockerfiles exist for kvstore, openraft_kv, queue, pubsub, and metrics_counter.

- **nats and redis_streams have no node crate at all**: they run the upstream images `nats:2.10` and `redis:7` (override via `NATS_IMAGE` / `REDIS_STREAMS_IMAGE`, platform via `NATS_PLATFORM` / `REDIS_STREAMS_PLATFORM`). Pull them once with `docker pull nats:2.10` / `docker pull redis:7`.

---

## What Each Group Exercises

**kvstore** demonstrates a uniform cluster. `kvstore_app_host_convergence` deploys a local cluster through `AppHost::scenario().with_app(...)` and drives a write/restart/write convergence workload ([Quickstart](quickstart.md) walks it line by line). `kvstore_basic_convergence` is the same coverage through a direct `ScenarioBuilder`. The Compose and Kubernetes variants run the same scenario against those backends; `kvstore_k8s_manual_convergence` bypasses the scenario runner and drives the cluster imperatively via `manual_cluster_from_descriptors` ([ManualCluster](manual-cluster.md)).

**openraft_kv** demonstrates consensus and leader failover. `openraft_kv_app_host_smoke` is the AppHost entry point. `openraft_kv_basic_failover` and `openraft_kv_compose_failover` share one scenario built with `.enable_node_control()`: write a batch, restart the Raft leader through the node-control capability, write again, and expect convergence ([Scenario Capabilities](capabilities.md)). `openraft_kv_k8s_failover` runs the same failover flow imperatively through the Kubernetes `ManualCluster`, because the Kubernetes deployer wires no node control into managed scenarios ([ManualCluster](manual-cluster.md)).

**multi_app** demonstrates application composition and runs as an acceptance test rather than a binary: `cargo test -p multi-app-e2e`. The `multi-app-fixture` crate deploys a queue cluster and a key-value result-store cluster inside one root `AppDeployment` and launches the `multi-app-job-worker` binary between them (resolved via `MULTI_APP_JOB_WORKER_BIN`, else built by Cargo); the test enqueues ten jobs and expects ten results on every store node ([Composing Heterogeneous Stacks](composing-stacks.md)).

**nats / redis_streams** test unmodified third-party servers. Round-trip workloads publish and consume messages; `redis_streams_compose_failover` runs a consumer-group failover where a second consumer reclaims another's pending stream entries. `nats_parity_check` runs the same scenario against Compose and local backends in one binary.

**pubsub / queue** exercise WebSocket fan-out and work-queue semantics on small in-repo nodes; `queue_basic_restart_chaos` enables node control for restart chaos under load ([Chaos and Controlled Failure](chaos.md)).

**metrics_counter** is the telemetry demonstration. The compose variant deploys nodes plus a Prometheus container and asserts on scraped metrics through a Prometheus-backed expectation; it honors `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL` as a query-endpoint override ([Telemetry and External Observability](telemetry.md)).

The app-layer examples (`*_app_host_*`, the `multi-app-e2e` tests) show composed systems. The direct-builder binaries provide backend-specific coverage; see `examples/README.md`.

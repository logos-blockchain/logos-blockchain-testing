# Running the Examples

This chapter lists every runnable example target, the exact command to launch it, and what it needs from your machine.

---

## Conventions

All examples are ordinary binaries run with:

```bash
cargo run -p <package> --bin <bin>
```

Every scenario binary is the same app-model program — `AppHost::scenario().with_app(...)` deployed through `AppHostDeployer` — and naming encodes the backend provisioner: `*_basic_*` binaries use the default `LocalClusterProvisioner`, `*_compose_*` pass `ComposeProvisioner` and need a running Docker daemon, and `*_k8s_*` pass `K8sClusterProvisioner` and need a reachable Kubernetes cluster context. `*_k8s_manual_*` binaries construct the k8s `ManualCluster` directly and drive it imperatively. Compose binaries exit gracefully with a warning when Docker is unavailable (`ComposeRunnerError::DockerUnavailable`), and the k8s binaries skip when the cluster cannot be reached (`ManualClusterError::ClientInit`, or an unreachable-API `InstallStack`).

Logging uses `tracing_subscriber` with an env filter; set `RUST_LOG` to adjust verbosity.

---

## Summary

| Target | Package | Backend | Requirements |
|---|---|---|---|
| `kvstore_basic_convergence` | `kvstore-examples` | local | none — node auto-built |
| `local_kvstore_runs_a_complete_scenario` | `kvstore-examples` (test `local_smoke`) | local | none — node auto-built |
| `kvstore_compose_convergence` | `kvstore-examples` | compose | Docker + `kvstore-node:local` image |
| `kvstore_k8s_convergence` | `kvstore-examples` | k8s | cluster context, Helm, image |
| `kvstore_k8s_restart_chaos` | `kvstore-examples` | k8s | cluster context, Helm, image |
| `kvstore_k8s_manual_convergence` | `kvstore-examples` | k8s (manual) | cluster context, Helm, image |
| `openraft_kv_basic_failover` | `openraft-kv-examples` | local | none — node auto-built |
| `openraft_kv_compose_failover` | `openraft-kv-examples` | compose | Docker + `openraft-kv-node:local` image |
| `openraft_kv_k8s_failover` | `openraft-kv-examples` | k8s (manual) | cluster context, Helm, image |
| `processes_queued_jobs_and_converges_results` | `multi-app-e2e` (test) | local | none — nodes and worker auto-built |
| `containerized_services_process_jobs_and_converge` | `multi-app-e2e` (test) | compose | Docker + queue, kvstore, and worker images |
| `extensions_preserve_stopped_services_and_roll_back_failures` | `multi-app-e2e` (test) | compose | Docker |
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
| `queue_dsl_demo` | `queue-examples` | local | `QUEUE_NODE_BIN` |
| `dsl_restart_scenario_converges` | `queue-examples` (test `dsl_chaos`) | local | `QUEUE_NODE_BIN` |
| `queue_compose_convergence` | `queue-examples` | compose | Docker + `queue-node:local` image |
| `queue_compose_roundtrip` | `queue-examples` | compose | Docker + `queue-node:local` image |
| `metrics_counter_compose_prometheus_expectation` | `metrics-counter-examples` | compose | Docker + `metrics-counter-node:local` image |
| `metrics_counter_k8s_prometheus_expectation` | `metrics-counter-examples` | k8s | cluster context, Helm, image |
| `metrics_counter_k8s_manual_prometheus` | `metrics-counter-examples` | k8s (manual) | cluster context, Helm, image |

Tests run with `cargo test -p <package>` (add `--test <file>` to select one file, e.g. `cargo test -p kvstore-examples --test local_smoke`). CI exercises the kvstore local smoke test and the multi-app local happy path on every change.

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

The compose provisioner checks images with `docker image inspect` and does **not** build or pull them (`MissingImage` error otherwise; see [Troubleshooting](troubleshooting.md)):

- In-repo node apps default to `<binary-name>:local` (override via `<APP>_IMAGE`). Build them from the repository root, e.g.:

```bash
docker build -f examples/kvstore/Dockerfile -t kvstore-node:local .
```

Dockerfiles exist for kvstore, openraft_kv, queue, pubsub, and metrics_counter.

- **nats and redis_streams have no node crate at all**: they run the upstream images `nats:2.10` and `redis:7` (override via `NATS_IMAGE` / `REDIS_STREAMS_IMAGE`, platform via `NATS_PLATFORM` / `REDIS_STREAMS_PLATFORM`). Pull them once with `docker pull nats:2.10` / `docker pull redis:7`.

---

## What Each Group Exercises

**kvstore** demonstrates a uniform cluster. `kvstore_basic_convergence` deploys a three-node cluster through `ClusterApp` and drives a keyed write workload to convergence ([Quickstart](quickstart.md) walks it line by line); the `local_smoke` test adds a restart mid-scenario through the cluster handle. The Compose and Kubernetes variants pass a different provisioner to the same scenario. `kvstore_k8s_restart_chaos` runs `ClusterRestartChaos` against a managed k8s cluster under write load ([Chaos](chaos.md)); `kvstore_k8s_manual_convergence` bypasses the scenario runner and drives the k8s `ManualCluster` imperatively ([ManualCluster](manual-cluster.md)).

**openraft_kv** demonstrates consensus and leader failover. `openraft_kv_basic_failover` and `openraft_kv_compose_failover` share one provisioner-parameterized scenario: write a batch, restart the Raft leader through the cluster handle, write again, and expect convergence ([Chaos](chaos.md)). `openraft_kv_k8s_failover` runs the same failover flow imperatively through the Kubernetes `ManualCluster`.

**multi_app** demonstrates application composition and runs as acceptance tests rather than binaries. The local test composes child clusters and a worker process. The compose tests submit the queue, result store, and worker as separate container-stack requests into one project, exercise individual worker stop/start/restart, then run the same job workload and convergence expectation. Both expose the same typed `JobStackHandle` ([Composing Heterogeneous Stacks](composing-stacks.md)).

**nats / redis_streams** test unmodified third-party servers deployed as `ClusterApp` clusters. Round-trip workloads publish and consume messages; `redis_streams_compose_failover` runs a consumer-group failover where a second consumer reclaims another's pending stream entries. `nats_parity_check` runs the same scenario against compose and local backends in one binary.

**pubsub / queue** exercise WebSocket fan-out and work-queue semantics on small in-repo nodes. `queue_basic_restart_chaos` restarts nodes through the cluster handle under load ([Chaos](chaos.md)), and `queue_dsl_demo` plus the `dsl_chaos` test drive the queue verb DSL ([The Verb Layer](verb-layer.md)).

**metrics_counter** is the telemetry demonstration. The compose variant deploys nodes plus a Prometheus container and asserts on scraped metrics through a Prometheus-backed expectation; it honors `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL` as a query-endpoint override ([Telemetry and External Observability](telemetry.md)).

The `multi-app-e2e` tests show composed stacks; the single-cluster binaries provide backend-specific coverage; see `examples/README.md`.

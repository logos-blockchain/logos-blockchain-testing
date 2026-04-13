# PubSub Example

This example runs a small replicated pub/sub service with a WebSocket client
API.

The scenarios open WebSocket sessions, subscribe to a topic, publish messages,
and check that every subscriber sees the same events. There is also a reconnect
scenario and a k8s manual-cluster variant that restarts a node mid-run.

## How TF runs this

Each example follows the same pattern:

- TF starts a small deployment of pubsub nodes
- a workload opens WebSocket sessions and drives publish/subscribe behavior
- an expectation checks that the nodes end up with the same topic state

## Scenarios

- `basic_ws_roundtrip` runs a local roundtrip check
- `basic_ws_reconnect` runs a local reconnect scenario
- `compose_ws_roundtrip` and `compose_ws_reconnect` run the same checks in Docker Compose
- `k8s_ws_roundtrip` runs the roundtrip scenario on Kubernetes
- `k8s_manual_ws_roundtrip` starts the nodes through the k8s manual cluster API, restarts one node, and checks that the topic state converges again

## Run locally

```bash
cargo run -p pubsub-examples --bin basic_ws_roundtrip
cargo run -p pubsub-examples --bin basic_ws_reconnect
```

## Run with Docker Compose

```bash
cargo run -p pubsub-examples --bin compose_ws_roundtrip
cargo run -p pubsub-examples --bin compose_ws_reconnect
```

Set `PUBSUB_IMAGE` to override the default compose image tag.

## Run with Kubernetes

```bash
docker build -t pubsub-node:local -f examples/pubsub/Dockerfile .
cargo run -p pubsub-examples --bin k8s_ws_roundtrip
```

Prerequisites:
- `kubectl` configured with a reachable cluster
- `helm` installed

Optional image override:
- `PUBSUB_K8S_IMAGE` (falls back to `PUBSUB_IMAGE`, then `pubsub-node:local`)

## Run with Kubernetes manual cluster

```bash
docker build -t pubsub-node:local -f examples/pubsub/Dockerfile .
cargo run -p pubsub-examples --bin k8s_manual_ws_roundtrip
```

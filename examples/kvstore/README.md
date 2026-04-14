# KV Store Example

This example runs a small replicated key-value store.

The usual scenario writes keys through one node and checks that the other nodes
eventually return the same values.

## How TF runs this

Each example follows the same pattern:

- TF starts a small deployment of kvstore nodes
- a workload writes keys through one node
- an expectation keeps reading from all nodes until they agree on the values

## Scenarios

- `basic_convergence` runs the convergence check locally
- `compose_convergence` runs the same check in Docker Compose
- `k8s_convergence` runs it on Kubernetes
- `k8s_manual_convergence` starts the nodes through the k8s manual cluster API, restarts one node, and checks convergence again

## API

Each node exposes:

- `PUT /kv/:key` to write a value
- `GET /kv/:key` to read a value
- `GET /internal/snapshot` to read the local replicated state

## Run locally

```bash
cargo run -p kvstore-examples --bin kvstore_basic_convergence
```

## Run with Docker Compose

```bash
cargo run -p kvstore-examples --bin kvstore_compose_convergence
```

Set `KVSTORE_IMAGE` to override the default compose image tag.

## Run with Kubernetes

```bash
docker build -t kvstore-node:local -f examples/kvstore/Dockerfile .
cargo run -p kvstore-examples --bin kvstore_k8s_convergence
```

Prerequisites:
- `kubectl` configured with a reachable cluster
- `helm` installed

Optional image override:
- `KVSTORE_K8S_IMAGE` (falls back to `KVSTORE_IMAGE`, then `kvstore-node:local`)

## Run with Kubernetes manual cluster

```bash
docker build -t kvstore-node:local -f examples/kvstore/Dockerfile .
cargo run -p kvstore-examples --bin kvstore_k8s_manual_convergence
```

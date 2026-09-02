# OpenRaft KV Example

This example runs a small key-value service built on top of `OpenRaft`.

The main scenario does four things:

- bootstraps node 0 as a one-node cluster
- adds nodes 1 and 2 as learners and promotes them to voters
- writes one batch of keys through the current leader
- restarts that leader, waits for a new leader, writes again, and then checks
  that all three nodes expose the same replicated state

## How TF runs this

- TF starts three OpenRaft nodes
- the workload bootstraps the cluster through the admin API
- the workload writes a first batch, restarts the current leader, waits for failover, and writes again
- the expectation checks that all three nodes converge on the same key/value state and membership

## Scenario

- `basic_failover` runs the leader-restart flow locally
- `compose_failover` runs the same flow in Docker Compose
- `k8s_failover` runs the same flow against a manual Kubernetes cluster deployment

## API

Each node exposes:

- `GET /healthz` for readiness
- `GET /state` for current Raft role, leader, membership, log progress, and replicated key/value data
- `POST /kv/write` to submit a write through the local Raft node
- `POST /kv/read` to read a key from the local state machine
- `POST /admin/init` to initialize a single-node cluster
- `POST /admin/add-learner` to add a new Raft learner
- `POST /admin/change-membership` to promote learners into the voting set

The node also exposes internal Raft RPC endpoints used only for replication:

- `POST /raft/vote`
- `POST /raft/append`
- `POST /raft/snapshot`

## Run locally

```bash
OPENRAFT_KV_NODE_BIN="$(pwd)/target/debug/openraft-kv-node" \
cargo run -p openraft-kv-examples --bin openraft_kv_basic_failover
```

Build the node first if you have not done that yet:

```bash
cargo build -p openraft-kv-node
```

## Run with Docker Compose

Build the image first:

```bash
docker build -t openraft-kv-node:local -f examples/openraft_kv/Dockerfile .
```

Then run:

```bash
cargo run -p openraft-kv-examples --bin openraft_kv_compose_failover
```

Set `OPENRAFT_KV_IMAGE` to override the default compose image tag.

## Run on Kubernetes

Build the same image first:

```bash
docker build -t openraft-kv-node:local -f examples/openraft_kv/Dockerfile .
```

Then run:

```bash
cargo run -p openraft-kv-examples --bin openraft_kv_k8s_failover
```

If no cluster is available, the example exits early and prints a skip message.
Set `K8S_RUNNER_REQUIRE_CLUSTER=1` to fail instead, as CI does.

# Queue Example

This example runs a small replicated FIFO queue.

The scenarios enqueue messages, dequeue them again, and check that queue state
either converges or drains as expected.

## How TF runs this

Each example follows the same pattern:

- TF starts a small deployment of queue nodes
- a workload produces messages, or produces and consumes them
- an expectation checks either that queue state converges or that the queue drains

## Scenarios

- `basic_convergence` produces messages and checks that queue state converges locally
- `basic_roundtrip` produces and consumes messages locally until the queue drains
- `basic_restart_chaos` injects random local node restarts during the run
- `compose_convergence` and `compose_roundtrip` run the same checks in Docker Compose

## API

Each node exposes:

- `POST /queue/enqueue` to add a message
- `POST /queue/dequeue` to remove a message
- `GET /queue/state` to inspect the current queue state
- `GET /internal/snapshot` to read the local replicated state

## Run locally

```bash
cargo run -p queue-examples --bin queue_basic_convergence
cargo run -p queue-examples --bin queue_basic_roundtrip
cargo run -p queue-examples --bin queue_basic_restart_chaos
```

## Run with Docker Compose

```bash
cargo run -p queue-examples --bin queue_compose_convergence
cargo run -p queue-examples --bin queue_compose_roundtrip
```

Set `QUEUE_IMAGE` to override the default compose image tag.

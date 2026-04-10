# Redis Streams Example

This example uses Redis Streams consumer groups.

There are two scenarios. One produces messages, consumes them, acknowledges
them, and checks that the pending queue drains to zero. The other leaves
messages pending on one consumer and has a second consumer reclaim them with
`XAUTOCLAIM`.

This example is compose-only.

## How TF runs this

Each example follows the same pattern:

- TF starts Redis in Docker Compose
- a workload drives stream and consumer-group operations through the Redis client
- an expectation checks health and pending-message state

## Scenarios

- `compose_roundtrip` covers stream setup, production, consumer-group reads, acknowledgements, and pending drain
- `compose_failover` simulates pending-message reclaim after one consumer stops acknowledging

## Run with Docker Compose

```bash
cargo run -p redis-streams-examples --bin compose_roundtrip
```

## Run the reclaim scenario

```bash
cargo run -p redis-streams-examples --bin compose_failover
```

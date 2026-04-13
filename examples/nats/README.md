# NATS Example

This example uses `nats-server` as the system under test.

The main scenario publishes messages and checks that they can be received back
through NATS. The same behavior can be run against a local `nats-server`
process or a Docker Compose deployment.

## How TF runs this

Each example follows the same pattern:

- TF starts `nats-server` either as a local process or in Docker Compose
- a workload publishes and subscribes through the NATS client
- an expectation checks that the server stays healthy during the run

## Scenarios

- `basic_roundtrip` runs the roundtrip check against a local `nats-server`
- `compose_roundtrip` runs the same check in Docker Compose
- `parity_check` runs compose first and then runs local when `nats-server` is available

## Run locally

```bash
cargo run -p nats-examples --bin basic_roundtrip
```

If `nats-server` is not on `PATH`:

```bash
NATS_SERVER_BIN=/path/to/nats-server cargo run -p nats-examples --bin basic_roundtrip
```

## Run with Docker Compose

```bash
cargo run -p nats-examples --bin compose_roundtrip
```

## Run the parity check

```bash
cargo run -p nats-examples --bin parity_check
```

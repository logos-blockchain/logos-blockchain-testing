# Scheduler Example

This example runs a small replicated job scheduler with worker leases.

The scenario enqueues jobs, lets one worker claim them, stops making progress,
and then checks that another worker can reclaim and complete them after the
lease expires.

## How TF runs this

Each example follows the same pattern:

- TF starts a small deployment of scheduler nodes
- the workload drives the worker flow through the HTTP API
- the expectation checks that jobs are eventually reclaimed and completed

## Scenario

- `basic_failover` runs the failover flow locally
- `compose_failover` runs the same flow in Docker Compose

## API

Each node exposes:

- `POST /jobs/enqueue` to add jobs
- `POST /jobs/claim` to claim pending jobs with a lease
- `POST /jobs/heartbeat` to extend a lease
- `POST /jobs/ack` to mark a job complete
- `GET /jobs/state` to inspect scheduler state
- `GET /internal/snapshot` to read the local replicated state

## Run locally

```bash
cargo run -p scheduler-examples --bin basic_failover
```

## Run with Docker Compose

```bash
cargo run -p scheduler-examples --bin compose_failover
```

Set `SCHEDULER_IMAGE` to override the default compose image tag.

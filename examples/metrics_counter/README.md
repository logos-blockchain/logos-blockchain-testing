# Metrics Counter Example

This example runs a tiny counter service together with a Prometheus scraper.

The scenarios increment counters through the node API and then query Prometheus
to check the aggregate result.

## How TF runs this

Each example follows the same pattern:

- TF starts the counter nodes and a Prometheus scraper
- a workload drives counter increments through the node API
- an expectation, or the manual scenario, checks the Prometheus query result

## API

Each node exposes:

- `POST /counter/inc` to increment the local counter
- `GET /counter/value` to read the current counter value
- `GET /metrics` for Prometheus scraping

## Scenarios

- `compose_prometheus_expectation` runs the app and Prometheus in Docker Compose, then checks the Prometheus query result
- `k8s_prometheus_expectation` runs the same check on Kubernetes
- `k8s_manual_prometheus` starts the nodes through the k8s manual cluster API, restarts one node, and checks the Prometheus aggregate again

## Run with Docker Compose

```bash
LOGOS_BLOCKCHAIN_METRICS_QUERY_URL=http://127.0.0.1:19091 \
cargo run -p metrics-counter-examples --bin metrics_counter_compose_prometheus_expectation
```

## Run with Kubernetes

```bash
docker build -t metrics-counter-node:local -f examples/metrics_counter/Dockerfile .
LOGOS_BLOCKCHAIN_METRICS_QUERY_URL=http://127.0.0.1:30991 \
cargo run -p metrics-counter-examples --bin metrics_counter_k8s_prometheus_expectation
```

Overrides:
- `METRICS_COUNTER_K8S_IMAGE` (falls back to `METRICS_COUNTER_IMAGE`, then `metrics-counter-node:local`)
- `METRICS_COUNTER_K8S_PROMETHEUS_NODE_PORT` (defaults to `30991`)

## Run with Kubernetes manual cluster

```bash
docker build -t metrics-counter-node:local -f examples/metrics_counter/Dockerfile .
LOGOS_BLOCKCHAIN_METRICS_QUERY_URL=http://127.0.0.1:30991 \
cargo run -p metrics-counter-examples --bin metrics_counter_k8s_manual_prometheus
```

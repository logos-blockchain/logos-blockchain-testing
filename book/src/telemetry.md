# Telemetry and External Observability

Telemetry connects a scenario to external observability infrastructure such as Prometheus, an OTLP collector, and Grafana. It supports PromQL queries and external dashboards. For typed application state inside a test, use the [observation runtime](observation.md).

---

## Observation and Telemetry

| | Observation runtime | Telemetry |
|---|---|---|
| What | Typed app state (leaders, keys, heads) | Metrics/logs/traces on external endpoints |
| Where it lives | Inside the test process | Prometheus / OTLP collector / Grafana |
| Consumed by | Workloads and expectations, synchronously | PromQL queries, dashboards, humans |
| Chapter | [Continuous Observation](observation.md) | this one |

Telemetry endpoints are optional on every backend. Without telemetry configuration, the scenario still runs and the cluster's `metrics()` handle has no Prometheus backend.

---

## Declaring Endpoints on the Cluster

`ObservabilityInputs` (`testing-framework/core/src/scenario/observability.rs`) carries three optional URLs:

| Field | Meaning |
|-------|---------|
| `metrics_query_url` | Base URL the runner uses to query Prometheus |
| `metrics_otlp_ingest_url` | OTLP HTTP endpoint nodes export metrics to |
| `grafana_url` | Grafana base URL, for logs/output convenience |

Endpoints travel per cluster on the `ClusterRequest`. Set them with `ClusterApp::with_observability` (or `ClusterRequest::with_observability` inside a custom `AppDeployment`):

```rust,ignore
use testing_framework_core::scenario::ObservabilityInputs;

let inputs = ObservabilityInputs {
    metrics_query_url: Some("http://127.0.0.1:9090".parse()?),
    metrics_otlp_ingest_url: Some("http://127.0.0.1:4318".parse()?),
    grafana_url: None,
};

let scenario = AppHost::scenario()
    .with_app_using(
        ClusterApp::<MetricsCounterEnv>::new(topology).with_observability(inputs),
        ComposeProvisioner::default(),
    )
    .build()?;
```

---

## Environment Plus Request

Backend provisioners resolve effective inputs by merging two sources: environment values form the base, and any endpoint set on the cluster request overrides the corresponding environment value:

```rust,ignore
let inputs = ObservabilityInputs::from_env()?
    .with_overrides(request.observability().clone());
```

This allows the environment to supply infrastructure-specific endpoints while the scenario can override individual URLs per cluster.

**What `from_env` reads.** Verified against the source, it reads exactly three environment variables, each parsed as a URL (empty or unset values are skipped; an unparsable value is an error):

| Env var | Feeds |
|---------|-------|
| `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL` | `metrics_query_url` |
| `LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL` | `metrics_otlp_ingest_url` |
| `LOGOS_BLOCKCHAIN_GRAFANA_URL` | `grafana_url` |

`ObservabilityInputs` also offers `with_overrides(other)` (field-wise, `Some` wins) and `telemetry_handle()`, which builds the `Metrics` value exposed through the `ClusterHandle`: `Metrics::from_prometheus(url)` when `metrics_query_url` is set, `Metrics::empty()` otherwise.

**Backend support today:** the compose and k8s provisioners resolve env + request as above and wire the OTLP ingest URL into node configuration. The local provisioner does not wire telemetry endpoints into node configuration. See the [Capability Matrix](capability-matrix.md).

---

## Querying Metrics in a Run

`ClusterHandle::metrics()` returns the `Metrics` handle for that cluster's endpoints. Backed by Prometheus it evaluates instant queries:

```rust,ignore
let cluster = ctx.require_app::<ClusterHandle<MetricsCounterEnv>>()?;
let telemetry = cluster.metrics()?;
let values = telemetry.instant_values("up")?;          // all sample values
let total = telemetry.counter_value("requests_total")?; // summed counter
```

Without a configured `metrics_query_url`, `is_configured()` is false and query calls return a `MetricsError` ("prometheus endpoint unavailable"). Expectations that assert on metrics therefore require a configured telemetry endpoint.

Telemetry queries depend on scrape intervals, exporter lag, and external infrastructure. Observation polls application state from the test process and reports failures by source. Correctness checks can use observation when they require current typed state; performance checks and post-run analysis can use telemetry.

---

## A Local Stack for Development

To use a local Prometheus, OTLP collector, and Grafana stack, export the three environment variables above or set the URLs on the cluster request. The same scenario binary can then run with or without a metrics backend.

---

## See Also

- [Continuous Observation](observation.md) — test-visible state, the in-process counterpart
- [Capability Matrix](capability-matrix.md) — per-backend telemetry support
- [Environment Variables](environment-variables.md) — the full audited env var list

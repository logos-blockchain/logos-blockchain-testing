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

Telemetry endpoints are optional in every deployer. Without telemetry configuration, the scenario still runs and `RunContext::telemetry()` has no Prometheus backend.

---

## Declaring Endpoints on the Builder

`ObservabilityCapability` (`testing-framework/core/src/scenario/capabilities.rs`) carries three optional URLs:

| Field | Meaning |
|-------|---------|
| `metrics_query_url` | Base URL the runner uses to query Prometheus |
| `metrics_otlp_ingest_url` | OTLP HTTP endpoint nodes export metrics to |
| `grafana_url` | Grafana base URL, for logs/output convenience |

You populate it with `ObservabilityBuilderExt` (`testing-framework/core/src/scenario/builder_ext.rs`), which transitions a plain `ScenarioBuilder<E>` into an `ObservabilityScenarioBuilder<E>`, the capability-typed builder described in [Scenario Capabilities](capabilities.md):

```rust,ignore
use testing_framework_core::scenario::ObservabilityBuilderExt;

let scenario = ScenarioBuilder::with_deployment(topology)
    .with_metrics_query_url_str("http://127.0.0.1:9090")
    .with_metrics_otlp_ingest_url_str("http://127.0.0.1:4318")
    .with_run_duration(Duration::from_secs(60))
    .build()?;
```

Each endpoint has three setter flavors: `with_..._url(Url)`, `with_..._url_str(&str)` (panics on an invalid URL), and `try_with_..._url_str(&str)` (returns `BuilderInputError`).

---

## ObservabilityInputs: Capability Plus Environment

Deployers do not read the capability directly; they resolve an `ObservabilityInputs` (`testing-framework/core/src/scenario/observability.rs`) that merges two sources:

```rust,ignore
let env_inputs = ObservabilityInputs::from_env()?;
let cap_inputs = observability
    .observability_capability()          // via ObservabilityCapabilityProvider
    .map(ObservabilityInputs::from_capability)
    .unwrap_or_default();
let inputs = env_inputs.with_overrides(cap_inputs);
```

The compose and k8s orchestrators use this merge in `testing-framework/deployers/{compose,k8s}/src/deployer/orchestrator.rs`: environment values form the base, and any endpoint set on the scenario capability overrides the corresponding environment value.

This allows the environment to supply infrastructure-specific endpoints while the scenario can override individual URLs on the builder.

**What `from_env` reads.** Verified against the source, it reads exactly three environment variables, each parsed as a URL (empty or unset values are skipped; an unparsable value is an error):

| Env var | Feeds |
|---------|-------|
| `LOGOS_BLOCKCHAIN_METRICS_QUERY_URL` | `metrics_query_url` |
| `LOGOS_BLOCKCHAIN_METRICS_OTLP_INGEST_URL` | `metrics_otlp_ingest_url` |
| `LOGOS_BLOCKCHAIN_GRAFANA_URL` | `grafana_url` |

`ObservabilityInputs` also offers `from_capability(&cap)`, `with_overrides(other)` (field-wise, `Some` wins), and `telemetry_handle()`, which builds the `Metrics` value stored in the `RunContext`: `Metrics::from_prometheus(url)` when `metrics_query_url` is set, `Metrics::empty()` otherwise.

**Deployer support today:** compose and k8s resolve env + capability as above and wire the OTLP ingest URL into node configuration. The local deployer currently builds its runtime with `Metrics::empty()` and does not wire telemetry endpoints. See the [Capability Matrix](capability-matrix.md).

---

## Querying Metrics in a Run

`RunContext::telemetry()` returns the `Metrics` handle. Backed by Prometheus it evaluates instant queries:

```rust,ignore
let telemetry = ctx.telemetry();
let values = telemetry.instant_values("up")?;          // all sample values
let total = telemetry.counter_value("requests_total")?; // summed counter
```

Without a configured `metrics_query_url` these calls return a `MetricsError` ("prometheus endpoint unavailable"). Expectations that assert on metrics therefore require a configured telemetry endpoint.

Telemetry queries depend on scrape intervals, exporter lag, and external infrastructure. Observation polls application state from the test process and reports failures by source. Correctness checks can use observation when they require current typed state; performance checks and post-run analysis can use telemetry.

---

## A Local Stack for Development

To use a local Prometheus, OTLP collector, and Grafana stack, export the three environment variables above or set the URLs on the builder. The same scenario binary can then run with or without a metrics backend.

---

## See Also

- [Continuous Observation](observation.md) — test-visible state, the in-process counterpart
- [Scenario Capabilities](capabilities.md) — how the observability capability is typed
- [Capability Matrix](capability-matrix.md) — per-deployer telemetry support
- [Environment Variables](environment-variables.md) — the full audited env var list

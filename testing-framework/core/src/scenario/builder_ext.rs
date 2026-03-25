use reqwest::Url;

use super::{
    Application, ObservabilityCapability, ScenarioBuilder, internal::ObservabilityScenarioBuilder,
};

const METRICS_QUERY_URL_FIELD: &str = "metrics_query_url";
const METRICS_OTLP_INGEST_URL_FIELD: &str = "metrics_otlp_ingest_url";
const GRAFANA_URL_FIELD: &str = "grafana_url";

#[derive(Debug, thiserror::Error)]
pub enum BuilderInputError {
    #[error("invalid url for {field}: '{value}': {message}")]
    InvalidUrl {
        field: &'static str,
        value: String,
        message: String,
    },
}

fn parse_url_or_panic(field: &'static str, value: &str) -> Url {
    parse_url_field(field, value).unwrap_or_else(|error| panic!("{error}"))
}

fn parse_url_field(field: &'static str, value: &str) -> Result<Url, BuilderInputError> {
    Url::parse(value).map_err(|error| BuilderInputError::InvalidUrl {
        field,
        value: value.to_string(),
        message: error.to_string(),
    })
}

fn apply_parsed_url<T>(
    field: &'static str,
    value: &str,
    apply: impl FnOnce(Url) -> T,
) -> Result<T, BuilderInputError> {
    let parsed = parse_url_field(field, value)?;
    Ok(apply(parsed))
}

fn single_url_observability(
    metrics_query_url: Option<Url>,
    metrics_otlp_ingest_url: Option<Url>,
    grafana_url: Option<Url>,
) -> ObservabilityCapability {
    ObservabilityCapability {
        metrics_query_url,
        metrics_otlp_ingest_url,
        grafana_url,
    }
}

/// Observability helpers for scenarios that want to reuse external telemetry.
pub trait ObservabilityBuilderExt: Sized {
    type Env: Application;

    /// Reuse an existing Prometheus endpoint.
    fn with_metrics_query_url(self, url: Url) -> ObservabilityScenarioBuilder<Self::Env>;

    /// Parse and set the Prometheus endpoint (panics on invalid URL).
    fn with_metrics_query_url_str(self, url: &str) -> ObservabilityScenarioBuilder<Self::Env>;

    /// Parse and set the Prometheus endpoint.
    fn try_with_metrics_query_url_str(
        self,
        url: &str,
    ) -> Result<ObservabilityScenarioBuilder<Self::Env>, BuilderInputError>;

    /// Set the OTLP HTTP ingest endpoint used by nodes.
    fn with_metrics_otlp_ingest_url(self, url: Url) -> ObservabilityScenarioBuilder<Self::Env>;

    /// Parse and set the OTLP ingest endpoint (panics on invalid URL).
    fn with_metrics_otlp_ingest_url_str(self, url: &str)
    -> ObservabilityScenarioBuilder<Self::Env>;

    /// Parse and set the OTLP ingest endpoint.
    fn try_with_metrics_otlp_ingest_url_str(
        self,
        url: &str,
    ) -> Result<ObservabilityScenarioBuilder<Self::Env>, BuilderInputError>;

    /// Set an optional Grafana base URL.
    fn with_grafana_url(self, url: Url) -> ObservabilityScenarioBuilder<Self::Env>;

    /// Parse and set the Grafana URL (panics on invalid URL).
    fn with_grafana_url_str(self, url: &str) -> ObservabilityScenarioBuilder<Self::Env>;

    /// Parse and set the Grafana URL.
    fn try_with_grafana_url_str(
        self,
        url: &str,
    ) -> Result<ObservabilityScenarioBuilder<Self::Env>, BuilderInputError>;
}

impl<E: Application> ObservabilityBuilderExt for ScenarioBuilder<E> {
    type Env = E;

    fn with_metrics_query_url(self, url: Url) -> ObservabilityScenarioBuilder<E> {
        self.with_observability_capability(single_url_observability(Some(url), None, None))
    }

    fn with_metrics_query_url_str(self, url: &str) -> ObservabilityScenarioBuilder<E> {
        self.with_metrics_query_url(parse_url_or_panic(METRICS_QUERY_URL_FIELD, url))
    }

    fn try_with_metrics_query_url_str(
        self,
        url: &str,
    ) -> Result<ObservabilityScenarioBuilder<E>, BuilderInputError> {
        apply_parsed_url(METRICS_QUERY_URL_FIELD, url, |parsed| {
            self.with_metrics_query_url(parsed)
        })
    }

    fn with_metrics_otlp_ingest_url(self, url: Url) -> ObservabilityScenarioBuilder<E> {
        self.with_observability_capability(single_url_observability(None, Some(url), None))
    }

    fn with_metrics_otlp_ingest_url_str(self, url: &str) -> ObservabilityScenarioBuilder<E> {
        self.with_metrics_otlp_ingest_url(parse_url_or_panic(METRICS_OTLP_INGEST_URL_FIELD, url))
    }

    fn try_with_metrics_otlp_ingest_url_str(
        self,
        url: &str,
    ) -> Result<ObservabilityScenarioBuilder<E>, BuilderInputError> {
        apply_parsed_url(METRICS_OTLP_INGEST_URL_FIELD, url, |parsed| {
            self.with_metrics_otlp_ingest_url(parsed)
        })
    }

    fn with_grafana_url(self, url: Url) -> ObservabilityScenarioBuilder<E> {
        self.with_observability_capability(single_url_observability(None, None, Some(url)))
    }

    fn with_grafana_url_str(self, url: &str) -> ObservabilityScenarioBuilder<E> {
        self.with_grafana_url(parse_url_or_panic(GRAFANA_URL_FIELD, url))
    }

    fn try_with_grafana_url_str(
        self,
        url: &str,
    ) -> Result<ObservabilityScenarioBuilder<E>, BuilderInputError> {
        apply_parsed_url(GRAFANA_URL_FIELD, url, |parsed| {
            self.with_grafana_url(parsed)
        })
    }
}

impl<E: Application> ObservabilityBuilderExt for ObservabilityScenarioBuilder<E> {
    type Env = E;

    fn with_metrics_query_url(mut self, url: Url) -> ObservabilityScenarioBuilder<E> {
        self.capabilities_mut().metrics_query_url = Some(url);
        self
    }

    fn with_metrics_query_url_str(self, url: &str) -> ObservabilityScenarioBuilder<E> {
        self.with_metrics_query_url(parse_url_or_panic(METRICS_QUERY_URL_FIELD, url))
    }

    fn try_with_metrics_query_url_str(
        self,
        url: &str,
    ) -> Result<ObservabilityScenarioBuilder<E>, BuilderInputError> {
        apply_parsed_url(METRICS_QUERY_URL_FIELD, url, |parsed| {
            self.with_metrics_query_url(parsed)
        })
    }

    fn with_metrics_otlp_ingest_url(mut self, url: Url) -> ObservabilityScenarioBuilder<E> {
        self.capabilities_mut().metrics_otlp_ingest_url = Some(url);
        self
    }

    fn with_metrics_otlp_ingest_url_str(self, url: &str) -> ObservabilityScenarioBuilder<E> {
        self.with_metrics_otlp_ingest_url(parse_url_or_panic(METRICS_OTLP_INGEST_URL_FIELD, url))
    }

    fn try_with_metrics_otlp_ingest_url_str(
        self,
        url: &str,
    ) -> Result<ObservabilityScenarioBuilder<E>, BuilderInputError> {
        apply_parsed_url(METRICS_OTLP_INGEST_URL_FIELD, url, |parsed| {
            self.with_metrics_otlp_ingest_url(parsed)
        })
    }

    fn with_grafana_url(mut self, url: Url) -> ObservabilityScenarioBuilder<E> {
        self.capabilities_mut().grafana_url = Some(url);
        self
    }

    fn with_grafana_url_str(self, url: &str) -> ObservabilityScenarioBuilder<E> {
        self.with_grafana_url(parse_url_or_panic(GRAFANA_URL_FIELD, url))
    }

    fn try_with_grafana_url_str(
        self,
        url: &str,
    ) -> Result<ObservabilityScenarioBuilder<E>, BuilderInputError> {
        apply_parsed_url(GRAFANA_URL_FIELD, url, |parsed| {
            self.with_grafana_url(parsed)
        })
    }
}

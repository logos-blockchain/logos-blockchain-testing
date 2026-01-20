use nomos_tracing::{
    logging::{local::FileConfig, loki::LokiConfig},
    metrics::otlp::OtlpMetricsConfig,
    tracing::otlp::OtlpTracingConfig,
};
use nomos_tracing_service::{
    ConsoleLayer, FilterLayer, LoggerLayer, MetricsLayer, TracingLayer, TracingSettings,
};
use testing_framework_env as tf_env;
use tracing::Level;

use crate::IS_DEBUG_TRACING;

#[derive(Clone, Default)]
pub struct GeneralTracingConfig {
    pub tracing_settings: TracingSettings,
}

impl GeneralTracingConfig {
    fn local_debug_tracing(id: usize) -> Self {
        let host_identifier = format!("node-{id}");
        let otlp_tracing = otlp_tracing_endpoint()
            .and_then(|endpoint| endpoint.parse().ok())
            .map(|endpoint| {
                TracingLayer::Otlp(OtlpTracingConfig {
                    endpoint,
                    sample_ratio: 0.5,
                    service_name: host_identifier.clone(),
                })
            })
            .unwrap_or(TracingLayer::None);
        let otlp_metrics = otlp_metrics_endpoint()
            .and_then(|endpoint| endpoint.parse().ok())
            .map(|endpoint| {
                MetricsLayer::Otlp(OtlpMetricsConfig {
                    endpoint,
                    host_identifier: host_identifier.clone(),
                })
            })
            .unwrap_or(MetricsLayer::None);

        let filter = file_filter_override().unwrap_or_else(|| {
            nomos_tracing::filter::envfilter::EnvFilterConfig {
                filters: std::iter::once(&("nomos", "debug"))
                    .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                    .collect(),
            }
        });

        Self {
            tracing_settings: TracingSettings {
                logger: LoggerLayer::Loki(LokiConfig {
                    endpoint: "http://localhost:3100".parse().unwrap_or_else(|_| unsafe {
                        // Safety: the URL is a valid constant.
                        std::hint::unreachable_unchecked()
                    }),
                    host_identifier: host_identifier.clone(),
                }),
                tracing: otlp_tracing,
                filter: FilterLayer::EnvFilter(filter),
                metrics: otlp_metrics,
                console: ConsoleLayer::None,
                level: Level::DEBUG,
            },
        }
    }
}

fn otlp_tracing_endpoint() -> Option<String> {
    tf_env::nomos_otlp_endpoint()
}

fn otlp_metrics_endpoint() -> Option<String> {
    tf_env::nomos_otlp_metrics_endpoint()
}

#[must_use]
pub fn create_tracing_configs(ids: &[[u8; 32]]) -> Vec<GeneralTracingConfig> {
    if *IS_DEBUG_TRACING {
        create_debug_configs(ids)
    } else {
        create_default_configs(ids)
    }
}

fn create_debug_configs(ids: &[[u8; 32]]) -> Vec<GeneralTracingConfig> {
    ids.iter()
        .enumerate()
        .map(|(i, _)| (i, GeneralTracingConfig::local_debug_tracing(i)))
        .map(|(i, cfg)| apply_file_logger_override(cfg, i))
        .map(maybe_disable_otlp_layers)
        .collect()
}

fn create_default_configs(ids: &[[u8; 32]]) -> Vec<GeneralTracingConfig> {
    ids.iter()
        .enumerate()
        .map(|(i, _)| (i, GeneralTracingConfig::default()))
        .map(|(i, cfg)| apply_file_logger_override(cfg, i))
        .map(maybe_disable_otlp_layers)
        .collect()
}

fn apply_file_logger_override(
    mut cfg: GeneralTracingConfig,
    node_index: usize,
) -> GeneralTracingConfig {
    if let Some(directory) = tf_env::nomos_log_dir() {
        cfg.tracing_settings.logger = LoggerLayer::File(FileConfig {
            directory,
            prefix: Some(format!("logos-blockchain-node-{node_index}").into()),
        });
        cfg.tracing_settings.level = file_log_level();
    }
    cfg
}

fn file_log_level() -> Level {
    tf_env::nomos_log_level()
        .and_then(|raw| raw.parse::<Level>().ok())
        .unwrap_or(Level::INFO)
}

fn file_filter_override() -> Option<nomos_tracing::filter::envfilter::EnvFilterConfig> {
    tf_env::nomos_log_filter().map(|raw| nomos_tracing::filter::envfilter::EnvFilterConfig {
        filters: raw
            .split(',')
            .filter_map(|pair| {
                let mut parts = pair.splitn(2, '=');
                let target = parts.next()?.trim().to_string();
                let level = parts.next()?.trim().to_string();
                (!target.is_empty() && !level.is_empty()).then_some((target, level))
            })
            .collect(),
    })
}

fn maybe_disable_otlp_layers(mut cfg: GeneralTracingConfig) -> GeneralTracingConfig {
    if otlp_tracing_endpoint().is_none() {
        cfg.tracing_settings.tracing = TracingLayer::None;
    }
    if otlp_metrics_endpoint().is_none() {
        cfg.tracing_settings.metrics = MetricsLayer::None;
    }
    cfg
}

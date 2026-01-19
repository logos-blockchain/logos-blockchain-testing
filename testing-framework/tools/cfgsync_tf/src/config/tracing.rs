use nomos_tracing_service::{LoggerLayer, MetricsLayer, TracingLayer, TracingSettings};
use testing_framework_config::topology::configs::tracing::GeneralTracingConfig;

pub fn update_tracing_identifier(
    settings: TracingSettings,
    identifier: String,
) -> GeneralTracingConfig {
    GeneralTracingConfig {
        tracing_settings: TracingSettings {
            logger: match settings.logger {
                LoggerLayer::Loki(mut config) => {
                    config.host_identifier.clone_from(&identifier);
                    LoggerLayer::Loki(config)
                }
                other => other,
            },
            tracing: match settings.tracing {
                TracingLayer::Otlp(mut config) => {
                    config.service_name.clone_from(&identifier);
                    TracingLayer::Otlp(config)
                }
                other @ TracingLayer::None => other,
            },
            filter: settings.filter,
            metrics: match settings.metrics {
                MetricsLayer::Otlp(mut config) => {
                    config.host_identifier = identifier;
                    MetricsLayer::Otlp(config)
                }
                other @ MetricsLayer::None => other,
            },
            console: settings.console,
            level: settings.level,
        },
    }
}

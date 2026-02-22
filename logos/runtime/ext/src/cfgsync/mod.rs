mod template;

use std::path::Path;

use anyhow::Result;
pub(crate) use cfgsync_runtime::render::CfgsyncOutputPaths;
use cfgsync_runtime::{
    bundle::build_cfgsync_bundle_with_hostnames,
    render::{RenderedCfgsync, apply_timeout_floor, ensure_bundle_path, write_rendered_cfgsync},
};
use lb_tracing::metrics::otlp::OtlpMetricsConfig;
use lb_tracing_service::MetricsLayerSettings;
use reqwest::Url;
use testing_framework_core::cfgsync::CfgsyncEnv;

pub(crate) struct CfgsyncRenderOptions {
    pub port: Option<u16>,
    pub bundle_path: Option<String>,
    pub min_timeout_secs: Option<u64>,
    pub metrics_otlp_ingest_url: Option<Url>,
}

pub(crate) fn render_cfgsync_from_template<E: CfgsyncEnv>(
    template_path: &Path,
    topology: &E::Deployment,
    hostnames: &[String],
    options: CfgsyncRenderOptions,
) -> Result<RenderedCfgsync> {
    let mut cfg = template::load_cfgsync_template(template_path)?;
    apply_render_options::<E>(&mut cfg, topology, options);

    let bundle = build_cfgsync_bundle_with_hostnames::<E>(topology, hostnames)?;
    let config_yaml = serde_yaml::to_string(&cfg)?;
    let bundle_yaml = serde_yaml::to_string(&bundle)?;

    Ok(RenderedCfgsync {
        config_yaml,
        bundle_yaml,
    })
}

pub(crate) fn render_and_write_cfgsync_from_template<E: CfgsyncEnv>(
    template_path: &Path,
    topology: &E::Deployment,
    hostnames: &[String],
    mut options: CfgsyncRenderOptions,
    output: CfgsyncOutputPaths<'_>,
) -> Result<RenderedCfgsync> {
    ensure_bundle_path(&mut options.bundle_path, output.bundle_path);

    let rendered = render_cfgsync_from_template::<E>(template_path, topology, hostnames, options)?;
    write_rendered_cfgsync(&rendered, output)?;
    Ok(rendered)
}

fn apply_render_options<E: CfgsyncEnv>(
    cfg: &mut template::CfgSyncConfig,
    topology: &E::Deployment,
    options: CfgsyncRenderOptions,
) {
    let CfgsyncRenderOptions {
        port,
        bundle_path,
        min_timeout_secs,
        metrics_otlp_ingest_url,
    } = options;

    if let Some(port) = port {
        cfg.port = port;
    }

    cfg.n_hosts = E::nodes(topology).len();
    cfg.bundle_path = bundle_path;
    apply_metrics_endpoint(cfg, metrics_otlp_ingest_url);
    apply_timeout_floor(&mut cfg.timeout, min_timeout_secs);
}

fn apply_metrics_endpoint(cfg: &mut template::CfgSyncConfig, endpoint: Option<Url>) {
    if let Some(endpoint) = endpoint {
        cfg.tracing_settings.metrics = MetricsLayerSettings::Otlp(OtlpMetricsConfig {
            endpoint,
            host_identifier: "node".into(),
        });
    }
}

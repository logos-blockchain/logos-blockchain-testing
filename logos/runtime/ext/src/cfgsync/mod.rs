use anyhow::Result;
pub(crate) use cfgsync_runtime::render::CfgsyncOutputPaths;
use cfgsync_runtime::{
    bundle::build_cfgsync_bundle_with_hostnames,
    render::{
        CfgsyncConfigOverrides, RenderedCfgsync, ensure_bundle_path,
        render_cfgsync_yaml_from_template, write_rendered_cfgsync,
    },
};
use reqwest::Url;
use serde_yaml::{Mapping, Value};
use testing_framework_core::cfgsync::CfgsyncEnv;

pub(crate) struct CfgsyncRenderOptions {
    pub port: Option<u16>,
    pub bundle_path: Option<String>,
    pub min_timeout_secs: Option<u64>,
    pub metrics_otlp_ingest_url: Option<Url>,
}

pub(crate) fn render_cfgsync_from_template<E: CfgsyncEnv>(
    topology: &E::Deployment,
    hostnames: &[String],
    options: CfgsyncRenderOptions,
) -> Result<RenderedCfgsync> {
    let cfg = build_cfgsync_server_config();
    let overrides = build_overrides::<E>(topology, options);
    let config_yaml = render_cfgsync_yaml_from_template(cfg, &overrides)?;
    let bundle = build_cfgsync_bundle_with_hostnames::<E>(topology, hostnames)?;
    let bundle_yaml = serde_yaml::to_string(&bundle)?;

    Ok(RenderedCfgsync {
        config_yaml,
        bundle_yaml,
    })
}

fn build_cfgsync_server_config() -> Value {
    let mut root = Mapping::new();
    root.insert(
        Value::String("port".to_string()),
        Value::Number(4400_u64.into()),
    );

    root.insert(
        Value::String("bundle_path".to_string()),
        Value::String("cfgsync.bundle.yaml".to_string()),
    );

    Value::Mapping(root)
}

pub(crate) fn render_and_write_cfgsync_from_template<E: CfgsyncEnv>(
    topology: &E::Deployment,
    hostnames: &[String],
    mut options: CfgsyncRenderOptions,
    output: CfgsyncOutputPaths<'_>,
) -> Result<RenderedCfgsync> {
    ensure_bundle_path(&mut options.bundle_path, output.bundle_path);

    let rendered = render_cfgsync_from_template::<E>(topology, hostnames, options)?;
    write_rendered_cfgsync(&rendered, output)?;

    Ok(rendered)
}

fn build_overrides<E: CfgsyncEnv>(
    topology: &E::Deployment,
    options: CfgsyncRenderOptions,
) -> CfgsyncConfigOverrides {
    let CfgsyncRenderOptions {
        port,
        bundle_path,
        min_timeout_secs,
        metrics_otlp_ingest_url,
    } = options;

    CfgsyncConfigOverrides {
        port,
        n_hosts: Some(E::nodes(topology).len()),
        timeout_floor_secs: min_timeout_secs,
        bundle_path,
        metrics_otlp_ingest_url: metrics_otlp_ingest_url.map(|url| url.to_string()),
    }
}

use anyhow::Result;
use cfgsync_artifacts::ArtifactFile;
pub(crate) use cfgsync_core::render::CfgsyncOutputPaths;
use cfgsync_core::render::{
    CfgsyncConfigOverrides, RenderedCfgsync, ensure_artifacts_path,
    render_cfgsync_yaml_from_template, write_rendered_cfgsync,
};
use reqwest::Url;
use serde_yaml::{Mapping, Value};
use testing_framework_core::cfgsync::{StaticArtifactRenderer, build_static_artifacts};
use thiserror::Error;

pub(crate) struct CfgsyncRenderOptions {
    pub port: Option<u16>,
    pub artifacts_path: Option<String>,
    pub min_timeout_secs: Option<u64>,
    pub metrics_otlp_ingest_url: Option<Url>,
}

#[derive(Debug, Error)]
enum BundleRenderError {
    #[error("cfgsync bundle node `{identifier}` is missing `/config.yaml`")]
    MissingConfigFile { identifier: String },
    #[error("cfgsync config file is missing `{key}`")]
    MissingYamlKey { key: String },
}

pub(crate) fn render_cfgsync_from_template<E: StaticArtifactRenderer>(
    topology: &E::Deployment,
    hostnames: &[String],
    options: CfgsyncRenderOptions,
) -> Result<RenderedCfgsync> {
    let cfg = build_cfgsync_server_config();
    let overrides = build_overrides::<E>(topology, options);
    let config_yaml = render_cfgsync_yaml_from_template(cfg, &overrides)?;
    let mut materialized = build_static_artifacts::<E>(topology, hostnames)?;
    append_deployment_files(&mut materialized)?;
    let artifacts_yaml = serde_yaml::to_string(&materialized)?;

    Ok(RenderedCfgsync {
        config_yaml,
        artifacts_yaml,
    })
}

fn append_deployment_files(
    materialized: &mut cfgsync_adapter::MaterializedArtifacts,
) -> Result<()> {
    if has_shared_file_path(materialized, "/deployment.yaml") {
        return Ok(());
    }

    let Some((identifier, artifacts)) = materialized.iter().next() else {
        return Ok(());
    };

    let config_content =
        config_file_content(artifacts).ok_or_else(|| BundleRenderError::MissingConfigFile {
            identifier: identifier.to_owned(),
        })?;
    let deployment_yaml = extract_yaml_key(&config_content, "deployment")?;

    let mut shared = materialized.shared().clone();
    shared
        .files
        .push(build_artifact_file("/deployment.yaml", deployment_yaml));
    *materialized = materialized.clone().with_shared(shared);

    Ok(())
}

fn has_shared_file_path(materialized: &cfgsync_adapter::MaterializedArtifacts, path: &str) -> bool {
    materialized
        .shared()
        .files
        .iter()
        .any(|file| file.path == path)
}

fn config_file_content(artifacts: &cfgsync_artifacts::ArtifactSet) -> Option<String> {
    artifacts
        .files
        .iter()
        .find_map(|file| (file.path == "/config.yaml").then_some(file.content.clone()))
}

fn build_artifact_file(path: &str, content: String) -> ArtifactFile {
    ArtifactFile::new(path, content)
}

fn extract_yaml_key(content: &str, key: &str) -> Result<String> {
    let document: Value = serde_yaml::from_str(content)?;
    let value = document
        .get(key)
        .cloned()
        .ok_or_else(|| BundleRenderError::MissingYamlKey {
            key: key.to_owned(),
        })?;

    Ok(serde_yaml::to_string(&value)?)
}

fn build_cfgsync_server_config() -> Value {
    let mut root = Mapping::new();
    root.insert(
        Value::String("port".to_string()),
        Value::Number(4400_u64.into()),
    );

    let mut source = Mapping::new();
    source.insert(
        Value::String("kind".to_string()),
        Value::String("registration".to_string()),
    );
    source.insert(
        Value::String("artifacts_path".to_string()),
        Value::String("cfgsync.artifacts.yaml".to_string()),
    );

    root.insert(Value::String("source".to_string()), Value::Mapping(source));

    Value::Mapping(root)
}

pub(crate) fn render_and_write_cfgsync_from_template<E: StaticArtifactRenderer>(
    topology: &E::Deployment,
    hostnames: &[String],
    mut options: CfgsyncRenderOptions,
    output: CfgsyncOutputPaths<'_>,
) -> Result<RenderedCfgsync> {
    ensure_artifacts_path(&mut options.artifacts_path, output.artifacts_path);

    let rendered = render_cfgsync_from_template::<E>(topology, hostnames, options)?;
    write_rendered_cfgsync(&rendered, output)?;

    Ok(rendered)
}

fn build_overrides<E: StaticArtifactRenderer>(
    topology: &E::Deployment,
    options: CfgsyncRenderOptions,
) -> CfgsyncConfigOverrides {
    let CfgsyncRenderOptions {
        port,
        artifacts_path,
        min_timeout_secs,
        metrics_otlp_ingest_url,
    } = options;

    CfgsyncConfigOverrides {
        port,
        n_hosts: Some(E::nodes(topology).len()),
        timeout_floor_secs: min_timeout_secs,
        artifacts_path,
        metrics_otlp_ingest_url: metrics_otlp_ingest_url.map(|url| url.to_string()),
    }
}

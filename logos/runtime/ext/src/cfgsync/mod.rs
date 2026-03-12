use anyhow::Result;
use cfgsync_adapter::static_deployment::{DeploymentAdapter, build_materialized_artifacts};
pub(crate) use cfgsync_core::render::CfgsyncOutputPaths;
use cfgsync_core::{
    NodeArtifactsBundle, NodeArtifactsBundleEntry,
    render::{
        CfgsyncConfigOverrides, RenderedCfgsync, ensure_bundle_path,
        render_cfgsync_yaml_from_template, write_rendered_cfgsync,
    },
};
use reqwest::Url;
use serde_yaml::{Mapping, Value};
use thiserror::Error;

pub(crate) struct CfgsyncRenderOptions {
    pub port: Option<u16>,
    pub bundle_path: Option<String>,
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

pub(crate) fn render_cfgsync_from_template<E: DeploymentAdapter>(
    topology: &E::Deployment,
    hostnames: &[String],
    options: CfgsyncRenderOptions,
) -> Result<RenderedCfgsync> {
    let cfg = build_cfgsync_server_config();
    let overrides = build_overrides::<E>(topology, options);
    let config_yaml = render_cfgsync_yaml_from_template(cfg, &overrides)?;
    let mut bundle = build_cfgsync_bundle::<E>(topology, hostnames)?;
    append_deployment_files(&mut bundle)?;
    let bundle_yaml = serde_yaml::to_string(&bundle)?;

    Ok(RenderedCfgsync {
        config_yaml,
        bundle_yaml,
    })
}

fn build_cfgsync_bundle<E: DeploymentAdapter>(
    topology: &E::Deployment,
    hostnames: &[String],
) -> Result<NodeArtifactsBundle> {
    let materialized = build_materialized_artifacts::<E>(topology, hostnames)?;
    let nodes = materialized
        .iter()
        .map(|(identifier, artifacts)| NodeArtifactsBundleEntry {
            identifier: identifier.to_owned(),
            files: artifacts.files.clone(),
        })
        .collect();

    Ok(NodeArtifactsBundle::new(nodes).with_shared_files(materialized.shared().files.clone()))
}

fn append_deployment_files(bundle: &mut NodeArtifactsBundle) -> Result<()> {
    if has_shared_file_path(bundle, "/deployment.yaml") {
        return Ok(());
    }

    let Some(node) = bundle.nodes.first() else {
        return Ok(());
    };

    let config_content =
        config_file_content(node).ok_or_else(|| BundleRenderError::MissingConfigFile {
            identifier: node.identifier.clone(),
        })?;
    let deployment_yaml = extract_yaml_key(&config_content, "deployment")?;

    bundle
        .shared_files
        .push(build_bundle_file("/deployment.yaml", deployment_yaml));

    Ok(())
}

fn has_shared_file_path(bundle: &NodeArtifactsBundle, path: &str) -> bool {
    bundle.shared_files.iter().any(|file| file.path == path)
}

fn config_file_content(node: &NodeArtifactsBundleEntry) -> Option<String> {
    node.files
        .iter()
        .find_map(|file| (file.path == "/config.yaml").then_some(file.content.clone()))
}

fn build_bundle_file(path: &str, content: String) -> cfgsync_core::NodeArtifactFile {
    cfgsync_core::NodeArtifactFile {
        path: path.to_owned(),
        content,
    }
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
        Value::String("registration_bundle".to_string()),
    );
    source.insert(
        Value::String("bundle_path".to_string()),
        Value::String("cfgsync.bundle.yaml".to_string()),
    );

    root.insert(Value::String("source".to_string()), Value::Mapping(source));

    Value::Mapping(root)
}

pub(crate) fn render_and_write_cfgsync_from_template<E: DeploymentAdapter>(
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

fn build_overrides<E: DeploymentAdapter>(
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

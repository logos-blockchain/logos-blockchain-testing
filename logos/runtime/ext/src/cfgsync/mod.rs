use anyhow::Result;
use cfgsync_adapter::{CfgsyncEnv, build_cfgsync_node_catalog};
pub(crate) use cfgsync_core::render::CfgsyncOutputPaths;
use cfgsync_core::{
    CfgSyncBundle, CfgSyncBundleNode,
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

pub(crate) fn render_cfgsync_from_template<E: CfgsyncEnv>(
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

fn build_cfgsync_bundle<E: CfgsyncEnv>(
    topology: &E::Deployment,
    hostnames: &[String],
) -> Result<CfgSyncBundle> {
    let nodes = build_cfgsync_node_catalog::<E>(topology, hostnames)?.into_configs();
    let nodes = nodes
        .into_iter()
        .map(|node| CfgSyncBundleNode {
            identifier: node.identifier,
            files: node.files,
        })
        .collect();

    Ok(CfgSyncBundle::new(nodes))
}

fn append_deployment_files(bundle: &mut CfgSyncBundle) -> Result<()> {
    for node in &mut bundle.nodes {
        if has_file_path(node, "/deployment.yaml") {
            continue;
        }

        let config_content =
            config_file_content(node).ok_or_else(|| BundleRenderError::MissingConfigFile {
                identifier: node.identifier.clone(),
            })?;
        let deployment_yaml = extract_yaml_key(&config_content, "deployment")?;

        node.files
            .push(build_bundle_file("/deployment.yaml", deployment_yaml));
    }

    Ok(())
}

fn has_file_path(node: &CfgSyncBundleNode, path: &str) -> bool {
    node.files.iter().any(|file| file.path == path)
}

fn config_file_content(node: &CfgSyncBundleNode) -> Option<String> {
    node.files
        .iter()
        .find_map(|file| (file.path == "/config.yaml").then_some(file.content.clone()))
}

fn build_bundle_file(path: &str, content: String) -> cfgsync_core::CfgSyncFile {
    cfgsync_core::CfgSyncFile {
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

    root.insert(
        Value::String("bundle_path".to_string()),
        Value::String("cfgsync.bundle.yaml".to_string()),
    );

    root.insert(
        Value::String("serving_mode".to_string()),
        Value::String("registration".to_string()),
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

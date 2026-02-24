use std::{fs, path::Path};

use anyhow::{Context as _, Result};
use serde_yaml::{Mapping, Value};

#[derive(Debug, Clone)]
pub struct RenderedCfgsync {
    pub config_yaml: String,
    pub bundle_yaml: String,
}

#[derive(Debug, Clone, Copy)]
pub struct CfgsyncOutputPaths<'a> {
    pub config_path: &'a Path,
    pub bundle_path: &'a Path,
}

pub fn ensure_bundle_path(bundle_path: &mut Option<String>, output_bundle_path: &Path) {
    if bundle_path.is_some() {
        return;
    }

    *bundle_path = Some(
        output_bundle_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("cfgsync.bundle.yaml")
            .to_string(),
    );
}

pub fn apply_timeout_floor(timeout: &mut u64, min_timeout_secs: Option<u64>) {
    if let Some(min_timeout_secs) = min_timeout_secs {
        *timeout = (*timeout).max(min_timeout_secs);
    }
}

pub fn write_rendered_cfgsync(
    rendered: &RenderedCfgsync,
    output: CfgsyncOutputPaths<'_>,
) -> Result<()> {
    fs::write(output.config_path, &rendered.config_yaml)?;
    fs::write(output.bundle_path, &rendered.bundle_yaml)?;
    Ok(())
}

#[derive(Debug, Clone, Default)]
pub struct CfgsyncConfigOverrides {
    pub port: Option<u16>,
    pub n_hosts: Option<usize>,
    pub timeout_floor_secs: Option<u64>,
    pub bundle_path: Option<String>,
    pub metrics_otlp_ingest_url: Option<String>,
}

pub fn load_cfgsync_template_yaml(path: &Path) -> Result<Value> {
    let file = fs::File::open(path)
        .with_context(|| format!("opening cfgsync template at {}", path.display()))?;
    serde_yaml::from_reader(file).context("parsing cfgsync template")
}

pub fn render_cfgsync_yaml_from_template(
    mut template: Value,
    overrides: &CfgsyncConfigOverrides,
) -> Result<String> {
    apply_cfgsync_overrides(&mut template, overrides)?;
    serde_yaml::to_string(&template).context("serializing rendered cfgsync config")
}

pub fn apply_cfgsync_overrides(
    template: &mut Value,
    overrides: &CfgsyncConfigOverrides,
) -> Result<()> {
    let root = mapping_mut(template)?;

    if let Some(port) = overrides.port {
        root.insert(
            Value::String("port".to_string()),
            serde_yaml::to_value(port)?,
        );
    }

    if let Some(n_hosts) = overrides.n_hosts {
        root.insert(
            Value::String("n_hosts".to_string()),
            serde_yaml::to_value(n_hosts)?,
        );
    }

    if let Some(bundle_path) = &overrides.bundle_path {
        root.insert(
            Value::String("bundle_path".to_string()),
            Value::String(bundle_path.clone()),
        );
    }

    if let Some(timeout_floor_secs) = overrides.timeout_floor_secs {
        let timeout_key = Value::String("timeout".to_string());
        let timeout_value = root
            .get(&timeout_key)
            .and_then(Value::as_u64)
            .unwrap_or(timeout_floor_secs)
            .max(timeout_floor_secs);
        root.insert(timeout_key, serde_yaml::to_value(timeout_value)?);
    }

    if let Some(endpoint) = &overrides.metrics_otlp_ingest_url {
        let tracing_settings = nested_mapping_mut(root, "tracing_settings");
        tracing_settings.insert(
            Value::String("metrics".to_string()),
            parse_otlp_metrics_layer(endpoint)?,
        );
    }

    Ok(())
}

fn mapping_mut(value: &mut Value) -> Result<&mut Mapping> {
    value
        .as_mapping_mut()
        .context("cfgsync template root must be a YAML map")
}

fn nested_mapping_mut<'a>(mapping: &'a mut Mapping, key: &str) -> &'a mut Mapping {
    let key = Value::String(key.to_string());
    let entry = mapping
        .entry(key)
        .or_insert_with(|| Value::Mapping(Mapping::new()));

    if !entry.is_mapping() {
        *entry = Value::Mapping(Mapping::new());
    }

    entry
        .as_mapping_mut()
        .expect("mapping entry should always be a mapping")
}

fn parse_otlp_metrics_layer(endpoint: &str) -> Result<Value> {
    let yaml = format!("!Otlp\nendpoint: \"{endpoint}\"\nhost_identifier: \"node\"\n");
    serde_yaml::from_str(&yaml).context("building metrics OTLP layer")
}

use std::{fs, path::Path};

use anyhow::{Context as _, Result};
use serde_yaml::{Mapping, Value};
use thiserror::Error;

/// Rendered cfgsync outputs written for server startup.
#[derive(Debug, Clone)]
pub struct RenderedCfgsync {
    /// Serialized cfgsync server config YAML.
    pub config_yaml: String,
    /// Serialized precomputed artifact YAML used by cfgsync runtime.
    pub artifacts_yaml: String,
}

/// Output paths used when materializing rendered cfgsync files.
#[derive(Debug, Clone, Copy)]
pub struct CfgsyncOutputPaths<'a> {
    /// Output path for the rendered server config YAML.
    pub config_path: &'a Path,
    /// Output path for the rendered precomputed artifacts YAML.
    pub artifacts_path: &'a Path,
}

/// Ensures artifacts path override exists, defaulting to the output artifacts
/// file name.
pub fn ensure_artifacts_path(artifacts_path: &mut Option<String>, output_artifacts_path: &Path) {
    if artifacts_path.is_some() {
        return;
    }

    *artifacts_path = Some(
        output_artifacts_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("cfgsync.artifacts.yaml")
            .to_string(),
    );
}

/// Applies a minimum timeout floor to an existing timeout value.
pub fn apply_timeout_floor(timeout: &mut u64, min_timeout_secs: Option<u64>) {
    if let Some(min_timeout_secs) = min_timeout_secs {
        *timeout = (*timeout).max(min_timeout_secs);
    }
}

/// Writes rendered cfgsync server and bundle YAML files.
pub fn write_rendered_cfgsync(
    rendered: &RenderedCfgsync,
    output: CfgsyncOutputPaths<'_>,
) -> Result<()> {
    fs::write(output.config_path, &rendered.config_yaml)?;
    fs::write(output.artifacts_path, &rendered.artifacts_yaml)?;
    Ok(())
}

/// Optional overrides applied to a cfgsync template document.
#[derive(Debug, Clone, Default)]
pub struct CfgsyncConfigOverrides {
    /// Override for the HTTP listen port.
    pub port: Option<u16>,
    /// Override for the expected initial host count.
    pub n_hosts: Option<usize>,
    /// Minimum timeout to enforce on the rendered template.
    pub timeout_floor_secs: Option<u64>,
    /// Override for the precomputed artifacts path written into cfgsync config.
    pub artifacts_path: Option<String>,
    /// Optional OTLP metrics endpoint injected into tracing settings.
    pub metrics_otlp_ingest_url: Option<String>,
}

#[derive(Debug, Error)]
enum RenderTemplateError {
    #[error("cfgsync template key `{key}` must be a YAML map")]
    NonMappingEntry { key: String },
}

/// Loads cfgsync template YAML from disk.
pub fn load_cfgsync_template_yaml(path: &Path) -> Result<Value> {
    let file = fs::File::open(path)
        .with_context(|| format!("opening cfgsync template at {}", path.display()))?;
    serde_yaml::from_reader(file).context("parsing cfgsync template")
}

/// Renders cfgsync config YAML by applying overrides to a template document.
pub fn render_cfgsync_yaml_from_template(
    mut template: Value,
    overrides: &CfgsyncConfigOverrides,
) -> Result<String> {
    apply_cfgsync_overrides(&mut template, overrides)?;
    serde_yaml::to_string(&template).context("serializing rendered cfgsync config")
}

/// Applies cfgsync-specific override fields to a mutable YAML document.
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

    if let Some(artifacts_path) = &overrides.artifacts_path {
        root.insert(
            Value::String("artifacts_path".to_string()),
            Value::String(artifacts_path.clone()),
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
        let tracing_settings = nested_mapping_mut(root, "tracing_settings")?;
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

fn nested_mapping_mut<'a>(mapping: &'a mut Mapping, key: &str) -> Result<&'a mut Mapping> {
    let key_name = key.to_owned();
    let key = Value::String(key_name.clone());
    let entry = mapping
        .entry(key)
        .or_insert_with(|| Value::Mapping(Mapping::new()));

    if !entry.is_mapping() {
        return Err(RenderTemplateError::NonMappingEntry { key: key_name }).map_err(Into::into);
    }

    entry
        .as_mapping_mut()
        .context("cfgsync template entry should be a YAML map")
}

fn parse_otlp_metrics_layer(endpoint: &str) -> Result<Value> {
    let yaml = format!("!Otlp\nendpoint: \"{endpoint}\"\nhost_identifier: \"node\"\n");
    serde_yaml::from_str(&yaml).context("building metrics OTLP layer")
}

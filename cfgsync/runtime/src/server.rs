use std::{fs, path::Path, sync::Arc};

use anyhow::Context as _;
use cfgsync_adapter::{NodeArtifacts, NodeArtifactsCatalog, RegistrationConfigProvider};
use cfgsync_core::{
    BundleConfigSource, CfgsyncServerState, NodeArtifactsBundle, NodeConfigSource, run_cfgsync,
};
use serde::Deserialize;
use thiserror::Error;

/// Runtime cfgsync server config loaded from YAML.
#[derive(Debug, Deserialize, Clone)]
pub struct CfgsyncServerConfig {
    pub port: u16,
    pub bundle_path: String,
    #[serde(default)]
    pub serving_mode: CfgsyncServingMode,
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CfgsyncServingMode {
    #[default]
    Bundle,
    Registration,
}

#[derive(Debug, Deserialize)]
struct RawCfgsyncServerConfig {
    port: u16,
    bundle_path: String,
    #[serde(default)]
    serving_mode: Option<CfgsyncServingMode>,
    #[serde(default)]
    registration_flow: Option<bool>,
}

#[derive(Debug, Error)]
pub enum LoadCfgsyncServerConfigError {
    #[error("failed to read cfgsync config file {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse cfgsync config file {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
}

impl CfgsyncServerConfig {
    /// Loads cfgsync runtime server config from a YAML file.
    pub fn load_from_file(path: &Path) -> Result<Self, LoadCfgsyncServerConfigError> {
        let config_path = path.display().to_string();
        let config_content =
            fs::read_to_string(path).map_err(|source| LoadCfgsyncServerConfigError::Read {
                path: config_path.clone(),
                source,
            })?;

        let raw: RawCfgsyncServerConfig =
            serde_yaml::from_str(&config_content).map_err(|source| {
                LoadCfgsyncServerConfigError::Parse {
                    path: config_path,
                    source,
                }
            })?;

        Ok(Self {
            port: raw.port,
            bundle_path: raw.bundle_path,
            serving_mode: raw
                .serving_mode
                .unwrap_or_else(|| mode_from_legacy_registration_flow(raw.registration_flow)),
        })
    }

    #[must_use]
    pub fn for_bundle(port: u16, bundle_path: impl Into<String>) -> Self {
        Self {
            port,
            bundle_path: bundle_path.into(),
            serving_mode: CfgsyncServingMode::Bundle,
        }
    }

    #[must_use]
    pub fn for_registration(port: u16, bundle_path: impl Into<String>) -> Self {
        Self {
            port,
            bundle_path: bundle_path.into(),
            serving_mode: CfgsyncServingMode::Registration,
        }
    }
}

fn mode_from_legacy_registration_flow(registration_flow: Option<bool>) -> CfgsyncServingMode {
    if registration_flow.unwrap_or(false) {
        CfgsyncServingMode::Registration
    } else {
        CfgsyncServingMode::Bundle
    }
}

fn load_bundle_provider(bundle_path: &Path) -> anyhow::Result<Arc<dyn NodeConfigSource>> {
    let provider = BundleConfigSource::from_yaml_file(bundle_path)
        .with_context(|| format!("loading cfgsync provider from {}", bundle_path.display()))?;

    Ok(Arc::new(provider))
}

fn load_materializing_provider(bundle_path: &Path) -> anyhow::Result<Arc<dyn NodeConfigSource>> {
    let bundle = load_bundle_yaml(bundle_path)?;
    let catalog = build_node_catalog(bundle);
    let provider = RegistrationConfigProvider::new(catalog);

    Ok(Arc::new(provider))
}

fn load_bundle_yaml(bundle_path: &Path) -> anyhow::Result<NodeArtifactsBundle> {
    let raw = fs::read_to_string(bundle_path)
        .with_context(|| format!("reading cfgsync bundle from {}", bundle_path.display()))?;

    serde_yaml::from_str(&raw)
        .with_context(|| format!("parsing cfgsync bundle from {}", bundle_path.display()))
}

fn build_node_catalog(bundle: NodeArtifactsBundle) -> NodeArtifactsCatalog {
    let nodes = bundle
        .nodes
        .into_iter()
        .map(|node| NodeArtifacts {
            identifier: node.identifier,
            files: node.files,
        })
        .collect();

    NodeArtifactsCatalog::new(nodes)
}

fn resolve_bundle_path(config_path: &Path, bundle_path: &str) -> std::path::PathBuf {
    let path = Path::new(bundle_path);
    if path.is_absolute() {
        return path.to_path_buf();
    }

    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(path)
}

/// Loads runtime config and starts cfgsync HTTP server process.
pub async fn run_cfgsync_server(config_path: &Path) -> anyhow::Result<()> {
    let config = CfgsyncServerConfig::load_from_file(config_path)?;
    let bundle_path = resolve_bundle_path(config_path, &config.bundle_path);

    let state = build_server_state(&config, &bundle_path)?;
    run_cfgsync(config.port, state).await?;

    Ok(())
}

fn build_server_state(
    config: &CfgsyncServerConfig,
    bundle_path: &Path,
) -> anyhow::Result<CfgsyncServerState> {
    let repo = match config.serving_mode {
        CfgsyncServingMode::Bundle => load_bundle_provider(bundle_path)?,
        CfgsyncServingMode::Registration => load_materializing_provider(bundle_path)?,
    };

    Ok(CfgsyncServerState::new(repo))
}

#[doc(hidden)]
pub type CfgSyncServerConfig = CfgsyncServerConfig;

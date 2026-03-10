use std::{fs, path::Path, sync::Arc};

use anyhow::Context as _;
use cfgsync_adapter::{NodeArtifacts, NodeArtifactsCatalog, RegistrationConfigProvider};
use cfgsync_core::{CfgSyncBundle, CfgSyncState, ConfigProvider, FileConfigProvider, run_cfgsync};
use serde::Deserialize;

/// Runtime cfgsync server config loaded from YAML.
#[derive(Debug, Deserialize, Clone)]
pub struct CfgSyncServerConfig {
    pub port: u16,
    pub bundle_path: String,
    #[serde(default)]
    pub registration_flow: bool,
}

impl CfgSyncServerConfig {
    /// Loads cfgsync runtime server config from a YAML file.
    pub fn load_from_file(path: &Path) -> anyhow::Result<Self> {
        let config_content = fs::read_to_string(path)
            .with_context(|| format!("failed to read cfgsync config file {}", path.display()))?;

        serde_yaml::from_str(&config_content)
            .with_context(|| format!("failed to parse cfgsync config file {}", path.display()))
    }
}

fn load_bundle_provider(bundle_path: &Path) -> anyhow::Result<Arc<dyn ConfigProvider>> {
    let provider = FileConfigProvider::from_yaml_file(bundle_path)
        .with_context(|| format!("loading cfgsync provider from {}", bundle_path.display()))?;

    Ok(Arc::new(provider))
}

fn load_materializing_provider(bundle_path: &Path) -> anyhow::Result<Arc<dyn ConfigProvider>> {
    let bundle = load_bundle_yaml(bundle_path)?;
    let catalog = build_node_catalog(bundle);
    let provider = RegistrationConfigProvider::new(catalog);

    Ok(Arc::new(provider))
}

fn load_bundle_yaml(bundle_path: &Path) -> anyhow::Result<CfgSyncBundle> {
    let raw = fs::read_to_string(bundle_path)
        .with_context(|| format!("reading cfgsync bundle from {}", bundle_path.display()))?;

    serde_yaml::from_str(&raw)
        .with_context(|| format!("parsing cfgsync bundle from {}", bundle_path.display()))
}

fn build_node_catalog(bundle: CfgSyncBundle) -> NodeArtifactsCatalog {
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
    let config = CfgSyncServerConfig::load_from_file(config_path)?;
    let bundle_path = resolve_bundle_path(config_path, &config.bundle_path);

    let state = build_server_state(&config, &bundle_path)?;
    run_cfgsync(config.port, state).await?;

    Ok(())
}

fn build_server_state(
    config: &CfgSyncServerConfig,
    bundle_path: &Path,
) -> anyhow::Result<CfgSyncState> {
    let repo = if config.registration_flow {
        load_materializing_provider(bundle_path)?
    } else {
        load_bundle_provider(bundle_path)?
    };

    Ok(CfgSyncState::new(repo))
}

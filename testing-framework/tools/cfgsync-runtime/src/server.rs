use std::{collections::HashMap, fs, path::Path, sync::Arc};

use anyhow::Context as _;
use cfgsync_core::{CfgSyncFile, CfgSyncPayload, CfgSyncState, ConfigRepo, run_cfgsync};
use serde::Deserialize;

#[derive(Debug, Deserialize, Clone)]
pub struct CfgSyncServerConfig {
    pub port: u16,
    pub bundle_path: String,
}

impl CfgSyncServerConfig {
    pub fn load_from_file(path: &Path) -> anyhow::Result<Self> {
        let config_content = fs::read_to_string(path)
            .with_context(|| format!("failed to read cfgsync config file {}", path.display()))?;
        serde_yaml::from_str(&config_content)
            .with_context(|| format!("failed to parse cfgsync config file {}", path.display()))
    }
}

#[derive(Debug, Deserialize)]
struct CfgSyncBundle {
    nodes: Vec<CfgSyncBundleNode>,
}

#[derive(Debug, Deserialize)]
struct CfgSyncBundleNode {
    identifier: String,
    #[serde(default)]
    files: Vec<CfgSyncFile>,
    #[serde(default)]
    config_yaml: Option<String>,
}

fn load_bundle(bundle_path: &Path) -> anyhow::Result<Arc<ConfigRepo>> {
    let bundle = read_cfgsync_bundle(bundle_path)?;

    let configs = bundle
        .nodes
        .into_iter()
        .map(build_repo_entry)
        .collect::<HashMap<_, _>>();

    Ok(ConfigRepo::from_bundle(configs))
}

fn read_cfgsync_bundle(bundle_path: &Path) -> anyhow::Result<CfgSyncBundle> {
    let bundle_content = fs::read_to_string(bundle_path).with_context(|| {
        format!(
            "failed to read cfgsync bundle file {}",
            bundle_path.display()
        )
    })?;

    serde_yaml::from_str(&bundle_content)
        .with_context(|| format!("failed to parse cfgsync bundle {}", bundle_path.display()))
}

fn build_repo_entry(node: CfgSyncBundleNode) -> (String, CfgSyncPayload) {
    let files = if node.files.is_empty() {
        build_legacy_files(node.config_yaml)
    } else {
        node.files
    };

    (node.identifier, CfgSyncPayload::from_files(files))
}

fn build_legacy_files(config_yaml: Option<String>) -> Vec<CfgSyncFile> {
    config_yaml
        .map(|content| {
            vec![CfgSyncFile {
                path: "/config.yaml".to_owned(),
                content,
            }]
        })
        .unwrap_or_default()
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

pub async fn run_cfgsync_server(config_path: &Path) -> anyhow::Result<()> {
    let config = CfgSyncServerConfig::load_from_file(config_path)?;
    let bundle_path = resolve_bundle_path(config_path, &config.bundle_path);

    let repo = load_bundle(&bundle_path)?;
    let state = CfgSyncState::new(repo);
    run_cfgsync(config.port, state).await?;
    Ok(())
}

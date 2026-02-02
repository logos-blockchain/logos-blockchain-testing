use std::{fs::File, path::Path};

use anyhow::{Context as _, Result};
use lb_tracing_service::TracingSettings;
use serde::{Deserialize, Serialize};
use tracing::debug;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgSyncConfig {
    pub port: u16,
    #[serde(default)]
    pub n_hosts: usize,
    pub timeout: u64,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_path: Option<String>,
    #[serde(default)]
    pub tracing_settings: TracingSettings,
}

pub fn load_cfgsync_template(path: &Path) -> Result<CfgSyncConfig> {
    debug!(path = %path.display(), "loading cfgsync template");
    let file = File::open(path)
        .with_context(|| format!("opening cfgsync template at {}", path.display()))?;
    serde_yaml::from_reader(file).context("parsing cfgsync template")
}

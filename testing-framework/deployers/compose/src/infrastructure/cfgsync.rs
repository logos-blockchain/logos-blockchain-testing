use std::{path::Path, process::Command as StdCommand};

use nomos_tracing::metrics::otlp::OtlpMetricsConfig;
use nomos_tracing_service::MetricsLayer;
use reqwest::Url;
use testing_framework_core::{
    scenario::cfgsync::{apply_topology_overrides, load_cfgsync_template, write_cfgsync_template},
    topology::generation::GeneratedTopology,
};
use tracing::{debug, info, warn};

/// Handle that tracks a cfgsync server started for compose runs.
#[derive(Debug)]
pub enum CfgsyncServerHandle {
    Container { name: String, stopped: bool },
}

impl CfgsyncServerHandle {
    /// Stop the backing container if still running.
    pub fn shutdown(&mut self) {
        match self {
            Self::Container { name, stopped } if !*stopped => {
                info!(container = name, "stopping cfgsync container");
                remove_container(name);
                *stopped = true;
            }
            _ => {}
        }
    }
}

fn remove_container(name: &str) {
    match StdCommand::new("docker")
        .arg("rm")
        .arg("-f")
        .arg(name)
        .status()
    {
        Ok(status) if status.success() => {
            debug!(container = name, "removed cfgsync container");
        }
        Ok(status) => {
            warn!(container = name, status = ?status, "failed to remove cfgsync container");
        }
        Err(_) => {
            warn!(
                container = name,
                "failed to spawn docker rm for cfgsync container"
            );
        }
    }
}

impl Drop for CfgsyncServerHandle {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Updates the cfgsync template on disk with topology-driven overrides.
pub fn update_cfgsync_config(
    path: &Path,
    topology: &GeneratedTopology,
    port: u16,
    metrics_otlp_ingest_url: Option<&Url>,
) -> anyhow::Result<()> {
    debug!(
        path = %path.display(),
        port,
        nodes = topology.nodes().len(),
        "updating cfgsync template"
    );
    let mut cfg = load_cfgsync_template(path)?;
    cfg.port = port;
    apply_topology_overrides(&mut cfg, topology);
    if let Some(endpoint) = metrics_otlp_ingest_url.cloned() {
        cfg.tracing_settings.metrics = MetricsLayer::Otlp(OtlpMetricsConfig {
            endpoint,
            host_identifier: "node".into(),
        });
    }
    write_cfgsync_template(path, &cfg)?;
    Ok(())
}

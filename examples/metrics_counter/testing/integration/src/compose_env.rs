use std::{env, fs, path::Path};

use reqwest::Url;
use testing_framework_core::scenario::DynError;
use testing_framework_runner_compose::{
    BinaryConfigNodeSpec, ComposeBinaryApp, EnvEntry, NodeDescriptor,
};

use crate::MetricsCounterEnv;

const NODE_CONFIG_PATH: &str = "/etc/metrics-counter/config.yaml";
const PROM_CONFIG_MOUNT_PATH: &str = "/etc/prometheus/prometheus.yml:ro";
const PROMETHEUS_SERVICE_NAME: &str = "prometheus";
const PROMETHEUS_CONTAINER_PORT: u16 = 9090;
const DEFAULT_PROMETHEUS_QUERY_PORT: u16 = 19091;

impl ComposeBinaryApp for MetricsCounterEnv {
    fn compose_node_spec() -> BinaryConfigNodeSpec {
        BinaryConfigNodeSpec::conventional(
            "/usr/local/bin/metrics-counter-node",
            NODE_CONFIG_PATH,
            vec![8080, 8081],
        )
    }

    fn prepare_compose_workspace(
        path: &Path,
        topology: &<Self as testing_framework_core::scenario::Application>::Deployment,
        _metrics_otlp_ingest_url: Option<&Url>,
    ) -> Result<(), DynError> {
        if prometheus_query_port().is_none() {
            return Ok(());
        }

        let stack_dir = path
            .parent()
            .ok_or_else(|| anyhow::anyhow!("cfgsync path has no parent"))?;
        let configs_dir = stack_dir.join("configs");
        fs::create_dir_all(&configs_dir)?;
        fs::write(
            configs_dir.join("prometheus.yml"),
            render_prometheus_config(topology),
        )?;
        Ok(())
    }

    fn compose_extra_services(
        _topology: &<Self as testing_framework_core::scenario::Application>::Deployment,
    ) -> Result<Vec<NodeDescriptor>, DynError> {
        let Some(prom_port) = prometheus_query_port() else {
            return Ok(Vec::new());
        };

        Ok(vec![NodeDescriptor::new(
            PROMETHEUS_SERVICE_NAME,
            "prom/prometheus:v2.54.1",
            vec![
                "/bin/prometheus".to_owned(),
                "--config.file=/etc/prometheus/prometheus.yml".to_owned(),
                "--web.listen-address=0.0.0.0:9090".to_owned(),
            ],
            vec![prometheus_config_volume()],
            vec![],
            vec![format!("127.0.0.1:{prom_port}:{PROMETHEUS_CONTAINER_PORT}")],
            vec![PROMETHEUS_CONTAINER_PORT],
            vec![EnvEntry::new("TZ", "UTC")],
            None,
        )])
    }
}

fn prometheus_config_volume() -> String {
    format!("./stack/configs/prometheus.yml:{PROM_CONFIG_MOUNT_PATH}")
}

fn prometheus_query_port() -> Option<u16> {
    let raw = env::var("LOGOS_BLOCKCHAIN_METRICS_QUERY_URL")
        .unwrap_or_else(|_| format!("http://127.0.0.1:{DEFAULT_PROMETHEUS_QUERY_PORT}"));
    let parsed = Url::parse(raw.trim()).ok()?;
    parsed
        .port_or_known_default()
        .or(Some(DEFAULT_PROMETHEUS_QUERY_PORT))
}

fn render_prometheus_config(topology: &crate::MetricsCounterTopology) -> String {
    let targets = (0..topology.node_count)
        .map(|index| format!("\"node-{index}:8080\""))
        .collect::<Vec<_>>()
        .join(", ");

    format!(
        "global:\n  scrape_interval: 1s\nscrape_configs:\n  - job_name: metrics_counter\n    metrics_path: /metrics\n    static_configs:\n      - targets: [{targets}]\n"
    )
}

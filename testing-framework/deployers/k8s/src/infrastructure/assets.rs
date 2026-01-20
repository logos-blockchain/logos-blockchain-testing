use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result as AnyResult};
use nomos_tracing::metrics::otlp::OtlpMetricsConfig;
use nomos_tracing_service::MetricsLayer;
use reqwest::Url;
use serde::Serialize;
use tempfile::TempDir;
use testing_framework_config::constants::{DEFAULT_ASSETS_STACK_DIR, cfgsync_port};
pub use testing_framework_core::kzg::KzgMode;
use testing_framework_core::{
    kzg::KzgParamsSpec,
    scenario::cfgsync::{apply_topology_overrides, load_cfgsync_template, render_cfgsync_yaml},
    topology::generation::GeneratedTopology,
};
use testing_framework_env as tf_env;
use thiserror::Error;
use tracing::{debug, info};

/// Paths and image metadata required to deploy the Helm chart.
pub struct RunnerAssets {
    pub image: String,
    pub kzg_mode: KzgMode,
    pub kzg_path: Option<PathBuf>,
    pub chart_path: PathBuf,
    pub cfgsync_file: PathBuf,
    pub run_cfgsync_script: PathBuf,
    pub run_nomos_script: PathBuf,
    pub run_nomos_node_script: PathBuf,
    pub values_file: PathBuf,
    _tempdir: TempDir,
}

pub fn cfgsync_port_value() -> u16 {
    cfgsync_port()
}

#[derive(Debug, Error)]
/// Failures preparing Helm assets and rendered cfgsync configuration.
pub enum AssetsError {
    #[error("failed to locate workspace root: {source}")]
    WorkspaceRoot {
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to render cfgsync configuration: {source}")]
    Cfgsync {
        #[source]
        source: anyhow::Error,
    },
    #[error("missing required script at {path}")]
    MissingScript { path: PathBuf },
    #[error("missing KZG parameters at {path}; build them with `make kzgrs_test_params`")]
    MissingKzg { path: PathBuf },
    #[error("missing Helm chart at {path}; ensure the repository is up-to-date")]
    MissingChart { path: PathBuf },
    #[error("failed to create temporary directory for rendered assets: {source}")]
    TempDir {
        #[source]
        source: io::Error,
    },
    #[error("failed to write asset at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to render Helm values: {source}")]
    Values {
        #[source]
        source: serde_yaml::Error,
    },
}

/// Render cfgsync config, Helm values, and locate scripts/KZG assets for a
/// topology.
pub fn prepare_assets(
    topology: &GeneratedTopology,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<RunnerAssets, AssetsError> {
    info!(
        nodes = topology.nodes().len(),
        "preparing k8s runner assets"
    );

    let root = workspace_root().map_err(|source| AssetsError::WorkspaceRoot { source })?;
    let kzg_spec = KzgParamsSpec::for_k8s(&root);

    let tempdir = create_assets_tempdir()?;

    let cfgsync_file = render_and_write_cfgsync(
        &root,
        topology,
        &kzg_spec,
        metrics_otlp_ingest_url,
        &tempdir,
    )?;
    let scripts = validate_scripts(&root)?;
    let kzg_path = resolve_kzg_path(&root, &kzg_spec)?;
    let chart_path = helm_chart_path()?;
    let values_file = render_and_write_values(topology, &tempdir)?;
    let image = testnet_image();

    let kzg_display = kzg_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<in-image>".to_string());
    debug!(
        cfgsync = %cfgsync_file.display(),
        values = %values_file.display(),
        image,
        kzg_mode = ?kzg_spec.mode,
        kzg = %kzg_display,
        chart = %chart_path.display(),
        "k8s runner assets prepared"
    );

    Ok(RunnerAssets {
        image,
        kzg_mode: kzg_spec.mode,
        kzg_path,
        chart_path,
        cfgsync_file,
        run_nomos_script: scripts.run_shared,
        run_cfgsync_script: scripts.run_cfgsync,
        run_nomos_node_script: scripts.run_node,
        values_file,
        _tempdir: tempdir,
    })
}

fn create_assets_tempdir() -> Result<TempDir, AssetsError> {
    tempfile::Builder::new()
        .prefix("nomos-helm-")
        .tempdir()
        .map_err(|source| AssetsError::TempDir { source })
}

fn render_and_write_cfgsync(
    root: &Path,
    topology: &GeneratedTopology,
    kzg_spec: &KzgParamsSpec,
    metrics_otlp_ingest_url: Option<&Url>,
    tempdir: &TempDir,
) -> Result<PathBuf, AssetsError> {
    let cfgsync_yaml = render_cfgsync_config(root, topology, kzg_spec, metrics_otlp_ingest_url)?;
    write_temp_file(tempdir.path(), "cfgsync.yaml", cfgsync_yaml)
}

fn resolve_kzg_path(root: &Path, kzg_spec: &KzgParamsSpec) -> Result<Option<PathBuf>, AssetsError> {
    match kzg_spec.mode {
        KzgMode::HostPath => Ok(Some(validate_kzg_params(root, kzg_spec)?)),
        KzgMode::InImage => Ok(None),
    }
}

fn render_and_write_values(
    topology: &GeneratedTopology,
    tempdir: &TempDir,
) -> Result<PathBuf, AssetsError> {
    let values_yaml = render_values_yaml(topology)?;
    write_temp_file(tempdir.path(), "values.yaml", values_yaml)
}

fn testnet_image() -> String {
    tf_env::nomos_testnet_image()
        .unwrap_or_else(|| String::from("public.ecr.aws/r4s5t9y4/logos/logos-blockchain:test"))
}

const CFGSYNC_K8S_TIMEOUT_SECS: u64 = 300;

fn render_cfgsync_config(
    root: &Path,
    topology: &GeneratedTopology,
    kzg_spec: &KzgParamsSpec,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<String, AssetsError> {
    let cfgsync_template_path = stack_assets_root(root).join("cfgsync.yaml");
    debug!(path = %cfgsync_template_path.display(), "loading cfgsync template");

    let mut cfg = load_cfgsync_template(&cfgsync_template_path)
        .map_err(|source| AssetsError::Cfgsync { source })?;

    apply_topology_overrides(&mut cfg, topology, kzg_spec.mode == KzgMode::HostPath);
    cfg.global_params_path = kzg_spec.node_params_path.clone();

    if let Some(endpoint) = metrics_otlp_ingest_url.cloned() {
        cfg.tracing_settings.metrics = MetricsLayer::Otlp(OtlpMetricsConfig {
            endpoint,
            host_identifier: "node".into(),
        });
    }

    cfg.timeout = cfg.timeout.max(CFGSYNC_K8S_TIMEOUT_SECS);

    render_cfgsync_yaml(&cfg).map_err(|source| AssetsError::Cfgsync { source })
}

struct ScriptPaths {
    run_cfgsync: PathBuf,
    run_shared: PathBuf,
    run_node: PathBuf,
}

fn validate_scripts(root: &Path) -> Result<ScriptPaths, AssetsError> {
    let scripts_dir = stack_scripts_root(root);
    let run_cfgsync = scripts_dir.join("run_cfgsync.sh");
    let run_shared = scripts_dir.join("run_nomos.sh");
    let run_node = scripts_dir.join("run_nomos_node.sh");

    for path in [&run_cfgsync, &run_shared, &run_node] {
        if !path.exists() {
            return Err(AssetsError::MissingScript { path: path.clone() });
        }
    }

    debug!(
        run_cfgsync = %run_cfgsync.display(),
        run_shared = %run_shared.display(),
        run_node = %run_node.display(),
        "validated runner scripts exist"
    );

    Ok(ScriptPaths {
        run_cfgsync,
        run_shared,
        run_node,
    })
}

fn validate_kzg_params(root: &Path, spec: &KzgParamsSpec) -> Result<PathBuf, AssetsError> {
    let Some(path) = spec.host_params_dir.clone() else {
        return Err(AssetsError::MissingKzg {
            path: root.join(testing_framework_config::constants::DEFAULT_KZG_HOST_DIR),
        });
    };
    if path.exists() {
        Ok(path)
    } else {
        Err(AssetsError::MissingKzg { path })
    }
}

fn helm_chart_path() -> Result<PathBuf, AssetsError> {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("helm/nomos-runner");
    if path.exists() {
        Ok(path)
    } else {
        Err(AssetsError::MissingChart { path })
    }
}

fn render_values_yaml(topology: &GeneratedTopology) -> Result<String, AssetsError> {
    let values = build_values(topology);
    serde_yaml::to_string(&values).map_err(|source| AssetsError::Values { source })
}

fn write_temp_file(
    dir: &Path,
    name: &str,
    contents: impl AsRef<[u8]>,
) -> Result<PathBuf, AssetsError> {
    let path = dir.join(name);
    fs::write(&path, contents).map_err(|source| AssetsError::Io {
        path: path.clone(),
        source,
    })?;
    Ok(path)
}

/// Locate the workspace root, honoring `CARGO_WORKSPACE_DIR` overrides.
pub fn workspace_root() -> AnyResult<PathBuf> {
    if let Ok(var) = env::var("CARGO_WORKSPACE_DIR") {
        return Ok(PathBuf::from(var));
    }
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .context("resolving workspace root from manifest dir")
}

fn stack_assets_root(root: &Path) -> PathBuf {
    let new_layout = root.join(DEFAULT_ASSETS_STACK_DIR);
    if new_layout.exists() {
        new_layout
    } else {
        root.join("testnet")
    }
}

fn stack_scripts_root(root: &Path) -> PathBuf {
    let new_layout = root.join(DEFAULT_ASSETS_STACK_DIR).join("scripts");
    if new_layout.exists() {
        new_layout
    } else {
        root.join("testnet/scripts")
    }
}

#[derive(Serialize)]
struct HelmValues {
    #[serde(rename = "imagePullPolicy")]
    image_pull_policy: String,
    cfgsync: CfgsyncValues,
    nodes: NodeGroup,
}

#[derive(Serialize)]
struct CfgsyncValues {
    port: u16,
}

#[derive(Serialize)]
struct NodeGroup {
    count: usize,
    nodes: Vec<NodeValues>,
}

#[derive(Serialize)]
struct NodeValues {
    #[serde(rename = "apiPort")]
    api_port: u16,
    #[serde(rename = "testingHttpPort")]
    testing_http_port: u16,
    env: BTreeMap<String, String>,
}

fn build_values(topology: &GeneratedTopology) -> HelmValues {
    let cfgsync = CfgsyncValues {
        port: cfgsync_port(),
    };
    let pol_mode = pol_proof_mode();
    let image_pull_policy =
        tf_env::nomos_testnet_image_pull_policy().unwrap_or_else(|| "IfNotPresent".into());
    debug!(pol_mode, "rendering Helm values for k8s stack");
    let nodes = build_node_group(topology.nodes(), &pol_mode);

    HelmValues {
        image_pull_policy,
        cfgsync,
        nodes,
    }
}

fn build_node_group(
    nodes: &[testing_framework_core::topology::generation::GeneratedNodeConfig],
    pol_mode: &str,
) -> NodeGroup {
    let node_values = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| build_node_values(index, node, pol_mode))
        .collect();

    NodeGroup {
        count: nodes.len(),
        nodes: node_values,
    }
}

fn build_node_values(
    index: usize,
    node: &testing_framework_core::topology::generation::GeneratedNodeConfig,
    pol_mode: &str,
) -> NodeValues {
    let mut env = BTreeMap::new();
    env.insert("POL_PROOF_DEV_MODE".into(), pol_mode.to_string());
    env.insert("CFG_NETWORK_PORT".into(), node.network_port().to_string());
    env.insert("CFG_DA_PORT".into(), node.da_port.to_string());
    env.insert("CFG_BLEND_PORT".into(), node.blend_port.to_string());
    env.insert(
        "CFG_API_PORT".into(),
        node.general.api_config.address.port().to_string(),
    );
    env.insert(
        "CFG_TESTING_HTTP_PORT".into(),
        node.general
            .api_config
            .testing_http_address
            .port()
            .to_string(),
    );
    env.insert("CFG_HOST_IDENTIFIER".into(), format!("node-{index}"));

    NodeValues {
        api_port: node.general.api_config.address.port(),
        testing_http_port: node.general.api_config.testing_http_address.port(),
        env,
    }
}

fn pol_proof_mode() -> String {
    tf_env::pol_proof_dev_mode().unwrap_or_else(|| "true".to_string())
}

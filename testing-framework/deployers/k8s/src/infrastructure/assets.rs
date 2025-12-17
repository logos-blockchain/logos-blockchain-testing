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
use testing_framework_core::{
    constants::cfgsync_port,
    scenario::cfgsync::{apply_topology_overrides, load_cfgsync_template, render_cfgsync_yaml},
    topology::generation::GeneratedTopology,
};
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
    pub run_nomos_executor_script: PathBuf,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KzgMode {
    HostPath,
    InImage,
}

fn kzg_mode() -> KzgMode {
    match env::var("NOMOS_KZG_MODE").ok().as_deref() {
        Some("inImage") => KzgMode::InImage,
        _ => KzgMode::HostPath,
    }
}

/// Render cfgsync config, Helm values, and locate scripts/KZG assets for a
/// topology.
pub fn prepare_assets(
    topology: &GeneratedTopology,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<RunnerAssets, AssetsError> {
    info!(
        validators = topology.validators().len(),
        executors = topology.executors().len(),
        "preparing k8s runner assets"
    );

    let root = workspace_root().map_err(|source| AssetsError::WorkspaceRoot { source })?;
    let kzg_mode = kzg_mode();
    let cfgsync_yaml = render_cfgsync_config(&root, topology, kzg_mode, metrics_otlp_ingest_url)?;

    let tempdir = tempfile::Builder::new()
        .prefix("nomos-helm-")
        .tempdir()
        .map_err(|source| AssetsError::TempDir { source })?;

    let cfgsync_file = write_temp_file(tempdir.path(), "cfgsync.yaml", cfgsync_yaml)?;
    let scripts = validate_scripts(&root)?;
    let kzg_path = match kzg_mode {
        KzgMode::HostPath => Some(validate_kzg_params(&root)?),
        KzgMode::InImage => None,
    };
    let chart_path = helm_chart_path()?;
    let values_yaml = render_values_yaml(topology)?;
    let values_file = write_temp_file(tempdir.path(), "values.yaml", values_yaml)?;
    let image = env::var("NOMOS_TESTNET_IMAGE")
        .unwrap_or_else(|_| String::from("public.ecr.aws/r4s5t9y4/logos/logos-blockchain:test"));

    let kzg_display = kzg_path
        .as_ref()
        .map(|path| path.display().to_string())
        .unwrap_or_else(|| "<in-image>".to_string());
    debug!(
        cfgsync = %cfgsync_file.display(),
        values = %values_file.display(),
        image,
        kzg_mode = ?kzg_mode,
        kzg = %kzg_display,
        chart = %chart_path.display(),
        "k8s runner assets prepared"
    );

    Ok(RunnerAssets {
        image,
        kzg_mode,
        kzg_path,
        chart_path,
        cfgsync_file,
        run_nomos_script: scripts.run_shared,
        run_cfgsync_script: scripts.run_cfgsync,
        run_nomos_node_script: scripts.run_node,
        run_nomos_executor_script: scripts.run_executor,
        values_file,
        _tempdir: tempdir,
    })
}

const CFGSYNC_K8S_TIMEOUT_SECS: u64 = 300;
const DEFAULT_IN_IMAGE_KZG_PARAMS_PATH: &str = "/opt/nomos/kzg-params/kzgrs_test_params";

fn render_cfgsync_config(
    root: &Path,
    topology: &GeneratedTopology,
    kzg_mode: KzgMode,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<String, AssetsError> {
    let cfgsync_template_path = stack_assets_root(root).join("cfgsync.yaml");
    debug!(path = %cfgsync_template_path.display(), "loading cfgsync template");

    let mut cfg = load_cfgsync_template(&cfgsync_template_path)
        .map_err(|source| AssetsError::Cfgsync { source })?;

    apply_topology_overrides(&mut cfg, topology, kzg_mode == KzgMode::HostPath);

    if kzg_mode == KzgMode::InImage {
        cfg.global_params_path = env::var("NOMOS_KZGRS_PARAMS_PATH")
            .ok()
            .unwrap_or_else(|| DEFAULT_IN_IMAGE_KZG_PARAMS_PATH.to_string());
    }

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
    run_executor: PathBuf,
}

fn validate_scripts(root: &Path) -> Result<ScriptPaths, AssetsError> {
    let scripts_dir = stack_scripts_root(root);
    let run_cfgsync = scripts_dir.join("run_cfgsync.sh");
    let run_shared = scripts_dir.join("run_nomos.sh");
    let run_node = scripts_dir.join("run_nomos_node.sh");
    let run_executor = scripts_dir.join("run_nomos_executor.sh");

    for path in [&run_cfgsync, &run_shared, &run_node, &run_executor] {
        if !path.exists() {
            return Err(AssetsError::MissingScript { path: path.clone() });
        }
    }

    debug!(
        run_cfgsync = %run_cfgsync.display(),
        run_shared = %run_shared.display(),
        run_node = %run_node.display(),
        run_executor = %run_executor.display(),
        "validated runner scripts exist"
    );

    Ok(ScriptPaths {
        run_cfgsync,
        run_shared,
        run_node,
        run_executor,
    })
}

fn validate_kzg_params(root: &Path) -> Result<PathBuf, AssetsError> {
    let rel = env::var("NOMOS_KZG_DIR_REL")
        .ok()
        .unwrap_or_else(|| testing_framework_core::constants::DEFAULT_KZG_HOST_DIR.to_string());
    let path = root.join(rel);
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
    let new_layout = root.join("testing-framework/assets/stack");
    if new_layout.exists() {
        new_layout
    } else {
        root.join("testnet")
    }
}

fn stack_scripts_root(root: &Path) -> PathBuf {
    let new_layout = root.join("testing-framework/assets/stack/scripts");
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
    validators: NodeGroup,
    executors: NodeGroup,
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
        env::var("NOMOS_TESTNET_IMAGE_PULL_POLICY").unwrap_or_else(|_| "IfNotPresent".into());
    debug!(pol_mode, "rendering Helm values for k8s stack");
    let validators = topology
        .validators()
        .iter()
        .enumerate()
        .map(|(index, validator)| {
            let mut env = BTreeMap::new();
            env.insert("POL_PROOF_DEV_MODE".into(), pol_mode.clone());
            env.insert(
                "CFG_NETWORK_PORT".into(),
                validator.network_port().to_string(),
            );
            env.insert("CFG_DA_PORT".into(), validator.da_port.to_string());
            env.insert("CFG_BLEND_PORT".into(), validator.blend_port.to_string());
            env.insert(
                "CFG_API_PORT".into(),
                validator.general.api_config.address.port().to_string(),
            );
            env.insert(
                "CFG_TESTING_HTTP_PORT".into(),
                validator
                    .general
                    .api_config
                    .testing_http_address
                    .port()
                    .to_string(),
            );
            env.insert("CFG_HOST_KIND".into(), "validator".into());
            env.insert("CFG_HOST_IDENTIFIER".into(), format!("validator-{index}"));

            NodeValues {
                api_port: validator.general.api_config.address.port(),
                testing_http_port: validator.general.api_config.testing_http_address.port(),
                env,
            }
        })
        .collect();

    let executors = topology
        .executors()
        .iter()
        .enumerate()
        .map(|(index, executor)| {
            let mut env = BTreeMap::new();
            env.insert("POL_PROOF_DEV_MODE".into(), pol_mode.clone());
            env.insert(
                "CFG_NETWORK_PORT".into(),
                executor.network_port().to_string(),
            );
            env.insert("CFG_DA_PORT".into(), executor.da_port.to_string());
            env.insert("CFG_BLEND_PORT".into(), executor.blend_port.to_string());
            env.insert(
                "CFG_API_PORT".into(),
                executor.general.api_config.address.port().to_string(),
            );
            env.insert(
                "CFG_TESTING_HTTP_PORT".into(),
                executor
                    .general
                    .api_config
                    .testing_http_address
                    .port()
                    .to_string(),
            );
            env.insert("CFG_HOST_KIND".into(), "executor".into());
            env.insert("CFG_HOST_IDENTIFIER".into(), format!("executor-{index}"));

            NodeValues {
                api_port: executor.general.api_config.address.port(),
                testing_http_port: executor.general.api_config.testing_http_address.port(),
                env,
            }
        })
        .collect();

    HelmValues {
        image_pull_policy,
        cfgsync,
        validators: NodeGroup {
            count: topology.validators().len(),
            nodes: validators,
        },
        executors: NodeGroup {
            count: topology.executors().len(),
            nodes: executors,
        },
    }
}

fn pol_proof_mode() -> String {
    env::var("POL_PROOF_DEV_MODE").unwrap_or_else(|_| "true".to_string())
}

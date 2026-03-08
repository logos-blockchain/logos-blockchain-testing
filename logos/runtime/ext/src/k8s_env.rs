use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
    process::Output,
};

use anyhow::{Result as AnyhowResult, anyhow};
use async_trait::async_trait;
use kube::Client;
use lb_framework::{
    NodeHttpClient,
    internal::{DeploymentPlan, NodePlan},
};
use lb_http_api_common::paths;
use reqwest::Url;
use serde::Serialize;
use tempfile::TempDir;
use testing_framework_core::scenario::DynError;
use testing_framework_env as tf_env;
use testing_framework_runner_k8s::{K8sDeployEnv, PortSpecs, RunnerCleanup, wait::NodeConfigPorts};
use thiserror::Error;
use tokio::process::Command;
use tracing::{debug, info};

use crate::{
    LbcExtEnv,
    cfgsync::{CfgsyncOutputPaths, CfgsyncRenderOptions, render_and_write_cfgsync_from_template},
    constants::{DEFAULT_ASSETS_STACK_DIR, cfgsync_port},
};

const CFGSYNC_K8S_TIMEOUT_SECS: u64 = 300;
const K8S_FULLNAME_OVERRIDE: &str = "logos-runner";
const DEFAULT_K8S_TESTNET_IMAGE: &str = "logos-blockchain-testing:local";

/// Paths and image metadata required to deploy the Helm chart.
pub struct K8sAssets {
    pub image: String,
    pub chart_path: PathBuf,
    pub cfgsync_file: PathBuf,
    pub run_cfgsync_script: PathBuf,
    pub run_logos_script: PathBuf,
    pub run_logos_node_script: PathBuf,
    pub values_file: PathBuf,
    _tempdir: TempDir,
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

#[derive(Debug, Error)]
/// Errors returned from Helm invocations.
pub enum HelmError {
    #[error("failed to spawn {command}: {source}")]
    Spawn {
        command: String,
        #[source]
        source: io::Error,
    },
    #[error("{command} exited with status {status:?}\nstderr:\n{stderr}\nstdout:\n{stdout}")]
    Failed {
        command: String,
        status: Option<i32>,
        stdout: String,
        stderr: String,
    },
}

#[async_trait]
impl K8sDeployEnv for LbcExtEnv {
    type Assets = K8sAssets;

    fn collect_port_specs(topology: &Self::Deployment) -> PortSpecs {
        let nodes = topology
            .nodes()
            .iter()
            .map(|node| NodeConfigPorts {
                api: node.general.api_config.address.port(),
                testing: node.general.api_config.testing_http_address.port(),
            })
            .collect();
        PortSpecs { nodes }
    }

    fn prepare_assets(
        topology: &Self::Deployment,
        metrics_otlp_ingest_url: Option<&Url>,
    ) -> Result<Self::Assets, DynError> {
        prepare_assets(topology, metrics_otlp_ingest_url).map_err(|err| err.into())
    }

    async fn install_stack(
        client: &Client,
        assets: &Self::Assets,
        namespace: &str,
        release: &str,
        nodes: usize,
    ) -> Result<RunnerCleanup, DynError> {
        install_release(assets, release, namespace, nodes)
            .await
            .map_err(|err| -> DynError { Box::new(err) })?;

        let preserve = env::var("K8S_RUNNER_PRESERVE").is_ok();
        Ok(RunnerCleanup::new(
            client.clone(),
            namespace.to_owned(),
            release.to_owned(),
            preserve,
        ))
    }

    fn node_client_from_ports(
        host: &str,
        api_port: u16,
        testing_port: u16,
    ) -> Result<Self::NodeClient, DynError> {
        let base_url = node_url(host, api_port)?;
        let testing_url = Url::parse(&format!("http://{host}:{testing_port}")).ok();
        Ok(NodeHttpClient::from_urls(base_url, testing_url))
    }

    fn readiness_path() -> &'static str {
        paths::CRYPTARCHIA_INFO
    }

    fn node_base_url(client: &Self::NodeClient) -> Option<String> {
        Some(client.base_url().to_string())
    }

    fn node_deployment_name(_release: &str, index: usize) -> String {
        format!("{K8S_FULLNAME_OVERRIDE}-node-{index}")
    }

    fn node_service_name(_release: &str, index: usize) -> String {
        format!("{K8S_FULLNAME_OVERRIDE}-node-{index}")
    }
}

fn node_url(host: &str, port: u16) -> Result<Url, DynError> {
    let url = Url::parse(&format!("http://{host}:{port}"))?;
    Ok(url)
}

/// Render cfgsync config, Helm values, and locate scripts for a topology.
pub fn prepare_assets(
    topology: &DeploymentPlan,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<K8sAssets, AssetsError> {
    log_assets_prepare_start(topology);
    let root = workspace_root().map_err(|source| AssetsError::WorkspaceRoot { source })?;
    let tempdir = create_assets_tempdir()?;

    let (cfgsync_file, cfgsync_yaml, bundle_yaml) =
        render_and_write_cfgsync(topology, metrics_otlp_ingest_url, &tempdir)?;
    let scripts = validate_scripts(&root)?;
    let chart_path = helm_chart_path()?;
    let values_file = render_and_write_values(topology, &tempdir, &cfgsync_yaml, &bundle_yaml)?;
    let image = testnet_image();

    log_assets_prepare_done(&cfgsync_file, &values_file, &chart_path, &image);

    Ok(K8sAssets {
        image,
        chart_path,
        cfgsync_file,
        run_logos_script: scripts.run_shared,
        run_cfgsync_script: scripts.run_cfgsync,
        run_logos_node_script: scripts.run_node,
        values_file,
        _tempdir: tempdir,
    })
}

fn log_assets_prepare_start(topology: &DeploymentPlan) {
    info!(
        nodes = topology.nodes().len(),
        "preparing k8s runner assets"
    );
}

fn log_assets_prepare_done(
    cfgsync_file: &Path,
    values_file: &Path,
    chart_path: &Path,
    image: &str,
) {
    debug!(
        cfgsync = %cfgsync_file.display(),
        values = %values_file.display(),
        image,
        chart = %chart_path.display(),
        "k8s runner assets prepared"
    );
}

async fn install_release(
    assets: &K8sAssets,
    release: &str,
    namespace: &str,
    nodes: usize,
) -> Result<(), HelmError> {
    info!(
        release,
        namespace,
        nodes,
        image = %assets.image,
        cfgsync_port = cfgsync_port(),
        values = %assets.values_file.display(),
        "installing helm release"
    );

    let command = format!("helm install {release}");
    let cmd = build_install_command(assets, release, namespace, nodes);
    let output = run_helm_command(cmd, &command).await?;

    maybe_log_install_output(&command, &output);

    info!(release, namespace, "helm install completed");
    Ok(())
}

fn build_install_command(
    assets: &K8sAssets,
    release: &str,
    namespace: &str,
    nodes: usize,
) -> Command {
    let mut cmd = Command::new("helm");
    cmd.arg("install").arg(release).arg(&assets.chart_path);
    add_install_scoping_args(&mut cmd, namespace);
    add_install_settings(&mut cmd, assets, nodes);
    add_script_file_settings(&mut cmd, assets);

    if let Ok(root) = workspace_root() {
        cmd.current_dir(root);
    }

    cmd
}

fn add_install_scoping_args(cmd: &mut Command, namespace: &str) {
    cmd.arg("--namespace")
        .arg(namespace)
        .arg("--create-namespace")
        .arg("--wait")
        .arg("--timeout")
        .arg("5m");
}

fn add_install_settings(cmd: &mut Command, assets: &K8sAssets, nodes: usize) {
    cmd.arg("--set")
        .arg(format!("image={}", assets.image))
        .arg("--set")
        .arg(format!("nodes.count={nodes}"))
        .arg("--set")
        .arg(format!("cfgsync.port={}", cfgsync_port()))
        .arg("-f")
        .arg(&assets.values_file)
        .arg("--set-file")
        .arg(format!("cfgsync.config={}", assets.cfgsync_file.display()));
}

fn add_script_file_settings(cmd: &mut Command, assets: &K8sAssets) {
    add_set_file_arg(cmd, "scripts.runCfgsyncSh", &assets.run_cfgsync_script);
    add_set_file_arg(cmd, "scripts.runLogosNodeSh", &assets.run_logos_node_script);
    add_set_file_arg(cmd, "scripts.runLogosSh", &assets.run_logos_script);
}

fn add_set_file_arg(cmd: &mut Command, key: &str, value: &Path) {
    cmd.arg("--set-file")
        .arg(format!("{key}={}", value.display()));
}

fn maybe_log_install_output(command: &str, output: &Output) {
    if env::var("K8S_RUNNER_DEBUG").is_err() {
        return;
    }

    debug!(
        command,
        stdout = %String::from_utf8_lossy(&output.stdout),
        "helm install stdout"
    );
    debug!(
        command,
        stderr = %String::from_utf8_lossy(&output.stderr),
        "helm install stderr"
    );
}

async fn run_helm_command(mut cmd: Command, command: &str) -> Result<Output, HelmError> {
    let output = cmd.output().await.map_err(|source| HelmError::Spawn {
        command: command.to_owned(),
        source,
    })?;

    if output.status.success() {
        Ok(output)
    } else {
        Err(HelmError::Failed {
            command: command.to_owned(),
            status: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    }
}

fn create_assets_tempdir() -> Result<TempDir, AssetsError> {
    tempfile::Builder::new()
        .prefix("nomos-helm-")
        .tempdir()
        .map_err(|source| AssetsError::TempDir { source })
}

fn render_and_write_cfgsync(
    topology: &DeploymentPlan,
    metrics_otlp_ingest_url: Option<&Url>,
    tempdir: &TempDir,
) -> Result<(PathBuf, String, String), AssetsError> {
    let cfgsync_file = tempdir.path().join("cfgsync.yaml");
    let bundle_file = tempdir.path().join("cfgsync.bundle.yaml");
    let (cfgsync_yaml, bundle_yaml) = render_cfgsync_config(
        topology,
        metrics_otlp_ingest_url,
        &cfgsync_file,
        &bundle_file,
    )?;

    Ok((cfgsync_file, cfgsync_yaml, bundle_yaml))
}

fn render_and_write_values(
    topology: &DeploymentPlan,
    tempdir: &TempDir,
    cfgsync_yaml: &str,
    bundle_yaml: &str,
) -> Result<PathBuf, AssetsError> {
    let values_yaml = render_values_yaml(topology, cfgsync_yaml, bundle_yaml)?;
    write_temp_file(tempdir.path(), "values.yaml", values_yaml)
}

fn testnet_image() -> String {
    tf_env::nomos_testnet_image().unwrap_or_else(|| String::from(DEFAULT_K8S_TESTNET_IMAGE))
}

fn render_cfgsync_config(
    topology: &DeploymentPlan,
    metrics_otlp_ingest_url: Option<&Url>,
    cfgsync_file: &Path,
    bundle_file: &Path,
) -> Result<(String, String), AssetsError> {
    let hostnames = k8s_node_hostnames(topology);
    let rendered = render_and_write_cfgsync_from_template::<lb_framework::LbcEnv>(
        topology,
        &hostnames,
        CfgsyncRenderOptions {
            port: Some(cfgsync_port()),
            bundle_path: Some("cfgsync.bundle.yaml".to_string()),
            min_timeout_secs: Some(CFGSYNC_K8S_TIMEOUT_SECS),
            metrics_otlp_ingest_url: metrics_otlp_ingest_url.cloned(),
        },
        CfgsyncOutputPaths {
            config_path: cfgsync_file,
            bundle_path: bundle_file,
        },
    )
    .map_err(|source| AssetsError::Cfgsync { source })?;

    Ok((rendered.config_yaml, rendered.bundle_yaml))
}

fn k8s_node_hostnames(topology: &DeploymentPlan) -> Vec<String> {
    topology
        .nodes()
        .iter()
        .map(|node| format!("{K8S_FULLNAME_OVERRIDE}-node-{}", node.index()))
        .collect()
}

struct ScriptPaths {
    run_cfgsync: PathBuf,
    run_shared: PathBuf,
    run_node: PathBuf,
}

fn validate_scripts(root: &Path) -> Result<ScriptPaths, AssetsError> {
    let scripts_dir = stack_scripts_root(root);
    let run_cfgsync = scripts_dir.join("run_cfgsync.sh");
    let run_shared = scripts_dir.join("run_logos.sh");
    let run_node = scripts_dir.join("run_logos_node.sh");

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

fn helm_chart_path() -> Result<PathBuf, AssetsError> {
    let root = workspace_root().map_err(|source| AssetsError::WorkspaceRoot { source })?;
    let path = if let Some(override_dir) = helm_override_dir(&root) {
        override_dir
    } else {
        root.join("logos/infra/helm/logos-runner")
    };
    if path.exists() {
        Ok(path)
    } else {
        Err(AssetsError::MissingChart { path })
    }
}

fn render_values_yaml(
    topology: &DeploymentPlan,
    cfgsync_yaml: &str,
    bundle_yaml: &str,
) -> Result<String, AssetsError> {
    let values = build_values(topology, cfgsync_yaml, bundle_yaml);
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
pub fn workspace_root() -> AnyhowResult<PathBuf> {
    if let Ok(var) = env::var("CARGO_WORKSPACE_DIR") {
        return Ok(PathBuf::from(var));
    }
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let candidate_roots = [
        manifest_dir
            .parent()
            .and_then(Path::parent)
            .and_then(Path::parent),
        manifest_dir.parent().and_then(Path::parent),
    ];

    for candidate in candidate_roots.iter().flatten() {
        let stack_root = if let Some(override_dir) = assets_override_dir(candidate) {
            override_dir
        } else {
            candidate.join(DEFAULT_ASSETS_STACK_DIR)
        };
        if stack_root.exists() {
            return Ok(candidate.to_path_buf());
        }
    }

    Err(anyhow!(
        "resolving workspace root from manifest dir: {manifest_dir:?}"
    ))
}

fn stack_scripts_root(root: &Path) -> PathBuf {
    if let Some(scripts) = override_scripts_dir(root)
        && scripts.exists()
    {
        return scripts;
    }
    root.join(DEFAULT_ASSETS_STACK_DIR).join("scripts")
}

fn assets_override_dir(root: &Path) -> Option<PathBuf> {
    env::var("REL_ASSETS_STACK_DIR").ok().map(|value| {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    })
}

fn override_scripts_dir(root: &Path) -> Option<PathBuf> {
    assets_override_dir(root).map(|dir| dir.join("scripts"))
}

fn helm_override_dir(root: &Path) -> Option<PathBuf> {
    env::var("REL_HELM_CHART_DIR").ok().map(|value| {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    })
}

#[derive(Serialize)]
struct HelmValues {
    #[serde(rename = "imagePullPolicy")]
    image_pull_policy: String,
    #[serde(rename = "fullnameOverride")]
    fullname_override: String,
    kzg: KzgValues,
    cfgsync: CfgsyncValues,
    nodes: NodeGroup,
}

#[derive(Serialize)]
struct KzgValues {
    mode: String,
    #[serde(rename = "storageSize")]
    storage_size: String,
    #[serde(rename = "hostPath")]
    host_path: String,
    #[serde(rename = "hostPathType")]
    host_path_type: String,
}

#[derive(Serialize)]
struct CfgsyncValues {
    port: u16,
    config: String,
    bundle: String,
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
    #[serde(rename = "networkPort")]
    network_port: u16,
    env: BTreeMap<String, String>,
}

fn build_values(topology: &DeploymentPlan, cfgsync_yaml: &str, bundle_yaml: &str) -> HelmValues {
    let cfgsync = CfgsyncValues {
        port: cfgsync_port(),
        config: cfgsync_yaml.to_string(),
        bundle: bundle_yaml.to_string(),
    };
    let kzg = KzgValues::disabled();
    let image_pull_policy =
        tf_env::nomos_testnet_image_pull_policy().unwrap_or_else(|| "IfNotPresent".into());
    debug!("rendering Helm values for k8s stack");
    let nodes = build_node_group("node", topology.nodes());

    HelmValues {
        image_pull_policy,
        fullname_override: K8S_FULLNAME_OVERRIDE.to_string(),
        kzg,
        cfgsync,
        nodes,
    }
}

impl KzgValues {
    fn disabled() -> Self {
        Self {
            mode: "disabled".to_string(),
            storage_size: "1Gi".to_string(),
            host_path: "/tmp/nomos-kzg".to_string(),
            host_path_type: "DirectoryOrCreate".to_string(),
        }
    }
}

fn build_node_group(kind: &'static str, nodes: &[NodePlan]) -> NodeGroup {
    let node_values = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| build_node_values(kind, index, node))
        .collect();

    NodeGroup {
        count: nodes.len(),
        nodes: node_values,
    }
}

fn build_node_values(kind: &'static str, index: usize, node: &NodePlan) -> NodeValues {
    let mut env = BTreeMap::new();
    env.insert("CFG_HOST_KIND".into(), kind.to_string());
    env.insert("CFG_HOST_IDENTIFIER".into(), format!("{kind}-{index}"));

    NodeValues {
        api_port: node.general.api_config.address.port(),
        testing_http_port: node.general.api_config.testing_http_address.port(),
        network_port: node.general.network_config.backend.swarm.port,
        env,
    }
}

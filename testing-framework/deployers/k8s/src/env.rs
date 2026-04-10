use std::{
    env, fs,
    path::PathBuf,
    process,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use cfgsync_artifacts::ArtifactSet;
use kube::Client;
use reqwest::Url;
use serde::Serialize;
use tempfile::TempDir;
use testing_framework_core::{
    cfgsync::StaticNodeConfigProvider,
    scenario::{
        Application, DynError, HttpReadinessRequirement, NodeAccess,
        wait_for_http_ports_with_host_and_requirement, wait_http_readiness,
    },
    topology::DeploymentDescriptor,
};

use crate::{
    HelmReleaseBundle,
    infrastructure::{cluster::PortSpecs, helm::install_release},
    lifecycle::cleanup::RunnerCleanup,
};

pub trait HelmReleaseAssets {
    fn release_bundle(&self) -> HelmReleaseBundle;
}

#[derive(Debug)]
pub struct RenderedHelmChartAssets {
    chart_path: PathBuf,
    _tempdir: TempDir,
}

#[derive(Clone, Debug, Default)]
pub struct HelmManifest {
    documents: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct BinaryConfigK8sSpec {
    pub chart_name: String,
    pub node_name_prefix: String,
    pub binary_path: String,
    pub config_container_path: String,
    pub container_http_port: u16,
    pub service_testing_port: u16,
    pub image_env_var: String,
    pub fallback_image_env_var: String,
    pub default_image: String,
    pub image_pull_policy: String,
}

impl HelmReleaseAssets for RenderedHelmChartAssets {
    fn release_bundle(&self) -> HelmReleaseBundle {
        HelmReleaseBundle::new(self.chart_path.clone())
    }
}

impl HelmManifest {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_yaml<T>(&mut self, value: &T) -> Result<(), DynError>
    where
        T: Serialize,
    {
        self.documents
            .push(normalize_yaml_document(&serde_yaml::to_string(value)?));
        Ok(())
    }

    pub fn push_raw_yaml(&mut self, yaml: &str) {
        let yaml = yaml.trim();
        if !yaml.is_empty() {
            self.documents.push(yaml.to_owned());
        }
    }

    pub fn extend(&mut self, other: Self) {
        self.documents.extend(other.documents);
    }

    #[must_use]
    pub fn render(&self) -> String {
        self.documents.join("\n---\n")
    }
}

pub fn standard_port_specs(node_count: usize, api: u16, auxiliary: u16) -> PortSpecs {
    PortSpecs {
        nodes: (0..node_count)
            .map(|_| crate::wait::NodeConfigPorts { api, auxiliary })
            .collect(),
    }
}

impl BinaryConfigK8sSpec {
    #[must_use]
    pub fn conventional(
        chart_name: &str,
        node_name_prefix: &str,
        binary_path: &str,
        config_container_path: &str,
        container_http_port: u16,
        service_testing_port: u16,
    ) -> Self {
        let binary_name = binary_path
            .rsplit('/')
            .next()
            .unwrap_or(binary_path)
            .to_owned();
        let env_prefix = binary_name
            .strip_suffix("-node")
            .unwrap_or(&binary_name)
            .replace('-', "_")
            .to_ascii_uppercase();

        Self {
            chart_name: chart_name.to_owned(),
            node_name_prefix: node_name_prefix.to_owned(),
            binary_path: binary_path.to_owned(),
            config_container_path: config_container_path.to_owned(),
            container_http_port,
            service_testing_port,
            image_env_var: format!("{env_prefix}_K8S_IMAGE"),
            fallback_image_env_var: format!("{env_prefix}_IMAGE"),
            default_image: format!("{binary_name}:local"),
            image_pull_policy: "IfNotPresent".to_owned(),
        }
    }
}

pub fn render_binary_config_node_chart_assets<E>(
    deployment: &E::Deployment,
    spec: &BinaryConfigK8sSpec,
) -> Result<RenderedHelmChartAssets, DynError>
where
    E: StaticNodeConfigProvider,
    E::Deployment: DeploymentDescriptor,
{
    let manifest = render_binary_config_node_manifest::<E>(deployment, spec)?;
    render_single_template_chart_assets(
        &spec.chart_name,
        &format!("{}.yaml", spec.chart_name),
        &manifest,
    )
}

pub fn render_binary_config_node_manifest<E>(
    deployment: &E::Deployment,
    spec: &BinaryConfigK8sSpec,
) -> Result<String, DynError>
where
    E: StaticNodeConfigProvider,
    E::Deployment: DeploymentDescriptor,
{
    let node_count = deployment.node_count();
    let mut docs = Vec::with_capacity(node_count * 3);
    let hostnames = (0..node_count)
        .map(|index| format!("{}-{index}", spec.node_name_prefix))
        .collect::<Vec<_>>();

    for index in 0..node_count {
        let name = &hostnames[index];
        let mut config = E::build_node_config(deployment, index)?;
        E::rewrite_for_hostnames(deployment, index, &hostnames, &mut config)?;
        let config_yaml = E::serialize_node_config(&config)?;

        docs.push(render_node_config_map(name, &config_yaml));
        docs.push(render_node_deployment(name, spec));
        docs.push(render_node_service(name, spec));
    }

    Ok(docs.join("\n---\n"))
}

fn render_node_config_map(name: &str, config_yaml: &str) -> String {
    format!(
        "apiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: {name}-config\ndata:\n  config.yaml: |\n{}",
        indent_yaml(config_yaml, 4)
    )
}

fn render_node_deployment(name: &str, spec: &BinaryConfigK8sSpec) -> String {
    format!(
        "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  name: {name}\nspec:\n  replicas: 1\n  selector:\n    matchLabels:\n      app: {name}\n  template:\n    metadata:\n      labels:\n        app: {name}\n    spec:\n      containers:\n        - name: app\n          image: {}\n          imagePullPolicy: {}\n          args:\n            - --config\n            - {}\n          ports:\n            - containerPort: {}\n          volumeMounts:\n            - name: config\n              mountPath: {}\n              subPath: config.yaml\n      volumes:\n        - name: config\n          configMap:\n            name: {name}-config",
        k8s_image(spec),
        spec.image_pull_policy,
        spec.config_container_path,
        spec.container_http_port,
        spec.config_container_path,
    )
}

fn render_node_service(name: &str, spec: &BinaryConfigK8sSpec) -> String {
    format!(
        "apiVersion: v1\nkind: Service\nmetadata:\n  name: {name}\nspec:\n  selector:\n    app: {name}\n  type: NodePort\n  ports:\n    - name: api\n      port: {api_port}\n      targetPort: {api_port}\n      protocol: TCP\n    - name: testing\n      port: {testing_port}\n      targetPort: {api_port}\n      protocol: TCP",
        api_port = spec.container_http_port,
        testing_port = spec.service_testing_port
    )
}

fn indent_yaml(value: &str, spaces: usize) -> String {
    let padding = " ".repeat(spaces);
    value
        .lines()
        .map(|line| format!("{padding}{line}"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn k8s_image(spec: &BinaryConfigK8sSpec) -> String {
    env::var(&spec.image_env_var)
        .or_else(|_| env::var(&spec.fallback_image_env_var))
        .unwrap_or_else(|_| spec.default_image.clone())
}

pub fn render_single_template_chart_assets(
    chart_name: &str,
    template_name: &str,
    manifest: &str,
) -> Result<RenderedHelmChartAssets, DynError> {
    let tempdir = tempfile::tempdir()?;
    let chart_path = tempdir.path().join("chart");
    let templates_path = chart_path.join("templates");
    fs::create_dir_all(&templates_path)?;
    fs::write(chart_path.join("Chart.yaml"), render_chart_yaml(chart_name))?;
    fs::write(templates_path.join(template_name), manifest)?;
    Ok(RenderedHelmChartAssets {
        chart_path,
        _tempdir: tempdir,
    })
}

pub fn render_manifest_chart_assets(
    chart_name: &str,
    template_name: &str,
    manifest: &HelmManifest,
) -> Result<RenderedHelmChartAssets, DynError> {
    render_single_template_chart_assets(chart_name, template_name, &manifest.render())
}

pub fn discovered_node_access(host: &str, api_port: u16, auxiliary_port: u16) -> NodeAccess {
    NodeAccess::new(host, api_port).with_testing_port(auxiliary_port)
}

fn render_chart_yaml(chart_name: &str) -> String {
    format!("apiVersion: v2\nname: {chart_name}\nversion: 0.1.0\n")
}

fn normalize_yaml_document(yaml: &str) -> String {
    yaml.trim_start_matches("---\n").trim().to_owned()
}

pub async fn install_helm_release_with_cleanup<A: HelmReleaseAssets>(
    client: &Client,
    assets: &A,
    namespace: &str,
    release: &str,
) -> Result<RunnerCleanup, DynError> {
    let spec = assets
        .release_bundle()
        .install_spec(release.to_owned(), namespace.to_owned());

    install_release(&spec).await?;

    let preserve = env::var("K8S_RUNNER_PRESERVE").is_ok();
    Ok(RunnerCleanup::new(
        client.clone(),
        namespace.to_owned(),
        release.to_owned(),
        preserve,
    ))
}

#[async_trait]
pub trait K8sDeployEnv: Application + Sized {
    type Assets: Send + Sync;

    /// Collect container port specs from the topology.
    fn collect_port_specs(topology: &Self::Deployment) -> PortSpecs;

    /// Build deploy-time assets (charts, config payloads, scripts).
    fn prepare_assets(
        topology: &Self::Deployment,
        metrics_otlp_ingest_url: Option<&Url>,
    ) -> Result<Self::Assets, DynError>;

    /// Install the k8s stack using the prepared assets.
    async fn install_stack(
        client: &Client,
        assets: &Self::Assets,
        namespace: &str,
        release: &str,
        nodes: usize,
    ) -> Result<RunnerCleanup, DynError>
    where
        Self::Assets: HelmReleaseAssets,
    {
        let _ = nodes;
        install_helm_release_with_cleanup(client, assets, namespace, release).await
    }

    /// Provide a namespace/release identifier pair.
    fn cluster_identifiers() -> (String, String) {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or_default();
        let suffix = format!("{stamp:x}-{:x}", process::id());
        (format!("tf-testnet-{suffix}"), String::from("tf-runner"))
    }

    /// Build a single node client from forwarded ports.
    fn node_client_from_ports(
        host: &str,
        api_port: u16,
        auxiliary_port: u16,
    ) -> Result<Self::NodeClient, DynError> {
        <Self as Application>::build_node_client(&discovered_node_access(
            host,
            api_port,
            auxiliary_port,
        ))
    }

    /// Build node clients from forwarded ports.
    fn build_node_clients(
        host: &str,
        node_api_ports: &[u16],
        node_auxiliary_ports: &[u16],
    ) -> Result<Vec<Self::NodeClient>, DynError> {
        node_api_ports
            .iter()
            .zip(node_auxiliary_ports.iter())
            .map(|(&api_port, &auxiliary_port)| {
                Self::node_client_from_ports(host, api_port, auxiliary_port)
            })
            .collect()
    }

    /// Path appended to readiness probe URLs.
    fn node_readiness_path() -> &'static str {
        <Self as Application>::node_readiness_path()
    }

    /// Wait for remote readiness using topology + URLs.
    async fn wait_remote_readiness(
        topology: &Self::Deployment,
        urls: &[Url],
        requirement: HttpReadinessRequirement,
    ) -> Result<(), DynError> {
        let _ = topology;
        let readiness_urls: Vec<_> = urls
            .iter()
            .map(|url| {
                let mut endpoint = url.clone();
                endpoint.set_path(<Self as K8sDeployEnv>::node_readiness_path());
                endpoint
            })
            .collect();
        wait_http_readiness(&readiness_urls, requirement).await?;
        Ok(())
    }

    /// Label used for readiness probe logging.
    fn node_role() -> &'static str {
        "node"
    }

    /// Deployment resource name for a node index.
    fn node_deployment_name(release: &str, index: usize) -> String {
        format!("{release}-node-{index}")
    }

    /// Service resource name for a node index.
    fn node_service_name(release: &str, index: usize) -> String {
        format!("{release}-node-{index}")
    }

    /// Label selector used to discover managed node services in
    /// existing-cluster mode.
    fn attach_node_service_selector(release: &str) -> String {
        format!("app.kubernetes.io/instance={release}")
    }

    /// Wait for HTTP readiness on provided ports for a given host.
    async fn wait_for_node_http(
        ports: &[u16],
        role: &'static str,
        host: &str,
        timeout: Duration,
        poll_interval: Duration,
        requirement: HttpReadinessRequirement,
    ) -> Result<(), DynError> {
        let _ = role;
        let _ = timeout;
        let _ = poll_interval;
        wait_for_http_ports_with_host_and_requirement(
            ports,
            host,
            <Self as K8sDeployEnv>::node_readiness_path(),
            requirement,
        )
        .await?;
        Ok(())
    }

    /// Optional base URL for node client diagnostics.
    fn node_base_url(_client: &Self::NodeClient) -> Option<String> {
        None
    }

    /// Optional cfgsync/bootstrap service reachable from inside the cluster.
    ///
    /// Manual cluster uses this to update one node's served config before
    /// start.
    fn cfgsync_service(_release: &str) -> Option<(String, u16)> {
        None
    }

    /// Hostnames that should be rendered into cfgsync-served node configs.
    fn cfgsync_hostnames(release: &str, node_count: usize) -> Vec<String> {
        (0..node_count)
            .map(|index| Self::node_service_name(release, index))
            .collect()
    }

    /// Optional node-local artifact override for manual cluster startup
    /// options.
    ///
    /// Return `Some(..)` when options require a node-specific config
    /// replacement before the node starts. Return `None` to keep the
    /// original cfgsync artifact set.
    fn build_cfgsync_override_artifacts(
        _topology: &Self::Deployment,
        _node_index: usize,
        _hostnames: &[String],
        _options: &testing_framework_core::scenario::StartNodeOptions<Self>,
    ) -> Result<Option<ArtifactSet>, DynError> {
        Ok(None)
    }
}

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

/// Assets that can be installed as one Helm release.
pub trait HelmReleaseAssets {
    /// Returns the Helm bundle used to install the release.
    fn release_bundle(&self) -> HelmReleaseBundle;
}

#[derive(Debug)]
/// Rendered chart directory backed by a temporary workspace.
pub struct RenderedHelmChartAssets {
    chart_path: PathBuf,
    _tempdir: TempDir,
}

#[derive(Clone, Debug, Default)]
/// In-memory YAML manifest that can be rendered into a single Helm template.
pub struct HelmManifest {
    documents: Vec<String>,
}

#[derive(Clone, Debug)]
/// Standard binary+config k8s node installation contract.
pub struct BinaryConfigK8sSpec {
    /// Helm chart name used for the generated chart.
    pub chart_name: String,
    /// Prefix used for generated node deployment and service names.
    pub node_name_prefix: String,
    /// Binary path inside the container image.
    pub binary_path: String,
    /// Absolute config path inside the container.
    pub config_container_path: String,
    /// Main HTTP port exposed by the container.
    pub container_http_port: u16,
    /// Auxiliary service port exposed by the Service object.
    pub service_testing_port: u16,
    /// Primary environment variable used to override the image.
    pub image_env_var: String,
    /// Secondary fallback environment variable used to override the image.
    pub fallback_image_env_var: String,
    /// Default local image tag used when no override is present.
    pub default_image: String,
    /// Image pull policy written into the generated Deployment.
    pub image_pull_policy: String,
}

impl HelmReleaseAssets for RenderedHelmChartAssets {
    fn release_bundle(&self) -> HelmReleaseBundle {
        HelmReleaseBundle::new(self.chart_path.clone())
    }
}

impl HelmManifest {
    /// Creates an empty manifest.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Serializes one structured value as YAML and appends it as a document.
    pub fn push_yaml<T>(&mut self, value: &T) -> Result<(), DynError>
    where
        T: Serialize,
    {
        self.documents
            .push(normalize_yaml_document(&serde_yaml::to_string(value)?));
        Ok(())
    }

    /// Appends a raw YAML document after trimming surrounding whitespace.
    pub fn push_raw_yaml(&mut self, yaml: &str) {
        let yaml = yaml.trim();
        if !yaml.is_empty() {
            self.documents.push(yaml.to_owned());
        }
    }

    /// Appends all documents from another manifest.
    pub fn extend(&mut self, other: Self) {
        self.documents.extend(other.documents);
    }

    /// Renders all documents separated by `---`.
    #[must_use]
    pub fn render(&self) -> String {
        self.documents.join("\n---\n")
    }
}

/// Builds the standard forwarded port layout for a generated binary-config
/// k8s stack.
pub fn standard_port_specs(node_count: usize, api: u16, auxiliary: u16) -> PortSpecs {
    PortSpecs {
        nodes: (0..node_count)
            .map(|_| crate::wait::NodeConfigPorts { api, auxiliary })
            .collect(),
    }
}

impl BinaryConfigK8sSpec {
    /// Builds the conventional binary-config spec for a generated chart.
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

/// Renders a temporary single-template Helm chart for the standard
/// binary-config node layout.
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

/// Renders the standard ConfigMap, Deployment, and Service objects for each
/// node in a binary-config k8s stack.
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

/// Renders a minimal chart directory with a single YAML template file.
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

/// Renders a chart from an in-memory manifest.
pub fn render_manifest_chart_assets(
    chart_name: &str,
    template_name: &str,
    manifest: &HelmManifest,
) -> Result<RenderedHelmChartAssets, DynError> {
    render_single_template_chart_assets(chart_name, template_name, &manifest.render())
}

/// Converts forwarded k8s ports into generic node access.
pub fn discovered_node_access(host: &str, api_port: u16, auxiliary_port: u16) -> NodeAccess {
    NodeAccess::new(host, api_port).with_testing_port(auxiliary_port)
}

fn render_chart_yaml(chart_name: &str) -> String {
    format!("apiVersion: v2\nname: {chart_name}\nversion: 0.1.0\n")
}

fn normalize_yaml_document(yaml: &str) -> String {
    yaml.trim_start_matches("---\n").trim().to_owned()
}

/// Installs a Helm release and returns the corresponding cleanup handle.
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
/// Prepared k8s stack ready to be installed into a namespace.
pub trait PreparedK8sStack: Send + Sync {
    /// Installs the prepared stack and returns the cleanup handle.
    async fn install(
        &self,
        client: &Client,
        namespace: &str,
        release: &str,
        nodes: usize,
    ) -> Result<RunnerCleanup, DynError>;
}

#[async_trait]
impl<T> PreparedK8sStack for T
where
    T: HelmReleaseAssets + Send + Sync,
{
    async fn install(
        &self,
        client: &Client,
        namespace: &str,
        release: &str,
        nodes: usize,
    ) -> Result<RunnerCleanup, DynError> {
        let _ = nodes;
        install_helm_release_with_cleanup(client, self, namespace, release).await
    }
}

/// Advanced k8s deployer integration.
#[async_trait]
pub trait K8sDeployEnv: Application + Sized {
    /// Prepared stack type produced by this environment.
    type Assets: PreparedK8sStack + Send + Sync + 'static;

    /// Returns the ports that must be exposed and forwarded for the deployment.
    fn collect_port_specs(topology: &Self::Deployment) -> PortSpecs;

    /// Prepares installable assets for the deployment.
    fn prepare_assets(
        topology: &Self::Deployment,
        metrics_otlp_ingest_url: Option<&Url>,
    ) -> Result<Self::Assets, DynError>;

    /// Installs one prepared stack into the target namespace and release name.
    async fn install_stack(
        client: &Client,
        assets: &Self::Assets,
        namespace: &str,
        release: &str,
        nodes: usize,
    ) -> Result<RunnerCleanup, DynError>
    where
        Self::Assets: PreparedK8sStack,
    {
        assets.install(client, namespace, release, nodes).await
    }

    /// Returns the generated namespace and release names for one test run.
    fn cluster_identifiers() -> (String, String) {
        default_cluster_identifiers()
    }

    /// Builds one node client from forwarded k8s ports.
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

    /// Builds node clients for the full deployment.
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

    /// Returns the readiness endpoint path used for remote HTTP probes.
    fn node_readiness_path() -> &'static str {
        <Self as Application>::node_readiness_path()
    }

    /// Waits for remote node readiness after port-forwarding is established.
    async fn wait_remote_readiness(
        _deployment: &Self::Deployment,
        urls: &[Url],
        requirement: HttpReadinessRequirement,
    ) -> Result<(), DynError> {
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

    /// Returns the Kubernetes role label used for node workloads.
    fn node_role() -> &'static str {
        "node"
    }

    /// Returns the Deployment name for one node.
    fn node_deployment_name(release: &str, index: usize) -> String {
        default_node_name(release, index)
    }

    /// Returns the Service name for one node.
    fn node_service_name(release: &str, index: usize) -> String {
        default_node_name(release, index)
    }

    /// Returns the label selector used by attach flows to locate node services.
    fn attach_node_service_selector(release: &str) -> String {
        default_attach_node_service_selector(release)
    }

    /// Waits for direct HTTP readiness against forwarded node ports.
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

    /// Returns the externally reachable base URL for a node client, if the app
    /// needs it for attach or manual flows.
    fn node_base_url(_client: &Self::NodeClient) -> Option<String> {
        None
    }

    /// Returns the cfgsync service host and port when manual-cluster override
    /// flows are enabled.
    fn cfgsync_service(_release: &str) -> Option<(String, u16)> {
        None
    }

    /// Returns cfgsync hostnames for all nodes in the deployment.
    fn cfgsync_hostnames(release: &str, node_count: usize) -> Vec<String> {
        (0..node_count)
            .map(|index| Self::node_service_name(release, index))
            .collect()
    }

    /// Builds app-specific cfgsync override artifacts for one node.
    fn build_cfgsync_override_artifacts(
        _deployment: &Self::Deployment,
        _node_index: usize,
        _hostnames: &[String],
        _options: &testing_framework_core::scenario::StartNodeOptions<Self>,
    ) -> Result<Option<ArtifactSet>, DynError> {
        Ok(None)
    }
}

/// Common binary+config k8s path.
pub trait K8sBinaryApp: Application + StaticNodeConfigProvider + Sized
where
    Self::Deployment: DeploymentDescriptor,
{
    /// Returns the standard binary+config install specification.
    fn k8s_binary_spec() -> BinaryConfigK8sSpec;

    /// Adds extra YAML resources to the generated manifest.
    fn extend_k8s_manifest(
        _deployment: &Self::Deployment,
        _manifest: &mut HelmManifest,
    ) -> Result<(), DynError> {
        Ok(())
    }

    /// Builds node clients for the full deployment.
    fn build_node_clients(
        host: &str,
        node_api_ports: &[u16],
        node_auxiliary_ports: &[u16],
    ) -> Result<Vec<Self::NodeClient>, DynError> {
        node_api_ports
            .iter()
            .zip(node_auxiliary_ports.iter())
            .map(|(&api_port, &auxiliary_port)| {
                <Self as Application>::build_node_client(&discovered_node_access(
                    host,
                    api_port,
                    auxiliary_port,
                ))
            })
            .collect()
    }

    /// Returns the Kubernetes role label used for node workloads.
    fn node_role() -> &'static str {
        "node"
    }

    /// Returns the externally reachable base URL for a node client, if needed.
    fn node_base_url(_client: &Self::NodeClient) -> Option<String> {
        None
    }
}

#[async_trait]
impl<T> K8sDeployEnv for T
where
    T: K8sBinaryApp,
    T::Deployment: DeploymentDescriptor,
{
    type Assets = RenderedHelmChartAssets;

    fn collect_port_specs(topology: &Self::Deployment) -> PortSpecs {
        let spec = T::k8s_binary_spec();
        standard_port_specs(
            topology.node_count(),
            spec.container_http_port,
            spec.service_testing_port,
        )
    }

    fn prepare_assets(
        topology: &Self::Deployment,
        _metrics_otlp_ingest_url: Option<&Url>,
    ) -> Result<Self::Assets, DynError> {
        let spec = T::k8s_binary_spec();
        let mut manifest = HelmManifest::new();
        manifest.push_raw_yaml(&render_binary_config_node_manifest::<T>(topology, &spec)?);
        T::extend_k8s_manifest(topology, &mut manifest)?;
        render_manifest_chart_assets(
            &spec.chart_name,
            &format!("{}.yaml", spec.chart_name),
            &manifest,
        )
    }

    fn build_node_clients(
        host: &str,
        node_api_ports: &[u16],
        node_auxiliary_ports: &[u16],
    ) -> Result<Vec<Self::NodeClient>, DynError> {
        T::build_node_clients(host, node_api_ports, node_auxiliary_ports)
    }

    fn node_role() -> &'static str {
        T::node_role()
    }

    fn node_base_url(client: &Self::NodeClient) -> Option<String> {
        T::node_base_url(client)
    }

    fn node_deployment_name(_release: &str, index: usize) -> String {
        format!("{}-{index}", T::k8s_binary_spec().node_name_prefix)
    }

    fn node_service_name(_release: &str, index: usize) -> String {
        format!("{}-{index}", T::k8s_binary_spec().node_name_prefix)
    }
}

pub(crate) fn collect_port_specs<E: K8sDeployEnv>(deployment: &E::Deployment) -> PortSpecs {
    E::collect_port_specs(deployment)
}

pub(crate) fn prepare_stack<E: K8sDeployEnv>(
    deployment: &E::Deployment,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<Box<dyn PreparedK8sStack>, DynError>
where
    E::Assets: PreparedK8sStack + 'static,
{
    Ok(Box::new(E::prepare_assets(
        deployment,
        metrics_otlp_ingest_url,
    )?))
}

pub(crate) fn cluster_identifiers<E: K8sDeployEnv>() -> (String, String) {
    E::cluster_identifiers()
}

pub(crate) fn build_node_clients<E: K8sDeployEnv>(
    host: &str,
    node_api_ports: &[u16],
    node_auxiliary_ports: &[u16],
) -> Result<Vec<E::NodeClient>, DynError> {
    E::build_node_clients(host, node_api_ports, node_auxiliary_ports)
}

pub(crate) fn node_readiness_path<E: K8sDeployEnv>() -> &'static str {
    <E as K8sDeployEnv>::node_readiness_path()
}

pub(crate) async fn wait_remote_readiness<E: K8sDeployEnv>(
    deployment: &E::Deployment,
    urls: &[Url],
    requirement: HttpReadinessRequirement,
) -> Result<(), DynError> {
    E::wait_remote_readiness(deployment, urls, requirement).await
}

pub(crate) fn node_role<E: K8sDeployEnv>() -> &'static str {
    E::node_role()
}

pub(crate) fn node_deployment_name<E: K8sDeployEnv>(release: &str, index: usize) -> String {
    E::node_deployment_name(release, index)
}

pub(crate) fn node_service_name<E: K8sDeployEnv>(release: &str, index: usize) -> String {
    E::node_service_name(release, index)
}

pub(crate) fn attach_node_service_selector<E: K8sDeployEnv>(release: &str) -> String {
    E::attach_node_service_selector(release)
}

pub(crate) async fn wait_for_node_http<E: K8sDeployEnv>(
    ports: &[u16],
    role: &'static str,
    host: &str,
    timeout: Duration,
    poll_interval: Duration,
    requirement: HttpReadinessRequirement,
) -> Result<(), DynError> {
    E::wait_for_node_http(ports, role, host, timeout, poll_interval, requirement).await
}

pub(crate) fn node_base_url<E: K8sDeployEnv>(client: &E::NodeClient) -> Option<String> {
    E::node_base_url(client)
}

pub(crate) fn cfgsync_service<E: K8sDeployEnv>(release: &str) -> Option<(String, u16)> {
    E::cfgsync_service(release)
}

pub(crate) fn cfgsync_hostnames<E: K8sDeployEnv>(release: &str, node_count: usize) -> Vec<String> {
    E::cfgsync_hostnames(release, node_count)
}

pub(crate) fn build_cfgsync_override_artifacts<E: K8sDeployEnv>(
    deployment: &E::Deployment,
    node_index: usize,
    hostnames: &[String],
    options: &testing_framework_core::scenario::StartNodeOptions<E>,
) -> Result<Option<ArtifactSet>, DynError> {
    E::build_cfgsync_override_artifacts(deployment, node_index, hostnames, options)
}

fn default_cluster_identifiers() -> (String, String) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let suffix = format!("{stamp:x}-{:x}", process::id());
    (format!("tf-testnet-{suffix}"), String::from("tf-runner"))
}

fn default_node_name(release: &str, index: usize) -> String {
    format!("{release}-node-{index}")
}

fn default_attach_node_service_selector(release: &str) -> String {
    format!("app.kubernetes.io/instance={release}")
}

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
pub trait PreparedK8sStack: Send + Sync {
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

type K8sPortSpecsBuilder<E> =
    Box<dyn Fn(&<E as Application>::Deployment) -> PortSpecs + Send + Sync>;
type K8sPreparedStackBuilder<E> = Box<
    dyn Fn(
            &<E as Application>::Deployment,
            Option<&Url>,
        ) -> Result<Box<dyn PreparedK8sStack>, DynError>
        + Send
        + Sync,
>;
type K8sClusterIdentifiers = Box<dyn Fn() -> (String, String) + Send + Sync>;
type K8sIndexedName = Box<dyn Fn(&str, usize) -> String + Send + Sync>;
type K8sAttachSelector = Box<dyn Fn(&str) -> String + Send + Sync>;
type K8sNodeClientsBuilder<E> = Box<
    dyn Fn(&str, &[u16], &[u16]) -> Result<Vec<<E as Application>::NodeClient>, DynError>
        + Send
        + Sync,
>;
type K8sNodeBaseUrl<E> =
    Box<dyn Fn(&<E as Application>::NodeClient) -> Option<String> + Send + Sync>;
type K8sCfgsyncService = Box<dyn Fn(&str) -> Option<(String, u16)> + Send + Sync>;
type K8sCfgsyncHostnames = Box<dyn Fn(&str, usize) -> Vec<String> + Send + Sync>;
type K8sCfgsyncOverrideBuilder<E> = Box<
    dyn Fn(
            &<E as Application>::Deployment,
            usize,
            &[String],
            &testing_framework_core::scenario::StartNodeOptions<E>,
        ) -> Result<Option<ArtifactSet>, DynError>
        + Send
        + Sync,
>;

pub struct K8sRuntime<E: Application> {
    install: K8sInstall<E>,
    access: K8sAccess<E>,
    manual: K8sManual<E>,
}

pub struct K8sInstall<E: Application> {
    collect_port_specs: K8sPortSpecsBuilder<E>,
    prepare_stack: K8sPreparedStackBuilder<E>,
    cluster_identifiers: K8sClusterIdentifiers,
    node_deployment_name: K8sIndexedName,
    node_service_name: K8sIndexedName,
    attach_node_service_selector: K8sAttachSelector,
}

pub struct K8sAccess<E: Application> {
    build_node_clients: K8sNodeClientsBuilder<E>,
    readiness_path: &'static str,
    node_role: &'static str,
    node_base_url: K8sNodeBaseUrl<E>,
}

pub struct K8sManual<E: Application> {
    cfgsync_service: K8sCfgsyncService,
    cfgsync_hostnames: Option<K8sCfgsyncHostnames>,
    build_cfgsync_override_artifacts: K8sCfgsyncOverrideBuilder<E>,
}

impl<E: Application> K8sRuntime<E> {
    #[must_use]
    pub fn new(install: K8sInstall<E>) -> Self {
        Self {
            install,
            access: K8sAccess::default(),
            manual: K8sManual::default(),
        }
    }

    #[must_use]
    pub fn with_access(mut self, access: K8sAccess<E>) -> Self {
        self.access = access;
        self
    }

    #[must_use]
    pub fn with_manual(mut self, manual: K8sManual<E>) -> Self {
        self.manual = manual;
        self
    }
}

impl<E> K8sRuntime<E>
where
    E: Application + StaticNodeConfigProvider,
    E::Deployment: DeploymentDescriptor,
{
    #[must_use]
    pub fn binary_config(spec: BinaryConfigK8sSpec) -> Self {
        let prepare_spec = spec.clone();
        let name_prefix = spec.node_name_prefix.clone();
        let container_http_port = spec.container_http_port;
        let service_testing_port = spec.service_testing_port;

        Self::new(
            K8sInstall::new(
                move |topology: &E::Deployment| {
                    standard_port_specs(
                        topology.node_count(),
                        container_http_port,
                        service_testing_port,
                    )
                },
                move |topology, _metrics_otlp_ingest_url| {
                    let assets =
                        render_binary_config_node_chart_assets::<E>(topology, &prepare_spec)?;
                    Ok(Box::new(assets) as Box<dyn PreparedK8sStack>)
                },
            )
            .with_node_name_prefix(name_prefix),
        )
    }
}

impl<E: Application> K8sInstall<E> {
    #[must_use]
    pub fn new<FP, FA>(collect_port_specs: FP, prepare_stack: FA) -> Self
    where
        FP: Fn(&E::Deployment) -> PortSpecs + Send + Sync + 'static,
        FA: Fn(&E::Deployment, Option<&Url>) -> Result<Box<dyn PreparedK8sStack>, DynError>
            + Send
            + Sync
            + 'static,
    {
        Self {
            collect_port_specs: Box::new(collect_port_specs),
            prepare_stack: Box::new(prepare_stack),
            cluster_identifiers: Box::new(default_cluster_identifiers),
            node_deployment_name: Box::new(default_node_name),
            node_service_name: Box::new(default_node_name),
            attach_node_service_selector: Box::new(default_attach_node_service_selector),
        }
    }

    #[must_use]
    pub fn with_cluster_identifiers<F>(mut self, cluster_identifiers: F) -> Self
    where
        F: Fn() -> (String, String) + Send + Sync + 'static,
    {
        self.cluster_identifiers = Box::new(cluster_identifiers);
        self
    }

    #[must_use]
    pub fn with_node_name_prefix(mut self, prefix: impl Into<String>) -> Self {
        let prefix = prefix.into();
        self.node_deployment_name = Box::new(named_resource(prefix.clone()));
        self.node_service_name = Box::new(named_resource(prefix));
        self
    }

    #[must_use]
    pub fn with_resource_names<FD, FS>(
        mut self,
        node_deployment_name: FD,
        node_service_name: FS,
    ) -> Self
    where
        FD: Fn(&str, usize) -> String + Send + Sync + 'static,
        FS: Fn(&str, usize) -> String + Send + Sync + 'static,
    {
        self.node_deployment_name = Box::new(node_deployment_name);
        self.node_service_name = Box::new(node_service_name);
        self
    }

    #[must_use]
    pub fn with_attach_node_service_selector<F>(mut self, attach_node_service_selector: F) -> Self
    where
        F: Fn(&str) -> String + Send + Sync + 'static,
    {
        self.attach_node_service_selector = Box::new(attach_node_service_selector);
        self
    }
}

impl<E: Application> Default for K8sAccess<E> {
    fn default() -> Self {
        Self {
            build_node_clients: Box::new(default_build_node_clients::<E>),
            readiness_path: E::node_readiness_path(),
            node_role: "node",
            node_base_url: Box::new(|_client| None),
        }
    }
}

impl<E: Application> K8sAccess<E> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_node_clients<F>(mut self, build_node_clients: F) -> Self
    where
        F: Fn(&str, &[u16], &[u16]) -> Result<Vec<E::NodeClient>, DynError> + Send + Sync + 'static,
    {
        self.build_node_clients = Box::new(build_node_clients);
        self
    }

    #[must_use]
    pub fn with_readiness_path(mut self, readiness_path: &'static str) -> Self {
        self.readiness_path = readiness_path;
        self
    }

    #[must_use]
    pub fn with_node_role(mut self, node_role: &'static str) -> Self {
        self.node_role = node_role;
        self
    }

    #[must_use]
    pub fn with_node_base_url<F>(mut self, node_base_url: F) -> Self
    where
        F: Fn(&E::NodeClient) -> Option<String> + Send + Sync + 'static,
    {
        self.node_base_url = Box::new(node_base_url);
        self
    }
}

impl<E: Application> Default for K8sManual<E> {
    fn default() -> Self {
        Self {
            cfgsync_service: Box::new(|_release| None),
            cfgsync_hostnames: None,
            build_cfgsync_override_artifacts: Box::new(
                |_topology, _node_index, _hostnames, _options| Ok(None),
            ),
        }
    }
}

impl<E: Application> K8sManual<E> {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_cfgsync_service<F>(mut self, cfgsync_service: F) -> Self
    where
        F: Fn(&str) -> Option<(String, u16)> + Send + Sync + 'static,
    {
        self.cfgsync_service = Box::new(cfgsync_service);
        self
    }

    #[must_use]
    pub fn with_cfgsync_hostnames<F>(mut self, cfgsync_hostnames: F) -> Self
    where
        F: Fn(&str, usize) -> Vec<String> + Send + Sync + 'static,
    {
        self.cfgsync_hostnames = Some(Box::new(cfgsync_hostnames));
        self
    }

    #[must_use]
    pub fn with_cfgsync_override_artifacts<F>(mut self, build_override_artifacts: F) -> Self
    where
        F: Fn(
                &E::Deployment,
                usize,
                &[String],
                &testing_framework_core::scenario::StartNodeOptions<E>,
            ) -> Result<Option<ArtifactSet>, DynError>
            + Send
            + Sync
            + 'static,
    {
        self.build_cfgsync_override_artifacts = Box::new(build_override_artifacts);
        self
    }
}

pub trait K8sDeployEnv: Application + Sized {
    fn k8s_runtime() -> K8sRuntime<Self>;
}

pub(crate) fn runtime_for<E: K8sDeployEnv>() -> K8sRuntime<E> {
    E::k8s_runtime()
}

pub(crate) fn collect_port_specs<E: K8sDeployEnv>(deployment: &E::Deployment) -> PortSpecs {
    (runtime_for::<E>().install.collect_port_specs)(deployment)
}

pub(crate) fn prepare_stack<E: K8sDeployEnv>(
    deployment: &E::Deployment,
    metrics_otlp_ingest_url: Option<&Url>,
) -> Result<Box<dyn PreparedK8sStack>, DynError> {
    (runtime_for::<E>().install.prepare_stack)(deployment, metrics_otlp_ingest_url)
}

pub(crate) fn cluster_identifiers<E: K8sDeployEnv>() -> (String, String) {
    (runtime_for::<E>().install.cluster_identifiers)()
}

pub(crate) fn build_node_clients<E: K8sDeployEnv>(
    host: &str,
    node_api_ports: &[u16],
    node_auxiliary_ports: &[u16],
) -> Result<Vec<E::NodeClient>, DynError> {
    (runtime_for::<E>().access.build_node_clients)(host, node_api_ports, node_auxiliary_ports)
}

pub(crate) fn node_readiness_path<E: K8sDeployEnv>() -> &'static str {
    runtime_for::<E>().access.readiness_path
}

pub(crate) async fn wait_remote_readiness<E: K8sDeployEnv>(
    _deployment: &E::Deployment,
    urls: &[Url],
    requirement: HttpReadinessRequirement,
) -> Result<(), DynError> {
    let readiness_urls: Vec<_> = urls
        .iter()
        .map(|url| {
            let mut endpoint = url.clone();
            endpoint.set_path(node_readiness_path::<E>());
            endpoint
        })
        .collect();
    wait_http_readiness(&readiness_urls, requirement).await?;
    Ok(())
}

pub(crate) fn node_role<E: K8sDeployEnv>() -> &'static str {
    runtime_for::<E>().access.node_role
}

pub(crate) fn node_deployment_name<E: K8sDeployEnv>(release: &str, index: usize) -> String {
    (runtime_for::<E>().install.node_deployment_name)(release, index)
}

pub(crate) fn node_service_name<E: K8sDeployEnv>(release: &str, index: usize) -> String {
    (runtime_for::<E>().install.node_service_name)(release, index)
}

pub(crate) fn attach_node_service_selector<E: K8sDeployEnv>(release: &str) -> String {
    (runtime_for::<E>().install.attach_node_service_selector)(release)
}

pub(crate) async fn wait_for_node_http<E: K8sDeployEnv>(
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
        node_readiness_path::<E>(),
        requirement,
    )
    .await?;
    Ok(())
}

pub(crate) fn node_base_url<E: K8sDeployEnv>(client: &E::NodeClient) -> Option<String> {
    (runtime_for::<E>().access.node_base_url)(client)
}

pub(crate) fn cfgsync_service<E: K8sDeployEnv>(release: &str) -> Option<(String, u16)> {
    (runtime_for::<E>().manual.cfgsync_service)(release)
}

pub(crate) fn cfgsync_hostnames<E: K8sDeployEnv>(release: &str, node_count: usize) -> Vec<String> {
    let runtime = runtime_for::<E>();
    if let Some(cfgsync_hostnames) = runtime.manual.cfgsync_hostnames {
        return cfgsync_hostnames(release, node_count);
    }

    (0..node_count)
        .map(|index| (runtime.install.node_service_name)(release, index))
        .collect()
}

pub(crate) fn build_cfgsync_override_artifacts<E: K8sDeployEnv>(
    deployment: &E::Deployment,
    node_index: usize,
    hostnames: &[String],
    options: &testing_framework_core::scenario::StartNodeOptions<E>,
) -> Result<Option<ArtifactSet>, DynError> {
    (runtime_for::<E>().manual.build_cfgsync_override_artifacts)(
        deployment, node_index, hostnames, options,
    )
}

fn default_cluster_identifiers() -> (String, String) {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let suffix = format!("{stamp:x}-{:x}", process::id());
    (format!("tf-testnet-{suffix}"), String::from("tf-runner"))
}

fn default_build_node_clients<E: Application>(
    host: &str,
    node_api_ports: &[u16],
    node_auxiliary_ports: &[u16],
) -> Result<Vec<E::NodeClient>, DynError> {
    node_api_ports
        .iter()
        .zip(node_auxiliary_ports.iter())
        .map(|(&api_port, &auxiliary_port)| {
            <E as Application>::build_node_client(&discovered_node_access(
                host,
                api_port,
                auxiliary_port,
            ))
        })
        .collect()
}

fn default_node_name(release: &str, index: usize) -> String {
    format!("{release}-node-{index}")
}

fn default_attach_node_service_selector(release: &str) -> String {
    format!("app.kubernetes.io/instance={release}")
}

fn named_resource(prefix: String) -> impl Fn(&str, usize) -> String + Send + Sync + 'static {
    move |_release, index| format!("{prefix}-{index}")
}

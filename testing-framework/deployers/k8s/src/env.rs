use std::{
    env, fs,
    path::PathBuf,
    process,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use async_trait::async_trait;
use kube::Client;
use reqwest::Url;
use tempfile::TempDir;
use testing_framework_core::scenario::{
    Application, DynError, HttpReadinessRequirement, NodeAccess,
    wait_for_http_ports_with_host_and_requirement, wait_http_readiness,
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

impl HelmReleaseAssets for RenderedHelmChartAssets {
    fn release_bundle(&self) -> HelmReleaseBundle {
        HelmReleaseBundle::new(self.chart_path.clone())
    }
}

pub fn standard_port_specs(node_count: usize, api: u16, auxiliary: u16) -> PortSpecs {
    PortSpecs {
        nodes: (0..node_count)
            .map(|_| crate::wait::NodeConfigPorts { api, auxiliary })
            .collect(),
    }
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

pub fn discovered_node_access(host: &str, api_port: u16, auxiliary_port: u16) -> NodeAccess {
    NodeAccess::new(host, api_port).with_testing_port(auxiliary_port)
}

fn render_chart_yaml(chart_name: &str) -> String {
    format!("apiVersion: v2\nname: {chart_name}\nversion: 0.1.0\n")
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
pub trait K8sDeployEnv: Application {
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
}

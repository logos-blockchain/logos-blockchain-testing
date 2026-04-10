mod deployer;
mod env;
mod host;
mod infrastructure;
mod lifecycle;
mod manual;
mod workspace;
use std::sync::Once;

pub use k8s_openapi;

pub mod wait {
    pub use crate::lifecycle::wait::*;
}

static RUSTLS_PROVIDER: Once = Once::new();

pub(crate) fn ensure_rustls_provider_installed() {
    RUSTLS_PROVIDER.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

pub use deployer::{K8sDeployer, K8sDeploymentMetadata, K8sRunnerError};
pub use env::{
    BinaryConfigK8sSpec, HelmManifest, HelmReleaseAssets, K8sDeployEnv, RenderedHelmChartAssets,
    discovered_node_access, install_helm_release_with_cleanup,
    render_binary_config_node_chart_assets, render_binary_config_node_manifest,
    render_manifest_chart_assets, render_single_template_chart_assets, standard_port_specs,
};
pub use infrastructure::{
    chart_values::{
        BootstrapExtraFile, BootstrapFiles, BootstrapScripts, BootstrapValues, NodeGroup,
        NodePortValues, NodeValues, RunnerAssetLayout, RunnerChartValues, RunnerFiles,
        RunnerValues,
    },
    cluster::PortSpecs,
    helm::{
        HelmError, HelmFileSetting, HelmInstallSpec, HelmReleaseBundle, HelmValueSetting,
        install_release,
    },
    runtime_spec::{NodeRuntimeSpec, RuntimeSpecError, SharedServiceFileSpec, SharedServiceSpec},
};
pub use lifecycle::cleanup::RunnerCleanup;
pub use manual::{ManualCluster, ManualClusterError};
pub use workspace::{
    RequiredPathError, bundled_runner_chart_path, create_temp_workspace, require_existing_paths,
    resolve_optional_relative_dir, resolve_workspace_root, write_temp_file,
};

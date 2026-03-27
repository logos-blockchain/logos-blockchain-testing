use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct RunnerChartValues {
    #[serde(rename = "nodeImage")]
    pub node_image: String,
    #[serde(rename = "nodeImagePullPolicy")]
    pub node_image_pull_policy: String,
    #[serde(rename = "fullnameOverride")]
    pub fullname_override: String,
    #[serde(rename = "assetMountPath")]
    pub asset_mount_path: String,
    pub bootstrap: BootstrapValues,
    pub runner: RunnerValues,
    pub nodes: NodeGroup,
}

impl RunnerChartValues {
    #[must_use]
    pub fn new(
        node_image: String,
        node_image_pull_policy: String,
        fullname_override: String,
        asset_mount_path: String,
        nodes: NodeGroup,
    ) -> Self {
        Self {
            node_image,
            node_image_pull_policy,
            fullname_override,
            asset_mount_path,
            bootstrap: BootstrapValues::disabled(),
            runner: RunnerValues::default(),
            nodes,
        }
    }
}

#[derive(Debug, Clone)]
pub struct RunnerAssetLayout {
    pub asset_mount_path: String,
    pub bootstrap_primary_config_path: String,
    pub bootstrap_artifacts_config_path: String,
    pub bootstrap_start_path: String,
    pub runner_common_start_path: String,
    pub runner_node_start_path: String,
}

impl RunnerAssetLayout {
    #[must_use]
    pub fn under_mount(mount_path: &str) -> Self {
        Self {
            asset_mount_path: mount_path.to_string(),
            bootstrap_primary_config_path: "bootstrap/primary.yaml".to_string(),
            bootstrap_artifacts_config_path: "bootstrap/artifacts.yaml".to_string(),
            bootstrap_start_path: "scripts/bootstrap-start.sh".to_string(),
            runner_common_start_path: "scripts/runner-common-start.sh".to_string(),
            runner_node_start_path: "scripts/runner-node-start.sh".to_string(),
        }
    }

    #[must_use]
    pub fn with_paths(
        mount_path: &str,
        bootstrap_primary_config_path: &str,
        bootstrap_artifacts_config_path: &str,
        bootstrap_start_path: &str,
        runner_common_start_path: &str,
        runner_node_start_path: &str,
    ) -> Self {
        Self {
            asset_mount_path: mount_path.to_string(),
            bootstrap_primary_config_path: bootstrap_primary_config_path.to_string(),
            bootstrap_artifacts_config_path: bootstrap_artifacts_config_path.to_string(),
            bootstrap_start_path: bootstrap_start_path.to_string(),
            runner_common_start_path: runner_common_start_path.to_string(),
            runner_node_start_path: runner_node_start_path.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapValues {
    pub enabled: bool,
    pub image: String,
    #[serde(rename = "imagePullPolicy")]
    pub image_pull_policy: String,
    #[serde(rename = "serviceName")]
    pub service_name: String,
    pub port: u16,
    pub env: BTreeMap<String, String>,
    #[serde(rename = "writableMountPath")]
    pub writable_mount_path: String,
    pub files: BootstrapFiles,
    pub scripts: BootstrapScripts,
}

impl BootstrapValues {
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            image: String::new(),
            image_pull_policy: "IfNotPresent".to_string(),
            service_name: "bootstrap".to_string(),
            port: 0,
            env: BTreeMap::new(),
            writable_mount_path: String::new(),
            files: BootstrapFiles::default(),
            scripts: BootstrapScripts::default(),
        }
    }

    #[must_use]
    pub fn enabled(
        service_name: String,
        image: String,
        image_pull_policy: String,
        port: u16,
    ) -> Self {
        Self {
            enabled: true,
            image,
            image_pull_policy,
            service_name,
            port,
            env: BTreeMap::new(),
            writable_mount_path: String::new(),
            files: BootstrapFiles::default(),
            scripts: BootstrapScripts::default(),
        }
    }

    pub fn apply_layout(&mut self, layout: &RunnerAssetLayout) {
        self.files.primary_config_path = layout.bootstrap_primary_config_path.clone();
        self.files.artifacts_config_path = layout.bootstrap_artifacts_config_path.clone();
        self.scripts.start_path = layout.bootstrap_start_path.clone();
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BootstrapFiles {
    #[serde(rename = "primaryConfig")]
    pub primary_config: String,
    #[serde(rename = "artifactsConfig")]
    pub artifacts_config: String,
    #[serde(rename = "primaryConfigPath")]
    pub primary_config_path: String,
    #[serde(rename = "artifactsConfigPath")]
    pub artifacts_config_path: String,
    #[serde(rename = "extraFiles")]
    pub extra_files: Vec<BootstrapExtraFile>,
}

#[derive(Debug, Clone, Serialize)]
pub struct BootstrapExtraFile {
    pub key: String,
    pub path: String,
    pub content: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct BootstrapScripts {
    pub start: String,
    #[serde(rename = "startPath")]
    pub start_path: String,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RunnerValues {
    pub files: RunnerFiles,
}

impl RunnerValues {
    pub fn apply_layout(&mut self, layout: &RunnerAssetLayout) {
        self.files.common_start_path = layout.runner_common_start_path.clone();
        self.files.node_start_path = layout.runner_node_start_path.clone();
    }
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RunnerFiles {
    #[serde(rename = "commonStart")]
    pub common_start: String,
    #[serde(rename = "nodeStart")]
    pub node_start: String,
    #[serde(rename = "commonStartPath")]
    pub common_start_path: String,
    #[serde(rename = "nodeStartPath")]
    pub node_start_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeGroup {
    pub count: usize,
    pub entries: Vec<NodeValues>,
}

impl NodeGroup {
    #[must_use]
    pub fn new(entries: Vec<NodeValues>) -> Self {
        Self {
            count: entries.len(),
            entries,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NodeValues {
    pub ports: Vec<NodePortValues>,
    pub env: BTreeMap<String, String>,
}

impl NodeValues {
    #[must_use]
    pub fn new(ports: Vec<NodePortValues>, env: BTreeMap<String, String>) -> Self {
        Self { ports, env }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct NodePortValues {
    pub name: String,
    #[serde(rename = "containerPort")]
    pub container_port: u16,
    #[serde(rename = "servicePort")]
    pub service_port: u16,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
}

impl NodePortValues {
    #[must_use]
    pub fn tcp(name: &str, port: u16) -> Self {
        Self {
            name: name.to_string(),
            container_port: port,
            service_port: port,
            protocol: None,
        }
    }

    #[must_use]
    pub fn udp(name: &str, port: u16) -> Self {
        Self {
            name: name.to_string(),
            container_port: port,
            service_port: port,
            protocol: Some("UDP".to_string()),
        }
    }
}

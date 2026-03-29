use std::{collections::BTreeMap, path::PathBuf};

use crate::{
    BootstrapExtraFile, HelmFileSetting, HelmReleaseBundle, HelmValueSetting, NodeGroup,
    RunnerAssetLayout, RunnerChartValues, RunnerFiles, RunnerValues, write_temp_file,
};

#[derive(Debug, Clone)]
pub struct SharedServiceSpec {
    pub service_name: String,
    pub image: String,
    pub image_pull_policy: String,
    pub port: u16,
    pub env: BTreeMap<String, String>,
    pub writable_mount_path: String,
    pub primary_config: String,
    pub artifacts_config: String,
    pub primary_config_file: PathBuf,
    pub start_script_file: PathBuf,
    pub extra_files: Vec<SharedServiceFileSpec>,
}

#[derive(Debug, Clone)]
pub struct SharedServiceFileSpec {
    pub key: String,
    pub path: String,
    pub content: String,
}

impl SharedServiceFileSpec {
    #[must_use]
    pub fn inline(key: String, path: String, content: String) -> Self {
        Self { key, path, content }
    }
}

#[derive(Debug, Clone)]
pub struct NodeRuntimeSpec {
    pub node_image: String,
    pub node_image_pull_policy: String,
    pub fullname_override: String,
    pub layout: RunnerAssetLayout,
    pub nodes: NodeGroup,
    pub shared_service: Option<SharedServiceSpec>,
    pub node_start_script_file: PathBuf,
    pub common_start_script_file: PathBuf,
    pub chart_path: PathBuf,
    pub current_dir: Option<PathBuf>,
}

impl SharedServiceSpec {
    #[must_use]
    pub fn new(
        service_name: String,
        image: String,
        image_pull_policy: String,
        port: u16,
        primary_config: String,
        artifacts_config: String,
        primary_config_file: PathBuf,
        start_script_file: PathBuf,
    ) -> Self {
        Self {
            service_name,
            image,
            image_pull_policy,
            port,
            env: BTreeMap::new(),
            writable_mount_path: String::new(),
            primary_config,
            artifacts_config,
            primary_config_file,
            start_script_file,
            extra_files: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_env(mut self, env: BTreeMap<String, String>) -> Self {
        self.env = env;
        self
    }

    #[must_use]
    pub fn with_writable_mount_path(mut self, path: String) -> Self {
        self.writable_mount_path = path;
        self
    }

    #[must_use]
    pub fn with_extra_files(mut self, files: Vec<SharedServiceFileSpec>) -> Self {
        self.extra_files = files;
        self
    }
}

impl NodeRuntimeSpec {
    #[must_use]
    pub fn chart_values(&self) -> RunnerChartValues {
        let mut values = RunnerChartValues::new(
            self.node_image.clone(),
            self.node_image_pull_policy.clone(),
            self.fullname_override.clone(),
            self.layout.asset_mount_path.clone(),
            self.nodes.clone(),
        );

        values.runner = self.runner_values();

        if let Some(shared) = self.bootstrap_values() {
            values.bootstrap = shared;
        }

        values
    }

    #[must_use]
    pub fn release_bundle(&self) -> HelmReleaseBundle {
        let mut bundle = HelmReleaseBundle::new(self.chart_path.clone());
        bundle.set_values.extend([
            HelmValueSetting {
                key: "nodeImage".to_string(),
                value: self.node_image.clone(),
            },
            HelmValueSetting {
                key: "nodes.count".to_string(),
                value: self.nodes.count.to_string(),
            },
        ]);
        bundle.set_files.extend([
            HelmFileSetting {
                key: "runner.files.nodeStart".to_string(),
                path: self.node_start_script_file.clone(),
            },
            HelmFileSetting {
                key: "runner.files.commonStart".to_string(),
                path: self.common_start_script_file.clone(),
            },
        ]);

        if let Some(shared) = &self.shared_service {
            bundle.set_values.push(HelmValueSetting {
                key: "bootstrap.port".to_string(),
                value: shared.port.to_string(),
            });
            bundle.set_files.extend([
                HelmFileSetting {
                    key: "bootstrap.files.primaryConfig".to_string(),
                    path: shared.primary_config_file.clone(),
                },
                HelmFileSetting {
                    key: "bootstrap.scripts.start".to_string(),
                    path: shared.start_script_file.clone(),
                },
            ]);
        }

        bundle.current_dir = self.current_dir.clone();
        bundle
    }

    pub fn write_values_file(
        &self,
        dir: &std::path::Path,
        name: &str,
    ) -> Result<PathBuf, RuntimeSpecError> {
        let values = self.chart_values();
        let yaml = serde_yaml::to_string(&values).map_err(RuntimeSpecError::Serialize)?;
        write_temp_file(dir, name, yaml).map_err(|source| RuntimeSpecError::Write {
            path: dir.join(name),
            source,
        })
    }

    fn runner_values(&self) -> RunnerValues {
        let mut runner = RunnerValues {
            files: RunnerFiles {
                common_start: String::new(),
                node_start: String::new(),
                common_start_path: String::new(),
                node_start_path: String::new(),
            },
        };

        runner.apply_layout(&self.layout);
        runner
    }

    fn bootstrap_values(&self) -> Option<crate::BootstrapValues> {
        let shared = self.shared_service.as_ref()?;
        let mut bootstrap = crate::BootstrapValues::enabled(
            shared.service_name.clone(),
            shared.image.clone(),
            shared.image_pull_policy.clone(),
            shared.port,
        );

        bootstrap.env = shared.env.clone();
        bootstrap.writable_mount_path = shared.writable_mount_path.clone();
        bootstrap.files.primary_config = shared.primary_config.clone();
        bootstrap.files.artifacts_config = shared.artifacts_config.clone();
        bootstrap.files.extra_files = collect_bootstrap_extra_files(shared);
        bootstrap.scripts.start = String::new();
        bootstrap.apply_layout(&self.layout);

        Some(bootstrap)
    }
}

fn collect_bootstrap_extra_files(shared: &SharedServiceSpec) -> Vec<BootstrapExtraFile> {
    shared
        .extra_files
        .iter()
        .map(|file| BootstrapExtraFile {
            key: file.key.clone(),
            path: file.path.clone(),
            content: file.content.clone(),
        })
        .collect()
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeSpecError {
    #[error("failed to serialize runtime spec values: {0}")]
    Serialize(#[source] serde_yaml::Error),
    #[error("failed to write runtime spec values to {}: {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

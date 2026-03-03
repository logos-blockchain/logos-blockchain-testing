use anyhow::Result;
use cfgsync_core::CfgSyncFile;
use serde::{Deserialize, Serialize};
use testing_framework_core::cfgsync::{CfgsyncEnv, build_cfgsync_node_configs};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgSyncBundle {
    pub nodes: Vec<CfgSyncBundleNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CfgSyncBundleNode {
    pub identifier: String,
    #[serde(default)]
    pub files: Vec<CfgSyncFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_yaml: Option<String>,
}

pub fn build_cfgsync_bundle_with_hostnames<E: CfgsyncEnv>(
    deployment: &E::Deployment,
    hostnames: &[String],
) -> Result<CfgSyncBundle> {
    let nodes = build_cfgsync_node_configs::<E>(deployment, hostnames)?;

    Ok(CfgSyncBundle {
        nodes: nodes
            .into_iter()
            .map(|node| CfgSyncBundleNode {
                identifier: node.identifier,
                files: vec![CfgSyncFile {
                    path: "/config.yaml".to_owned(),
                    content: node.config_yaml,
                }],
                config_yaml: None,
            })
            .collect(),
    })
}

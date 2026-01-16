use std::path::{Path, PathBuf};

use testing_framework_config::constants::{DEFAULT_KZG_CONTAINER_PATH, DEFAULT_KZG_HOST_DIR};
use testing_framework_env as tf_env;

/// Default in-image path for KZG params used by testnet images.
pub const DEFAULT_IN_IMAGE_KZG_PARAMS_PATH: &str = "/opt/nomos/kzg-params/kzgrs_test_params";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KzgMode {
    HostPath,
    InImage,
}

impl KzgMode {
    #[must_use]
    pub fn from_env_or_default() -> Self {
        match tf_env::nomos_kzg_mode().as_deref() {
            Some("hostPath") => Self::HostPath,
            Some("inImage") => Self::InImage,
            None => Self::InImage,
            Some(other) => {
                tracing::warn!(
                    value = other,
                    "unknown NOMOS_KZG_MODE; defaulting to inImage"
                );
                Self::InImage
            }
        }
    }
}

/// Canonical KZG parameters model used by runners and config distribution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KzgParamsSpec {
    pub mode: KzgMode,
    /// Value written into node configs (cfgsync `global_params_path`) and,
    /// where applicable, exported as `NOMOS_KZGRS_PARAMS_PATH` for node
    /// processes.
    pub node_params_path: String,
    /// Host directory that must exist when running in `HostPath` mode.
    pub host_params_dir: Option<PathBuf>,
}

impl KzgParamsSpec {
    #[must_use]
    pub fn for_compose(use_kzg_mount: bool) -> Self {
        let node_params_path = tf_env::nomos_kzgrs_params_path().unwrap_or_else(|| {
            if use_kzg_mount {
                DEFAULT_KZG_CONTAINER_PATH.to_string()
            } else {
                DEFAULT_IN_IMAGE_KZG_PARAMS_PATH.to_string()
            }
        });
        Self {
            mode: if use_kzg_mount {
                KzgMode::HostPath
            } else {
                KzgMode::InImage
            },
            node_params_path,
            host_params_dir: None,
        }
    }

    #[must_use]
    pub fn for_k8s(root: &Path) -> Self {
        let mode = KzgMode::from_env_or_default();
        match mode {
            KzgMode::HostPath => Self {
                mode,
                node_params_path: DEFAULT_KZG_CONTAINER_PATH.to_string(),
                host_params_dir: Some(root.join(
                    tf_env::nomos_kzg_dir_rel().unwrap_or_else(|| DEFAULT_KZG_HOST_DIR.to_string()),
                )),
            },
            KzgMode::InImage => Self {
                mode,
                node_params_path: tf_env::nomos_kzgrs_params_path()
                    .unwrap_or_else(|| DEFAULT_IN_IMAGE_KZG_PARAMS_PATH.to_string()),
                host_params_dir: None,
            },
        }
    }
}

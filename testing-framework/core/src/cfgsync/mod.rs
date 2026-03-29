use std::error::Error;

pub use cfgsync_adapter::*;
use cfgsync_artifacts::{ArtifactFile, ArtifactSet};
use cfgsync_core::render::write_rendered_cfgsync;
pub use cfgsync_core::render::{CfgsyncOutputPaths, RenderedCfgsync};
use serde::Serialize;
use thiserror::Error;

use crate::{
    scenario::{Application, StartNodeOptions},
    topology::DeploymentDescriptor,
};

#[doc(hidden)]
pub type DynCfgsyncError = Box<dyn Error + Send + Sync + 'static>;

#[doc(hidden)]
pub trait StaticArtifactRenderer {
    type Deployment;
    type NodeConfig;
    type Error: Error + Send + Sync + 'static;

    fn node_count(deployment: &Self::Deployment) -> usize;

    fn node_identifier(index: usize) -> String;

    fn build_node_config(
        deployment: &Self::Deployment,
        node_index: usize,
    ) -> Result<Self::NodeConfig, Self::Error>;

    fn rewrite_for_hostnames(
        _deployment: &Self::Deployment,
        _node_index: usize,
        _hostnames: &[String],
        _config: &mut Self::NodeConfig,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn serialize_node_config(config: &Self::NodeConfig) -> Result<String, Self::Error>;
}

#[doc(hidden)]
pub trait StaticNodeConfigProvider: Application + Sized {
    type Error: Error + Send + Sync + 'static;

    fn build_node_config(
        deployment: &Self::Deployment,
        node_index: usize,
    ) -> Result<Self::NodeConfig, Self::Error>;

    fn rewrite_for_hostnames(
        _deployment: &Self::Deployment,
        _node_index: usize,
        _hostnames: &[String],
        _config: &mut Self::NodeConfig,
    ) -> Result<(), Self::Error> {
        Ok(())
    }

    fn serialize_node_config(config: &Self::NodeConfig) -> Result<String, Self::Error>;

    fn build_node_artifacts_for_options(
        _deployment: &Self::Deployment,
        _node_index: usize,
        _hostnames: &[String],
        _options: &StartNodeOptions<Self>,
    ) -> Result<Option<ArtifactSet>, Self::Error> {
        Ok(None)
    }
}

impl<T> StaticArtifactRenderer for T
where
    T: StaticNodeConfigProvider,
    T::Deployment: DeploymentDescriptor,
{
    type Deployment = T::Deployment;
    type NodeConfig = T::NodeConfig;
    type Error = T::Error;

    fn node_count(deployment: &Self::Deployment) -> usize {
        deployment.node_count()
    }

    fn node_identifier(index: usize) -> String {
        format!("node-{index}")
    }

    fn build_node_config(
        deployment: &Self::Deployment,
        node_index: usize,
    ) -> Result<Self::NodeConfig, Self::Error> {
        T::build_node_config(deployment, node_index)
    }

    fn rewrite_for_hostnames(
        deployment: &Self::Deployment,
        node_index: usize,
        hostnames: &[String],
        config: &mut Self::NodeConfig,
    ) -> Result<(), Self::Error> {
        T::rewrite_for_hostnames(deployment, node_index, hostnames, config)
    }

    fn serialize_node_config(config: &Self::NodeConfig) -> Result<String, Self::Error> {
        T::serialize_node_config(config)
    }
}

#[derive(Debug, Error)]
pub enum BuildStaticArtifactsError {
    #[error("cfgsync hostnames mismatch (nodes={nodes}, hostnames={hostnames})")]
    HostnameCountMismatch { nodes: usize, hostnames: usize },
    #[error("cfgsync adapter failed: {source}")]
    Adapter {
        #[source]
        source: DynCfgsyncError,
    },
}

fn adapter_error<E>(source: E) -> BuildStaticArtifactsError
where
    E: Error + Send + Sync + 'static,
{
    BuildStaticArtifactsError::Adapter {
        source: Box::new(source),
    }
}

pub fn build_static_artifacts<E: StaticArtifactRenderer>(
    deployment: &E::Deployment,
    hostnames: &[String],
) -> Result<cfgsync_adapter::MaterializedArtifacts, BuildStaticArtifactsError> {
    let node_count = E::node_count(deployment);

    if node_count != hostnames.len() {
        return Err(BuildStaticArtifactsError::HostnameCountMismatch {
            nodes: node_count,
            hostnames: hostnames.len(),
        });
    }

    let mut output = std::collections::HashMap::with_capacity(node_count);

    for index in 0..node_count {
        let mut node_config = E::build_node_config(deployment, index).map_err(adapter_error)?;
        E::rewrite_for_hostnames(deployment, index, hostnames, &mut node_config)
            .map_err(adapter_error)?;
        let config_yaml = E::serialize_node_config(&node_config).map_err(adapter_error)?;

        output.insert(
            E::node_identifier(index),
            ArtifactSet::new(vec![ArtifactFile::new(
                "/config.yaml".to_string(),
                config_yaml.clone(),
            )]),
        );
    }

    Ok(cfgsync_adapter::MaterializedArtifacts::from_nodes(output))
}

pub fn build_node_artifact_override<E: StaticNodeConfigProvider>(
    deployment: &E::Deployment,
    node_index: usize,
    hostnames: &[String],
    options: &StartNodeOptions<E>,
) -> Result<Option<ArtifactSet>, BuildStaticArtifactsError> {
    E::build_node_artifacts_for_options(deployment, node_index, hostnames, options)
        .map_err(adapter_error)
}

#[doc(hidden)]
pub use build_static_artifacts as build_cfgsync_node_catalog;

pub struct RegistrationServerRenderOptions {
    pub port: Option<u16>,
    pub artifacts_path: Option<String>,
}

#[derive(Debug, Error)]
pub enum RenderRegistrationServerError {
    #[error(transparent)]
    BuildStaticArtifacts(#[from] BuildStaticArtifactsError),
    #[error("failed to serialize cfgsync server config: {source}")]
    SerializeConfig {
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to serialize cfgsync artifacts: {source}")]
    SerializeArtifacts {
        #[source]
        source: serde_yaml::Error,
    },
    #[error("failed to write cfgsync files: {source}")]
    Write {
        #[source]
        source: anyhow::Error,
    },
    #[error("failed to enrich cfgsync artifacts: {source}")]
    Enrich {
        #[source]
        source: DynCfgsyncError,
    },
}

#[derive(Serialize)]
struct RegistrationServerConfig {
    port: u16,
    source: RegistrationServerSource,
}

#[derive(Serialize)]
struct RegistrationServerSource {
    kind: String,
    artifacts_path: String,
}

pub fn render_and_write_registration_server<E, F>(
    deployment: &E::Deployment,
    hostnames: &[String],
    mut options: RegistrationServerRenderOptions,
    output: CfgsyncOutputPaths<'_>,
    enrich_artifacts: F,
) -> Result<RenderedCfgsync, RenderRegistrationServerError>
where
    E: StaticArtifactRenderer,
    F: FnOnce(&mut cfgsync_adapter::MaterializedArtifacts) -> Result<(), DynCfgsyncError>,
{
    ensure_artifacts_path(&mut options.artifacts_path, output.artifacts_path);

    let config_yaml =
        render_registration_server_config(options.port, options.artifacts_path.as_deref())?;

    let mut materialized = build_static_artifacts::<E>(deployment, hostnames)?;
    enrich_artifacts(&mut materialized)
        .map_err(|source| RenderRegistrationServerError::Enrich { source })?;

    let artifacts_yaml = serde_yaml::to_string(&materialized)
        .map_err(|source| RenderRegistrationServerError::SerializeArtifacts { source })?;

    let rendered = RenderedCfgsync {
        config_yaml,
        artifacts_yaml,
    };

    write_rendered_cfgsync(&rendered, output)
        .map_err(|source| RenderRegistrationServerError::Write { source })?;

    Ok(rendered)
}

fn ensure_artifacts_path(
    artifacts_path: &mut Option<String>,
    output_artifacts_path: &std::path::Path,
) {
    if artifacts_path.is_some() {
        return;
    }

    *artifacts_path = Some(
        output_artifacts_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("cfgsync.artifacts.yaml")
            .to_string(),
    );
}

fn render_registration_server_config(
    port: Option<u16>,
    artifacts_path: Option<&str>,
) -> Result<String, RenderRegistrationServerError> {
    serde_yaml::to_string(&registration_server_config(port, artifacts_path))
        .map_err(|source| RenderRegistrationServerError::SerializeConfig { source })
}

fn registration_server_config(
    port: Option<u16>,
    artifacts_path: Option<&str>,
) -> RegistrationServerConfig {
    RegistrationServerConfig {
        port: port.unwrap_or(4400),
        source: RegistrationServerSource {
            kind: "registration".to_string(),
            artifacts_path: artifacts_path
                .unwrap_or("cfgsync.artifacts.yaml")
                .to_string(),
        },
    }
}

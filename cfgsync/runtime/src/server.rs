use std::{fs, path::Path, sync::Arc};

use anyhow::Context as _;
use axum::Router;
use cfgsync_adapter::{
    ArtifactSet, CachedSnapshotMaterializer, MaterializedArtifacts, MaterializedArtifactsSink,
    PersistingSnapshotMaterializer, RegistrationSnapshotMaterializer, SnapshotConfigSource,
};
use cfgsync_core::{
    BundleConfigSource, CfgsyncServerState, NodeArtifactsBundle, NodeConfigSource, RunCfgsyncError,
    build_cfgsync_router, serve_cfgsync,
};
use serde::{Deserialize, de::Error as _};
use thiserror::Error;

/// Runtime cfgsync server config loaded from YAML.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CfgsyncServerConfig {
    pub port: u16,
    pub source: CfgsyncServerSource,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CfgsyncServerSource {
    Bundle { bundle_path: String },
    RegistrationBundle { bundle_path: String },
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LegacyServingMode {
    Bundle,
    Registration,
}

#[derive(Debug, Deserialize)]
struct RawCfgsyncServerConfig {
    port: u16,
    source: Option<CfgsyncServerSource>,
    bundle_path: Option<String>,
    serving_mode: Option<LegacyServingMode>,
}

#[derive(Debug, Error)]
pub enum LoadCfgsyncServerConfigError {
    #[error("failed to read cfgsync config file {path}: {source}")]
    Read {
        path: String,
        #[source]
        source: std::io::Error,
    },
    #[error("failed to parse cfgsync config file {path}: {source}")]
    Parse {
        path: String,
        #[source]
        source: serde_yaml::Error,
    },
}

impl CfgsyncServerConfig {
    /// Loads cfgsync runtime server config from a YAML file.
    pub fn load_from_file(path: &Path) -> Result<Self, LoadCfgsyncServerConfigError> {
        let config_path = path.display().to_string();
        let config_content =
            fs::read_to_string(path).map_err(|source| LoadCfgsyncServerConfigError::Read {
                path: config_path.clone(),
                source,
            })?;

        let raw: RawCfgsyncServerConfig =
            serde_yaml::from_str(&config_content).map_err(|source| {
                LoadCfgsyncServerConfigError::Parse {
                    path: config_path,
                    source,
                }
            })?;

        Self::from_raw(raw).map_err(|source| LoadCfgsyncServerConfigError::Parse {
            path: path.display().to_string(),
            source,
        })
    }

    #[must_use]
    pub fn for_bundle(port: u16, bundle_path: impl Into<String>) -> Self {
        Self {
            port,
            source: CfgsyncServerSource::Bundle {
                bundle_path: bundle_path.into(),
            },
        }
    }

    #[must_use]
    pub fn for_registration_bundle(port: u16, bundle_path: impl Into<String>) -> Self {
        Self {
            port,
            source: CfgsyncServerSource::RegistrationBundle {
                bundle_path: bundle_path.into(),
            },
        }
    }

    fn from_raw(raw: RawCfgsyncServerConfig) -> Result<Self, serde_yaml::Error> {
        let source = match (raw.source, raw.bundle_path, raw.serving_mode) {
            (Some(source), _, _) => source,
            (None, Some(bundle_path), Some(LegacyServingMode::Registration)) => {
                CfgsyncServerSource::RegistrationBundle { bundle_path }
            }
            (None, Some(bundle_path), None | Some(LegacyServingMode::Bundle)) => {
                CfgsyncServerSource::Bundle { bundle_path }
            }
            (None, None, _) => {
                return Err(serde_yaml::Error::custom(
                    "cfgsync server config requires source.kind or legacy bundle_path",
                ));
            }
        };

        Ok(Self {
            port: raw.port,
            source,
        })
    }
}

fn load_bundle_provider(bundle_path: &Path) -> anyhow::Result<Arc<dyn NodeConfigSource>> {
    let provider = BundleConfigSource::from_yaml_file(bundle_path)
        .with_context(|| format!("loading cfgsync provider from {}", bundle_path.display()))?;

    Ok(Arc::new(provider))
}

fn load_registration_source(bundle_path: &Path) -> anyhow::Result<Arc<dyn NodeConfigSource>> {
    let bundle = load_bundle_yaml(bundle_path)?;
    let materialized = build_materialized_artifacts(bundle);
    let provider = SnapshotConfigSource::new(materialized);

    Ok(Arc::new(provider))
}

fn load_bundle_yaml(bundle_path: &Path) -> anyhow::Result<NodeArtifactsBundle> {
    let raw = fs::read_to_string(bundle_path)
        .with_context(|| format!("reading cfgsync bundle from {}", bundle_path.display()))?;

    serde_yaml::from_str(&raw)
        .with_context(|| format!("parsing cfgsync bundle from {}", bundle_path.display()))
}

fn build_materialized_artifacts(bundle: NodeArtifactsBundle) -> MaterializedArtifacts {
    let nodes = bundle
        .nodes
        .into_iter()
        .map(|node| cfgsync_adapter::NodeArtifacts {
            identifier: node.identifier,
            files: node.files,
        })
        .collect();

    MaterializedArtifacts::new(
        cfgsync_adapter::NodeArtifactsCatalog::new(nodes),
        ArtifactSet::new(bundle.shared_files),
    )
}

fn resolve_bundle_path(config_path: &Path, bundle_path: &str) -> std::path::PathBuf {
    let path = Path::new(bundle_path);
    if path.is_absolute() {
        return path.to_path_buf();
    }

    config_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(path)
}

/// Loads runtime config and starts cfgsync HTTP server process.
pub async fn serve_cfgsync_from_config(config_path: &Path) -> anyhow::Result<()> {
    let config = CfgsyncServerConfig::load_from_file(config_path)?;
    let bundle_path = resolve_source_path(config_path, &config.source);

    let state = build_server_state(&config, &bundle_path)?;
    serve_cfgsync(config.port, state).await?;

    Ok(())
}

/// Builds a registration-backed cfgsync router directly from a snapshot
/// materializer.
pub fn build_snapshot_cfgsync_router<M>(materializer: M) -> Router
where
    M: RegistrationSnapshotMaterializer + 'static,
{
    let provider = SnapshotConfigSource::new(CachedSnapshotMaterializer::new(materializer));
    build_cfgsync_router(CfgsyncServerState::new(Arc::new(provider)))
}

/// Builds a registration-backed cfgsync router with a persistence hook for
/// ready materialization results.
pub fn build_persisted_snapshot_cfgsync_router<M, S>(materializer: M, sink: S) -> Router
where
    M: RegistrationSnapshotMaterializer + 'static,
    S: MaterializedArtifactsSink + 'static,
{
    let provider = SnapshotConfigSource::new(CachedSnapshotMaterializer::new(
        PersistingSnapshotMaterializer::new(materializer, sink),
    ));

    build_cfgsync_router(CfgsyncServerState::new(Arc::new(provider)))
}

/// Runs a registration-backed cfgsync server directly from a snapshot
/// materializer.
pub async fn serve_snapshot_cfgsync<M>(port: u16, materializer: M) -> Result<(), RunCfgsyncError>
where
    M: RegistrationSnapshotMaterializer + 'static,
{
    let router = build_snapshot_cfgsync_router(materializer);
    serve_router(port, router).await
}

/// Runs a registration-backed cfgsync server with a persistence hook for ready
/// materialization results.
pub async fn serve_persisted_snapshot_cfgsync<M, S>(
    port: u16,
    materializer: M,
    sink: S,
) -> Result<(), RunCfgsyncError>
where
    M: RegistrationSnapshotMaterializer + 'static,
    S: MaterializedArtifactsSink + 'static,
{
    let router = build_persisted_snapshot_cfgsync_router(materializer, sink);
    serve_router(port, router).await
}

async fn serve_router(port: u16, router: Router) -> Result<(), RunCfgsyncError> {
    let bind_addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .map_err(|source| RunCfgsyncError::Bind { bind_addr, source })?;

    axum::serve(listener, router)
        .await
        .map_err(|source| RunCfgsyncError::Serve { source })?;

    Ok(())
}

fn build_server_state(
    config: &CfgsyncServerConfig,
    source_path: &Path,
) -> anyhow::Result<CfgsyncServerState> {
    let repo = match &config.source {
        CfgsyncServerSource::Bundle { .. } => load_bundle_provider(source_path)?,
        CfgsyncServerSource::RegistrationBundle { .. } => load_registration_source(source_path)?,
    };

    Ok(CfgsyncServerState::new(repo))
}

fn resolve_source_path(config_path: &Path, source: &CfgsyncServerSource) -> std::path::PathBuf {
    match source {
        CfgsyncServerSource::Bundle { bundle_path }
        | CfgsyncServerSource::RegistrationBundle { bundle_path } => {
            resolve_bundle_path(config_path, bundle_path)
        }
    }
}

use std::{fs, path::Path, sync::Arc};

use anyhow::Context as _;
use axum::Router;
use cfgsync_adapter::{
    CachedSnapshotMaterializer, MaterializedArtifacts, MaterializedArtifactsSink,
    PersistingSnapshotMaterializer, RegistrationConfigSource, RegistrationSnapshotMaterializer,
};
use cfgsync_core::{
    BundleConfigSource, CfgsyncServerState, NodeConfigSource, RunCfgsyncError,
    serve_cfgsync as serve_cfgsync_state,
};
use serde::Deserialize;
use thiserror::Error;

/// Runtime cfgsync server config loaded from YAML.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct CfgsyncServerConfig {
    /// HTTP port to bind the cfgsync server on.
    pub port: u16,
    /// Source used by the runtime-managed cfgsync server.
    pub source: CfgsyncServerSource,
}

/// Runtime cfgsync source loaded from config.
///
/// This type is intentionally runtime-oriented:
/// - `Bundle` serves a static precomputed bundle directly
/// - `Registration` serves precomputed artifacts through the registration
///   protocol, which is useful when the consumer wants clients to register
///   before receiving already-materialized artifacts
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CfgsyncServerSource {
    /// Serve a static precomputed artifact bundle directly.
    Bundle { bundle_path: String },
    /// Require node registration before serving precomputed artifacts.
    Registration { artifacts_path: String },
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

        let config: CfgsyncServerConfig =
            serde_yaml::from_str(&config_content).map_err(|source| {
                LoadCfgsyncServerConfigError::Parse {
                    path: config_path,
                    source,
                }
            })?;

        Ok(config)
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

    /// Builds a config that serves a static bundle behind the registration
    /// flow.
    #[must_use]
    pub fn for_registration(port: u16, artifacts_path: impl Into<String>) -> Self {
        Self {
            port,
            source: CfgsyncServerSource::Registration {
                artifacts_path: artifacts_path.into(),
            },
        }
    }
}

fn load_bundle_provider(bundle_path: &Path) -> anyhow::Result<Arc<dyn NodeConfigSource>> {
    let provider = BundleConfigSource::from_yaml_file(bundle_path)
        .with_context(|| format!("loading cfgsync provider from {}", bundle_path.display()))?;

    Ok(Arc::new(provider))
}

fn load_registration_source(artifacts_path: &Path) -> anyhow::Result<Arc<dyn NodeConfigSource>> {
    let materialized = load_materialized_artifacts_yaml(artifacts_path)?;
    let provider = RegistrationConfigSource::new(materialized);

    Ok(Arc::new(provider))
}

fn load_materialized_artifacts_yaml(
    artifacts_path: &Path,
) -> anyhow::Result<MaterializedArtifacts> {
    let raw = fs::read_to_string(artifacts_path).with_context(|| {
        format!(
            "reading cfgsync materialized artifacts from {}",
            artifacts_path.display()
        )
    })?;

    serde_yaml::from_str(&raw).with_context(|| {
        format!(
            "parsing cfgsync materialized artifacts from {}",
            artifacts_path.display()
        )
    })
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
    serve_cfgsync_state(config.port, state).await?;

    Ok(())
}

/// Builds the default registration-backed cfgsync router from a snapshot
/// materializer.
///
/// This is the main code-driven entrypoint for apps that want cfgsync to own:
/// - node registration
/// - readiness polling
/// - artifact serving
///
/// while the app owns only snapshot materialization logic.
pub fn build_cfgsync_router<M>(materializer: M) -> Router
where
    M: RegistrationSnapshotMaterializer + 'static,
{
    let provider = RegistrationConfigSource::new(CachedSnapshotMaterializer::new(materializer));
    cfgsync_core::build_cfgsync_router(CfgsyncServerState::new(Arc::new(provider)))
}

/// Builds a registration-backed cfgsync router with a persistence hook for
/// ready materialization results.
///
/// Use this when the application wants cfgsync to persist or publish shared
/// artifacts after a snapshot becomes ready.
pub fn build_persisted_cfgsync_router<M, S>(materializer: M, sink: S) -> Router
where
    M: RegistrationSnapshotMaterializer + 'static,
    S: MaterializedArtifactsSink + 'static,
{
    let provider = RegistrationConfigSource::new(CachedSnapshotMaterializer::new(
        PersistingSnapshotMaterializer::new(materializer, sink),
    ));

    cfgsync_core::build_cfgsync_router(CfgsyncServerState::new(Arc::new(provider)))
}

/// Runs the default registration-backed cfgsync server directly from a snapshot
/// materializer.
///
/// This is the simplest runtime entrypoint when the application already has a
/// materializer value and does not need to compose extra routes.
pub async fn serve_cfgsync<M>(port: u16, materializer: M) -> Result<(), RunCfgsyncError>
where
    M: RegistrationSnapshotMaterializer + 'static,
{
    let router = build_cfgsync_router(materializer);
    serve_router(port, router).await
}

/// Runs a registration-backed cfgsync server with a persistence hook for ready
/// materialization results.
///
/// This is the direct serving counterpart to
/// [`build_persisted_cfgsync_router`].
pub async fn serve_persisted_cfgsync<M, S>(
    port: u16,
    materializer: M,
    sink: S,
) -> Result<(), RunCfgsyncError>
where
    M: RegistrationSnapshotMaterializer + 'static,
    S: MaterializedArtifactsSink + 'static,
{
    let router = build_persisted_cfgsync_router(materializer, sink);
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
        CfgsyncServerSource::Registration { .. } => load_registration_source(source_path)?,
    };

    Ok(CfgsyncServerState::new(repo))
}

fn resolve_source_path(config_path: &Path, source: &CfgsyncServerSource) -> std::path::PathBuf {
    match source {
        CfgsyncServerSource::Bundle { bundle_path } => {
            resolve_bundle_path(config_path, bundle_path)
        }
        CfgsyncServerSource::Registration { artifacts_path } => {
            resolve_bundle_path(config_path, artifacts_path)
        }
    }
}

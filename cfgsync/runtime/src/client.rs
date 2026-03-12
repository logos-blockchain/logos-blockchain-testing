use std::{
    collections::HashMap,
    env, fs,
    net::Ipv4Addr,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, bail};
use cfgsync_core::{
    CFGSYNC_SCHEMA_VERSION, CfgsyncClient, NodeArtifactFile, NodeArtifactsPayload,
    NodeRegistration, RegistrationPayload,
};
use thiserror::Error;
use tokio::time::{Duration, sleep};
use tracing::info;

const FETCH_ATTEMPTS: usize = 5;
const FETCH_RETRY_DELAY: Duration = Duration::from_millis(250);

/// Output routing for fetched artifact files.
#[derive(Debug, Clone, Default)]
pub struct ArtifactOutputMap {
    routes: HashMap<String, PathBuf>,
}

impl ArtifactOutputMap {
    /// Creates an empty artifact output map.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Routes one artifact path from the payload to a local output path.
    #[must_use]
    pub fn route(
        mut self,
        artifact_path: impl Into<String>,
        output_path: impl Into<PathBuf>,
    ) -> Self {
        self.routes.insert(artifact_path.into(), output_path.into());
        self
    }

    fn resolve_path(&self, file: &NodeArtifactFile) -> PathBuf {
        self.routes
            .get(&file.path)
            .cloned()
            .unwrap_or_else(|| PathBuf::from(&file.path))
    }
}

#[derive(Debug, Error)]
enum ClientEnvError {
    #[error("CFG_HOST_IP `{value}` is not a valid IPv4 address")]
    InvalidIp { value: String },
}

async fn fetch_with_retry(
    payload: &NodeRegistration,
    server_addr: &str,
) -> Result<NodeArtifactsPayload> {
    let client = CfgsyncClient::new(server_addr);

    for attempt in 1..=FETCH_ATTEMPTS {
        match fetch_once(&client, payload).await {
            Ok(config) => return Ok(config),
            Err(error) => {
                if attempt == FETCH_ATTEMPTS {
                    return Err(error).with_context(|| {
                        format!("fetching cfgsync payload after {attempt} attempts")
                    });
                }

                sleep(FETCH_RETRY_DELAY).await;
            }
        }
    }

    unreachable!("cfgsync fetch loop always returns before exhausting attempts");
}

async fn fetch_once(
    client: &CfgsyncClient,
    payload: &NodeRegistration,
) -> Result<NodeArtifactsPayload> {
    let response = client.fetch_node_config(payload).await?;

    Ok(response)
}

async fn register_node(payload: &NodeRegistration, server_addr: &str) -> Result<()> {
    let client = CfgsyncClient::new(server_addr);

    for attempt in 1..=FETCH_ATTEMPTS {
        match client.register_node(payload).await {
            Ok(()) => {
                info!(identifier = %payload.identifier, "cfgsync node registered");
                return Ok(());
            }
            Err(error) => {
                if attempt == FETCH_ATTEMPTS {
                    return Err(error).with_context(|| {
                        format!("registering node with cfgsync after {attempt} attempts")
                    });
                }

                sleep(FETCH_RETRY_DELAY).await;
            }
        }
    }

    unreachable!("cfgsync register loop always returns before exhausting attempts");
}

/// Registers a node and fetches its artifact payload from cfgsync.
pub async fn register_and_fetch_artifacts(
    registration: &NodeRegistration,
    server_addr: &str,
) -> Result<NodeArtifactsPayload> {
    register_node(registration, server_addr).await?;

    let payload = fetch_with_retry(registration, server_addr)
        .await
        .context("fetching cfgsync node config")?;
    ensure_schema_version(&payload)?;

    Ok(payload)
}

/// Registers a node, fetches its artifact payload, and writes the files using
/// the provided output routing policy.
pub async fn fetch_and_write_artifacts(
    registration: &NodeRegistration,
    server_addr: &str,
    outputs: &ArtifactOutputMap,
) -> Result<()> {
    let payload = register_and_fetch_artifacts(registration, server_addr).await?;
    let files = collect_payload_files(&payload)?;

    for file in files {
        write_cfgsync_file(file, outputs)?;
    }

    info!(files = files.len(), "cfgsync files saved");

    Ok(())
}

fn ensure_schema_version(config: &NodeArtifactsPayload) -> Result<()> {
    if config.schema_version != CFGSYNC_SCHEMA_VERSION {
        bail!(
            "unsupported cfgsync payload schema version {}, expected {}",
            config.schema_version,
            CFGSYNC_SCHEMA_VERSION
        );
    }

    Ok(())
}

fn collect_payload_files(config: &NodeArtifactsPayload) -> Result<&[NodeArtifactFile]> {
    if config.is_empty() {
        bail!("cfgsync payload contains no files");
    }

    Ok(config.files())
}

fn write_cfgsync_file(file: &NodeArtifactFile, outputs: &ArtifactOutputMap) -> Result<()> {
    let path = outputs.resolve_path(file);

    ensure_parent_dir(&path)?;

    fs::write(&path, &file.content).with_context(|| format!("writing {}", path.display()))?;

    info!(path = %path.display(), "cfgsync file saved");

    Ok(())
}

fn ensure_parent_dir(path: &Path) -> Result<()> {
    let Some(parent) = path.parent() else {
        return Ok(());
    };

    if parent.as_os_str().is_empty() {
        return Ok(());
    }

    fs::create_dir_all(parent)
        .with_context(|| format!("creating parent directory {}", parent.display()))?;

    Ok(())
}

/// Resolves cfgsync client inputs from environment and materializes node files.
pub async fn run_cfgsync_client_from_env(default_port: u16) -> Result<()> {
    let server_addr =
        env::var("CFG_SERVER_ADDR").unwrap_or_else(|_| format!("http://127.0.0.1:{default_port}"));
    let ip = parse_ip_env(&env::var("CFG_HOST_IP").unwrap_or_else(|_| "127.0.0.1".to_owned()))?;
    let identifier =
        env::var("CFG_HOST_IDENTIFIER").unwrap_or_else(|_| "unidentified-node".to_owned());
    let metadata = parse_registration_payload_env()?;
    let outputs = build_output_map();

    fetch_and_write_artifacts(
        &NodeRegistration::new(identifier, ip).with_payload(metadata),
        &server_addr,
        &outputs,
    )
    .await
}

fn parse_ip_env(ip_str: &str) -> Result<Ipv4Addr> {
    ip_str
        .parse()
        .map_err(|_| ClientEnvError::InvalidIp {
            value: ip_str.to_owned(),
        })
        .map_err(Into::into)
}

fn parse_registration_payload_env() -> Result<RegistrationPayload> {
    let Ok(raw) = env::var("CFG_REGISTRATION_METADATA_JSON") else {
        return Ok(RegistrationPayload::default());
    };

    parse_registration_payload(&raw)
}

fn parse_registration_payload(raw: &str) -> Result<RegistrationPayload> {
    RegistrationPayload::from_json_str(raw).context("parsing CFG_REGISTRATION_METADATA_JSON")
}

fn build_output_map() -> ArtifactOutputMap {
    let mut outputs = ArtifactOutputMap::default();

    if let Ok(path) = env::var("CFG_FILE_PATH") {
        outputs = outputs
            .route("/config.yaml", path.clone())
            .route("config.yaml", path);
    }

    if let Ok(path) = env::var("CFG_DEPLOYMENT_PATH") {
        outputs = outputs
            .route("/deployment.yaml", path.clone())
            .route("deployment-settings.yaml", path.clone())
            .route("/deployment-settings.yaml", path);
    }

    outputs
}

#[cfg(test)]
mod tests {
    use cfgsync_core::{
        CfgsyncServerState, NodeArtifactsBundle, NodeArtifactsBundleEntry, StaticConfigSource,
        serve_cfgsync,
    };
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn client_materializes_multi_file_payload_from_cfgsync_server() {
        let dir = tempdir().expect("create temp dir");
        let app_config_path = dir.path().join("config.yaml");
        let deployment_path = dir.path().join("deployment.yaml");

        let bundle = NodeArtifactsBundle::new(vec![NodeArtifactsBundleEntry {
            identifier: "node-1".to_owned(),
            files: vec![
                NodeArtifactFile::new(app_config_path.to_string_lossy(), "app_key: app_value"),
                NodeArtifactFile::new(deployment_path.to_string_lossy(), "mode: local"),
            ],
        }]);

        let repo = StaticConfigSource::from_bundle(bundle);
        let state = CfgsyncServerState::new(repo);
        let port = allocate_test_port();
        let address = format!("http://127.0.0.1:{port}");
        let server = tokio::spawn(async move {
            serve_cfgsync(port, state)
                .await
                .expect("run cfgsync server");
        });

        fetch_and_write_artifacts(
            &NodeRegistration::new("node-1", "127.0.0.1".parse().expect("parse ip")),
            &address,
            &ArtifactOutputMap::default(),
        )
        .await
        .expect("pull config files");

        server.abort();
        let _ = server.await;

        let app_config = fs::read_to_string(&app_config_path).expect("read app config");
        let deployment = fs::read_to_string(&deployment_path).expect("read deployment config");

        assert_eq!(app_config, "app_key: app_value");
        assert_eq!(deployment, "mode: local");
    }
    fn allocate_test_port() -> u16 {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port for test");
        let port = listener.local_addr().expect("read local addr").port();
        drop(listener);
        port
    }

    #[test]
    fn parses_registration_payload_object() {
        #[derive(Debug, serde::Deserialize, PartialEq, Eq)]
        struct ExamplePayload {
            network_port: u16,
            service: String,
        }

        let metadata = parse_registration_payload(r#"{"network_port":3000,"service":"blend"}"#)
            .expect("parse metadata");
        let payload: ExamplePayload = metadata
            .deserialize()
            .expect("deserialize payload")
            .expect("payload value");

        assert_eq!(
            payload,
            ExamplePayload {
                network_port: 3000,
                service: "blend".to_owned(),
            }
        );
    }

    #[test]
    fn parses_registration_payload_array() {
        let metadata = parse_registration_payload(r#"[1,2,3]"#).expect("parse metadata array");
        let payload: Vec<u8> = metadata
            .deserialize()
            .expect("deserialize payload")
            .expect("payload value");

        assert_eq!(payload, vec![1, 2, 3]);
    }
}

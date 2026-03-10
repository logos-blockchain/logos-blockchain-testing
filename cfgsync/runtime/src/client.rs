use std::{
    env, fs,
    net::Ipv4Addr,
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, bail};
use cfgsync_core::{
    CFGSYNC_SCHEMA_VERSION, CfgSyncClient, CfgSyncFile, CfgSyncPayload, NodeRegistration,
    RegistrationMetadata,
};
use serde_json::Value;
use thiserror::Error;
use tokio::time::{Duration, sleep};
use tracing::info;

const FETCH_ATTEMPTS: usize = 5;
const FETCH_RETRY_DELAY: Duration = Duration::from_millis(250);

#[derive(Debug, Error)]
enum ClientEnvError {
    #[error("CFG_HOST_IP `{value}` is not a valid IPv4 address")]
    InvalidIp { value: String },
    #[error("CFG_REGISTRATION_METADATA_JSON must be a JSON object")]
    InvalidRegistrationMetadataShape,
}

async fn fetch_with_retry(payload: &NodeRegistration, server_addr: &str) -> Result<CfgSyncPayload> {
    let client = CfgSyncClient::new(server_addr);

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

async fn fetch_once(client: &CfgSyncClient, payload: &NodeRegistration) -> Result<CfgSyncPayload> {
    let response = client.fetch_node_config(payload).await?;

    Ok(response)
}

async fn pull_config_files(payload: NodeRegistration, server_addr: &str) -> Result<()> {
    register_node(&payload, server_addr).await?;

    let config = fetch_with_retry(&payload, server_addr)
        .await
        .context("fetching cfgsync node config")?;
    ensure_schema_version(&config)?;

    let files = collect_payload_files(&config)?;

    for file in files {
        write_cfgsync_file(file)?;
    }

    info!(files = files.len(), "cfgsync files saved");

    Ok(())
}

async fn register_node(payload: &NodeRegistration, server_addr: &str) -> Result<()> {
    let client = CfgSyncClient::new(server_addr);

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

fn ensure_schema_version(config: &CfgSyncPayload) -> Result<()> {
    if config.schema_version != CFGSYNC_SCHEMA_VERSION {
        bail!(
            "unsupported cfgsync payload schema version {}, expected {}",
            config.schema_version,
            CFGSYNC_SCHEMA_VERSION
        );
    }

    Ok(())
}

fn collect_payload_files(config: &CfgSyncPayload) -> Result<&[CfgSyncFile]> {
    if config.is_empty() {
        bail!("cfgsync payload contains no files");
    }

    Ok(config.files())
}

fn write_cfgsync_file(file: &CfgSyncFile) -> Result<()> {
    let path = PathBuf::from(&file.path);

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
    let metadata = parse_registration_metadata_env()?;

    pull_config_files(
        NodeRegistration::new(identifier, ip).with_metadata(metadata),
        &server_addr,
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

fn parse_registration_metadata_env() -> Result<RegistrationMetadata> {
    let Ok(raw) = env::var("CFG_REGISTRATION_METADATA_JSON") else {
        return Ok(RegistrationMetadata::default());
    };

    parse_registration_metadata(&raw)
}

fn parse_registration_metadata(raw: &str) -> Result<RegistrationMetadata> {
    let value: Value =
        serde_json::from_str(raw).context("parsing CFG_REGISTRATION_METADATA_JSON")?;
    let Some(metadata) = value.as_object() else {
        return Err(ClientEnvError::InvalidRegistrationMetadataShape.into());
    };

    Ok(RegistrationMetadata::from(metadata.clone()))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use cfgsync_core::{
        CfgSyncBundle, CfgSyncBundleNode, CfgSyncPayload, CfgSyncState, ConfigRepo, run_cfgsync,
    };
    use tempfile::tempdir;

    use super::*;

    #[tokio::test]
    async fn client_materializes_multi_file_payload_from_cfgsync_server() {
        let dir = tempdir().expect("create temp dir");
        let app_config_path = dir.path().join("config.yaml");
        let deployment_path = dir.path().join("deployment.yaml");

        let bundle = CfgSyncBundle::new(vec![CfgSyncBundleNode {
            identifier: "node-1".to_owned(),
            files: vec![
                CfgSyncFile::new(app_config_path.to_string_lossy(), "app_key: app_value"),
                CfgSyncFile::new(deployment_path.to_string_lossy(), "mode: local"),
            ],
        }]);

        let repo = ConfigRepo::from_bundle(bundle_to_payload_map(bundle));
        let state = CfgSyncState::new(repo);
        let port = allocate_test_port();
        let address = format!("http://127.0.0.1:{port}");
        let server = tokio::spawn(async move {
            run_cfgsync(port, state).await.expect("run cfgsync server");
        });

        pull_config_files(
            NodeRegistration::new("node-1", "127.0.0.1".parse().expect("parse ip")),
            &address,
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

    fn bundle_to_payload_map(bundle: CfgSyncBundle) -> HashMap<String, CfgSyncPayload> {
        bundle
            .nodes
            .into_iter()
            .map(|node| {
                let CfgSyncBundleNode { identifier, files } = node;

                (identifier, CfgSyncPayload::from_files(files))
            })
            .collect()
    }

    fn allocate_test_port() -> u16 {
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port for test");
        let port = listener.local_addr().expect("read local addr").port();
        drop(listener);
        port
    }

    #[test]
    fn parses_registration_metadata_object() {
        let metadata = parse_registration_metadata(r#"{"network_port":3000,"service":"blend"}"#)
            .expect("parse metadata");

        assert_eq!(
            metadata.get("network_port"),
            Some(&Value::Number(3000_u16.into()))
        );
        assert_eq!(
            metadata.get("service"),
            Some(&Value::String("blend".to_owned()))
        );
    }

    #[test]
    fn rejects_non_object_registration_metadata() {
        let error = parse_registration_metadata(r#"[1,2,3]"#).expect_err("reject metadata array");

        assert!(
            error
                .to_string()
                .contains("CFG_REGISTRATION_METADATA_JSON must be a JSON object")
        );
    }
}

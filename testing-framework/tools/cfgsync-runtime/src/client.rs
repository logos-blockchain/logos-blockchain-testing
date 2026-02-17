use std::{env, fs, net::Ipv4Addr};

use anyhow::{Context as _, Result};
use cfgsync_core::{CFGSYNC_SCHEMA_VERSION, CfgSyncClient, ClientIp};
use tokio::time::{Duration, sleep};

const FETCH_ATTEMPTS: usize = 5;
const FETCH_RETRY_DELAY: Duration = Duration::from_millis(250);

fn parse_ip(ip_str: &str) -> Ipv4Addr {
    ip_str.parse().unwrap_or(Ipv4Addr::LOCALHOST)
}

async fn fetch_with_retry(
    payload: &ClientIp,
    server_addr: &str,
) -> Result<cfgsync_core::CfgSyncPayload> {
    let client = CfgSyncClient::new(server_addr);
    let mut last_error: Option<anyhow::Error> = None;

    for attempt in 1..=FETCH_ATTEMPTS {
        match client.fetch_node_config(payload).await {
            Ok(config) => return Ok(config),
            Err(error) => {
                last_error = Some(error.into());

                if attempt < FETCH_ATTEMPTS {
                    sleep(FETCH_RETRY_DELAY).await;
                }
            }
        }
    }

    match last_error {
        Some(error) => Err(error),
        None => Err(anyhow::anyhow!(
            "cfgsync client fetch failed without an error"
        )),
    }
}

async fn pull_to_file(payload: ClientIp, server_addr: &str, config_file: &str) -> Result<()> {
    let config = fetch_with_retry(&payload, server_addr)
        .await
        .context("fetching cfgsync node config")?;

    if config.schema_version != CFGSYNC_SCHEMA_VERSION {
        anyhow::bail!(
            "unsupported cfgsync payload schema version {}, expected {}",
            config.schema_version,
            CFGSYNC_SCHEMA_VERSION
        );
    }

    fs::write(config_file, &config.config_yaml)
        .with_context(|| format!("writing config to {}", config_file))?;

    println!("Config saved to {config_file}");
    Ok(())
}

pub async fn run_cfgsync_client_from_env(default_port: u16) -> Result<()> {
    let config_file_path = env::var("CFG_FILE_PATH").unwrap_or_else(|_| "config.yaml".to_owned());
    let server_addr =
        env::var("CFG_SERVER_ADDR").unwrap_or_else(|_| format!("http://127.0.0.1:{default_port}"));
    let ip = parse_ip(&env::var("CFG_HOST_IP").unwrap_or_else(|_| "127.0.0.1".to_owned()));
    let identifier =
        env::var("CFG_HOST_IDENTIFIER").unwrap_or_else(|_| "unidentified-node".to_owned());

    pull_to_file(ClientIp { ip, identifier }, &server_addr, &config_file_path).await
}

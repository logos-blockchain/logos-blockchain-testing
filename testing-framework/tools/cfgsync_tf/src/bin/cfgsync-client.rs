use std::{env, fs, net::Ipv4Addr, process};

use cfgsync_tf::{
    client::{FetchedConfig, get_config},
    server::ClientIp,
};
use nomos_node::Config as NodeConfig;
use serde::{Serialize, de::DeserializeOwned};
use testing_framework_config::constants::cfgsync_port as default_cfgsync_port;
use testing_framework_core::nodes::common::config::injection::{
    inject_ibd_into_cryptarchia, normalize_ed25519_sigs,
};

fn parse_ip(ip_str: &str) -> Ipv4Addr {
    ip_str.parse().unwrap_or_else(|_| {
        eprintln!("Invalid IP format, defaulting to 127.0.0.1");
        Ipv4Addr::LOCALHOST
    })
}

async fn pull_to_file<Config>(payload: ClientIp, url: &str, config_file: &str) -> Result<(), String>
where
    Config: Serialize + DeserializeOwned,
{
    let FetchedConfig {
        config,
        raw: _unused,
    } = get_config::<Config>(payload, url).await?;

    let mut yaml_value = serde_yaml::to_value(&config)
        .map_err(|err| format!("Failed to serialize config to YAML value: {err}"))?;
    inject_ibd_into_cryptarchia(&mut yaml_value);
    normalize_ed25519_sigs(&mut yaml_value);
    let yaml = serde_yaml::to_string(&yaml_value)
        .map_err(|err| format!("Failed to serialize config to YAML: {err}"))?;

    fs::write(config_file, yaml).map_err(|err| format!("Failed to write config to file: {err}"))?;

    println!("Config saved to {config_file}");
    Ok(())
}

#[tokio::main]
async fn main() {
    let config_file_path = env::var("CFG_FILE_PATH").unwrap_or_else(|_| "config.yaml".to_owned());
    let server_addr = env::var("CFG_SERVER_ADDR")
        .unwrap_or_else(|_| format!("http://127.0.0.1:{}", default_cfgsync_port()));
    let ip = parse_ip(&env::var("CFG_HOST_IP").unwrap_or_else(|_| "127.0.0.1".to_owned()));
    let identifier =
        env::var("CFG_HOST_IDENTIFIER").unwrap_or_else(|_| "unidentified-node".to_owned());

    let network_port = env::var("CFG_NETWORK_PORT")
        .ok()
        .and_then(|v| v.parse().ok());
    let blend_port = env::var("CFG_BLEND_PORT").ok().and_then(|v| v.parse().ok());
    let api_port = env::var("CFG_API_PORT").ok().and_then(|v| v.parse().ok());
    let testing_http_port = env::var("CFG_TESTING_HTTP_PORT")
        .ok()
        .and_then(|v| v.parse().ok());

    let payload = ClientIp {
        ip,
        identifier,
        network_port,
        blend_port,
        api_port,
        testing_http_port,
    };

    let node_config_endpoint = format!("{server_addr}/node");

    let config_result =
        pull_to_file::<NodeConfig>(payload, &node_config_endpoint, &config_file_path).await;

    // Handle error if the config request fails
    if let Err(err) = config_result {
        eprintln!("Error: {err}");
        process::exit(1);
    }
}

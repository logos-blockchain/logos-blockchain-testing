use std::{
    collections::{HashMap, HashSet},
    env, fs,
    net::Ipv4Addr,
    process,
    str::FromStr,
};

use cfgsync_tf::{
    client::{FetchedConfig, get_config},
    server::ClientIp,
};
use nomos_executor::config::Config as ExecutorConfig;
use nomos_libp2p::PeerId;
use nomos_node::Config as ValidatorConfig;
use serde::{Serialize, de::DeserializeOwned};
use subnetworks_assignations::{MembershipCreator, MembershipHandler, SubnetworkId};
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

fn parse_assignations(raw: &serde_json::Value) -> Option<HashMap<SubnetworkId, HashSet<PeerId>>> {
    let assignations = raw
        .pointer("/da_network/membership/assignations")?
        .as_object()?;
    let mut result = HashMap::new();

    for (subnetwork, peers) in assignations {
        let subnetwork_id = SubnetworkId::from_str(subnetwork).ok()?;
        let mut members = HashSet::new();

        for peer in peers.as_array()? {
            if let Some(peer) = peer.as_str().and_then(|p| PeerId::from_str(p).ok()) {
                members.insert(peer);
            }
        }

        result.insert(subnetwork_id, members);
    }

    Some(result)
}

fn apply_da_assignations<
    Membership: MembershipCreator<Id = PeerId> + MembershipHandler<NetworkId = SubnetworkId>,
>(
    membership: &Membership,
    assignations: HashMap<SubnetworkId, HashSet<PeerId>>,
) -> Membership {
    let session_id = membership.session_id();
    membership.init(session_id, assignations)
}

async fn pull_to_file<Config, F>(
    payload: ClientIp,
    url: &str,
    config_file: &str,
    apply_membership: F,
) -> Result<(), String>
where
    Config: Serialize + DeserializeOwned,
    F: FnOnce(&mut Config, HashMap<SubnetworkId, HashSet<PeerId>>),
{
    let FetchedConfig { mut config, raw } = get_config::<Config>(payload, url).await?;

    if let Some(assignations) = parse_assignations(&raw) {
        apply_membership(&mut config, assignations);
    }

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

    let host_kind = env::var("CFG_HOST_KIND").unwrap_or_else(|_| "validator".to_owned());

    let network_port = env::var("CFG_NETWORK_PORT")
        .ok()
        .and_then(|v| v.parse().ok());
    let da_port = env::var("CFG_DA_PORT").ok().and_then(|v| v.parse().ok());
    let blend_port = env::var("CFG_BLEND_PORT").ok().and_then(|v| v.parse().ok());
    let api_port = env::var("CFG_API_PORT").ok().and_then(|v| v.parse().ok());
    let testing_http_port = env::var("CFG_TESTING_HTTP_PORT")
        .ok()
        .and_then(|v| v.parse().ok());

    let payload = ClientIp {
        ip,
        identifier,
        network_port,
        da_port,
        blend_port,
        api_port,
        testing_http_port,
    };

    let node_config_endpoint = match host_kind.as_str() {
        "executor" => format!("{server_addr}/executor"),
        _ => format!("{server_addr}/validator"),
    };

    let config_result = match host_kind.as_str() {
        "executor" => {
            pull_to_file::<ExecutorConfig, _>(
                payload,
                &node_config_endpoint,
                &config_file_path,
                |config, assignations| {
                    config.da_network.membership =
                        apply_da_assignations(&config.da_network.membership, assignations);
                },
            )
            .await
        }
        _ => {
            pull_to_file::<ValidatorConfig, _>(
                payload,
                &node_config_endpoint,
                &config_file_path,
                |config, assignations| {
                    config.da_network.membership =
                        apply_da_assignations(&config.da_network.membership, assignations);
                },
            )
            .await
        }
    };

    // Handle error if the config request fails
    if let Err(err) = config_result {
        eprintln!("Error: {err}");
        process::exit(1);
    }
}

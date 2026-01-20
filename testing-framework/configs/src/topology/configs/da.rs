use std::{
    collections::{HashMap, HashSet},
    env, io,
    path::{Path, PathBuf},
    process,
    str::FromStr as _,
    sync::LazyLock,
    time::Duration,
};

use key_management_system_service::keys::{Ed25519Key, ZkKey};
use nomos_core::sdp::SessionNumber;
use nomos_da_network_core::swarm::{
    DAConnectionMonitorSettings, DAConnectionPolicySettings, ReplicationConfig,
};
use nomos_libp2p::{Multiaddr, PeerId, ed25519};
use nomos_node::LogosBlockchainDaMembership;
use num_bigint::BigUint;
use rand::random;
use subnetworks_assignations::{MembershipCreator as _, MembershipHandler as _};
use testing_framework_env as tf_env;
use thiserror::Error;
use tracing::warn;

use crate::{constants::DEFAULT_KZG_HOST_DIR, secret_key_to_peer_id};

pub static GLOBAL_PARAMS_PATH: LazyLock<String> = LazyLock::new(resolve_global_params_path);

const DEFAULT_OLD_BLOBS_CHECK_INTERVAL: Duration = Duration::from_secs(5);
const DEFAULT_BLOBS_VALIDITY_DURATION: Duration = Duration::from_secs(60);
const DEFAULT_FAILURE_TIME_WINDOW: Duration = Duration::from_secs(5);
const DEFAULT_BALANCER_INTERVAL: Duration = Duration::from_secs(1);
const DEFAULT_SEEN_MESSAGE_TTL: Duration = Duration::from_secs(3600);
const DEFAULT_SUBNETS_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

fn canonicalize_params_path(mut path: PathBuf) -> PathBuf {
    if path.is_dir() {
        let candidates = [
            path.join("kzgrs_test_params"),
            path.join("pol/proving_key.zkey"),
            path.join("proving_key.zkey"),
        ];
        if let Some(file) = candidates.iter().find(|p| p.is_file()) {
            return file.clone();
        }
    }
    if let Ok(resolved) = path.canonicalize() {
        path = resolved;
    }
    path
}

fn resolve_global_params_path() -> String {
    if let Some(path) = tf_env::nomos_kzgrs_params_path() {
        return canonicalize_params_path(PathBuf::from(path))
            .to_string_lossy()
            .to_string();
    }

    let workspace_root = env::var("CARGO_WORKSPACE_DIR")
        .map(PathBuf::from)
        .ok()
        .or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
        })
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")));

    let params_path = canonicalize_params_path(
        workspace_root.join(
            testing_framework_env::nomos_kzg_dir_rel()
                .unwrap_or_else(|| DEFAULT_KZG_HOST_DIR.to_string()),
        ),
    );
    match params_path.canonicalize() {
        Ok(path) => path.to_string_lossy().to_string(),
        Err(err) => {
            warn!(
                ?err,
                path = %params_path.display(),
                "falling back to non-canonical KZG params path; set NOMOS_KZGRS_PARAMS_PATH to override"
            );
            params_path.to_string_lossy().to_string()
        }
    }
}

#[derive(Clone)]
pub struct DaParams {
    pub subnetwork_size: usize,
    pub dispersal_factor: usize,
    pub num_samples: u16,
    pub num_subnets: u16,
    pub old_blobs_check_interval: Duration,
    pub blobs_validity_duration: Duration,
    pub global_params_path: String,
    pub policy_settings: DAConnectionPolicySettings,
    pub monitor_settings: DAConnectionMonitorSettings,
    pub balancer_interval: Duration,
    pub redial_cooldown: Duration,
    pub replication_settings: ReplicationConfig,
    pub subnets_refresh_interval: Duration,
    pub retry_shares_limit: usize,
    pub retry_commitments_limit: usize,
}

impl Default for DaParams {
    fn default() -> Self {
        Self {
            subnetwork_size: 2,
            dispersal_factor: 1,
            num_samples: 1,
            num_subnets: 2,
            old_blobs_check_interval: DEFAULT_OLD_BLOBS_CHECK_INTERVAL,
            blobs_validity_duration: DEFAULT_BLOBS_VALIDITY_DURATION,
            global_params_path: GLOBAL_PARAMS_PATH.to_string(),
            policy_settings: DAConnectionPolicySettings {
                min_dispersal_peers: 1,
                min_replication_peers: 1,
                max_dispersal_failures: 0,
                max_sampling_failures: 0,
                max_replication_failures: 0,
                malicious_threshold: 0,
            },
            monitor_settings: DAConnectionMonitorSettings {
                failure_time_window: DEFAULT_FAILURE_TIME_WINDOW,
                ..Default::default()
            },
            balancer_interval: DEFAULT_BALANCER_INTERVAL,
            redial_cooldown: Duration::ZERO,
            replication_settings: ReplicationConfig {
                seen_message_cache_size: 1000,
                seen_message_ttl: DEFAULT_SEEN_MESSAGE_TTL,
            },
            subnets_refresh_interval: DEFAULT_SUBNETS_REFRESH_INTERVAL,
            retry_shares_limit: 1,
            retry_commitments_limit: 1,
        }
    }
}

#[derive(Debug, Clone)]
pub struct GeneralDaConfig {
    pub node_key: ed25519::SecretKey,
    pub signer: Ed25519Key,
    pub peer_id: PeerId,
    pub membership: LogosBlockchainDaMembership,
    pub listening_address: Multiaddr,
    pub blob_storage_directory: PathBuf,
    pub global_params_path: String,
    pub verifier_sk: String,
    pub verifier_index: HashSet<u16>,
    pub num_samples: u16,
    pub num_subnets: u16,
    pub old_blobs_check_interval: Duration,
    pub blobs_validity_duration: Duration,
    pub policy_settings: DAConnectionPolicySettings,
    pub monitor_settings: DAConnectionMonitorSettings,
    pub balancer_interval: Duration,
    pub redial_cooldown: Duration,
    pub replication_settings: ReplicationConfig,
    pub subnets_refresh_interval: Duration,
    pub retry_shares_limit: usize,
    pub retry_commitments_limit: usize,
    pub secret_zk_key: ZkKey,
}

#[derive(Debug, Error)]
pub enum DaConfigError {
    #[error("DA ports length mismatch (ids={ids}, ports={ports})")]
    PortsLenMismatch { ids: usize, ports: usize },
    #[error(
        "DA subnetwork size too large for u16 subnetwork ids (effective_subnetwork_size={effective_subnetwork_size}, max={max})"
    )]
    SubnetworkTooLarge {
        effective_subnetwork_size: usize,
        max: usize,
    },
    #[error("failed to derive node key from bytes: {message}")]
    NodeKeyFromBytes { message: String },
    #[error("failed to create DA listening address for port {port}: {message}")]
    ListeningAddress { port: u16, message: String },
    #[error("failed to create blob storage directory at {path}: {source}")]
    BlobStorageCreate {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to generate verifier secret key: {message}")]
    VerifierKeyGen { message: String },
}

pub fn try_create_da_configs(
    ids: &[[u8; 32]],
    da_params: &DaParams,
    ports: &[u16],
) -> Result<Vec<GeneralDaConfig>, DaConfigError> {
    // Let the subnetwork size track the participant count so tiny local topologies
    // can form a membership.
    let effective_subnetwork_size = da_params.subnetwork_size.max(ids.len().max(1));
    let max_subnetworks = u16::MAX as usize + 1;
    if effective_subnetwork_size > max_subnetworks {
        return Err(DaConfigError::SubnetworkTooLarge {
            effective_subnetwork_size,
            max: max_subnetworks,
        });
    }
    if ports.len() < ids.len() {
        return Err(DaConfigError::PortsLenMismatch {
            ids: ids.len(),
            ports: ports.len(),
        });
    }
    let mut node_keys = Vec::with_capacity(ids.len());
    let mut peer_ids = Vec::with_capacity(ids.len());
    let mut listening_addresses = Vec::with_capacity(ids.len());

    for (index, id) in ids.iter().enumerate() {
        let mut node_key_bytes = *id;
        let node_key = ed25519::SecretKey::try_from_bytes(&mut node_key_bytes).map_err(|err| {
            DaConfigError::NodeKeyFromBytes {
                message: err.to_string(),
            }
        })?;
        node_keys.push(node_key.clone());

        let peer_id = secret_key_to_peer_id(node_key);
        peer_ids.push(peer_id);

        let port = ports[index];
        let listening_address = Multiaddr::from_str(&format!("/ip4/127.0.0.1/udp/{port}/quic-v1",))
            .map_err(|err| DaConfigError::ListeningAddress {
                port,
                message: err.to_string(),
            })?;
        listening_addresses.push(listening_address);
    }

    let membership = {
        let template = LogosBlockchainDaMembership::new(
            SessionNumber::default(),
            effective_subnetwork_size,
            da_params.dispersal_factor,
        );
        let mut assignations: HashMap<u16, HashSet<PeerId>> = HashMap::new();
        if peer_ids.is_empty() {
            for id in 0..effective_subnetwork_size {
                assignations.insert(id as u16, HashSet::new());
            }
        } else {
            let mut sorted_peers = peer_ids.clone();
            sorted_peers.sort_unstable();
            let dispersal = da_params.dispersal_factor.max(1);
            let mut peer_cycle = sorted_peers.iter().cycle();
            for id in 0..effective_subnetwork_size {
                let mut members = HashSet::new();
                for _ in 0..dispersal {
                    // cycle() only yields None when the iterator is empty, which we guard against.
                    if let Some(peer) = peer_cycle.next() {
                        members.insert(*peer);
                    }
                }
                assignations.insert(id as u16, members);
            }
        }

        template.init(SessionNumber::default(), assignations)
    };

    let mut configs = Vec::with_capacity(ids.len());

    for ((index, id), node_key) in ids.iter().enumerate().zip(node_keys.into_iter()) {
        let blob_storage_directory = env::temp_dir().join(format!(
            "nomos-da-blob-{}-{index}-{}",
            process::id(),
            random::<u64>()
        ));
        std::fs::create_dir_all(&blob_storage_directory).map_err(|source| {
            DaConfigError::BlobStorageCreate {
                path: blob_storage_directory.clone(),
                source,
            }
        })?;

        let verifier_sk = blst::min_sig::SecretKey::key_gen(id, &[]).map_err(|err| {
            DaConfigError::VerifierKeyGen {
                message: format!("{err:?}"),
            }
        })?;
        let verifier_sk_bytes = verifier_sk.to_bytes();

        let peer_id = peer_ids[index];
        let signer = Ed25519Key::from_bytes(id);
        let subnetwork_ids = membership.membership(&peer_id);

        let secret_zk_key = ZkKey::from(BigUint::from_bytes_le(signer.public_key().as_bytes()));

        configs.push(GeneralDaConfig {
            node_key,
            signer,
            peer_id,
            secret_zk_key,
            membership: membership.clone(),
            listening_address: listening_addresses[index].clone(),
            blob_storage_directory,
            global_params_path: da_params.global_params_path.clone(),
            verifier_sk: hex::encode(verifier_sk_bytes),
            verifier_index: subnetwork_ids,
            num_samples: da_params.num_samples,
            num_subnets: da_params.num_subnets,
            old_blobs_check_interval: da_params.old_blobs_check_interval,
            blobs_validity_duration: da_params.blobs_validity_duration,
            policy_settings: da_params.policy_settings.clone(),
            monitor_settings: da_params.monitor_settings.clone(),
            balancer_interval: da_params.balancer_interval,
            redial_cooldown: da_params.redial_cooldown,
            replication_settings: da_params.replication_settings,
            subnets_refresh_interval: da_params.subnets_refresh_interval,
            retry_shares_limit: da_params.retry_shares_limit,
            retry_commitments_limit: da_params.retry_commitments_limit,
        });
    }

    Ok(configs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_create_da_configs_rejects_subnetwork_overflow() {
        let ids = vec![[1u8; 32]];
        let ports = vec![12345u16];
        let mut params = DaParams::default();
        params.subnetwork_size = u16::MAX as usize + 2;

        let err = try_create_da_configs(&ids, &params, &ports).unwrap_err();
        assert!(matches!(err, DaConfigError::SubnetworkTooLarge { .. }));
    }

    #[test]
    fn try_create_da_configs_rejects_port_mismatch() {
        let ids = vec![[1u8; 32], [2u8; 32]];
        let ports = vec![12345u16];
        let params = DaParams::default();

        let err = try_create_da_configs(&ids, &params, &ports).unwrap_err();
        assert!(matches!(err, DaConfigError::PortsLenMismatch { .. }));
    }
}

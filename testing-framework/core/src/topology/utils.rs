use std::{collections::HashMap, iter};

use lb_groth16::fr_to_bytes;
use lb_key_management_system_service::{backend::preload::PreloadKMSBackendSettings, keys::Key};
use lb_utils::net::get_available_udp_port;
use rand::{Rng, thread_rng};
use thiserror::Error;

use crate::topology::configs::{
    blend::GeneralBlendConfig, consensus::GeneralConsensusConfig, wallet::WalletAccount,
};

#[must_use]
/// Build preload KMS configs for blend/DA and wallet keys for every node.
pub fn create_kms_configs(
    blend_configs: &[GeneralBlendConfig],
    consensus_configs: &[GeneralConsensusConfig],
    wallet_accounts: &[WalletAccount],
) -> Vec<PreloadKMSBackendSettings> {
    blend_configs
        .iter()
        .zip(consensus_configs.iter())
        .map(|(blend_conf, consensus_conf)| {
            let mut keys = HashMap::from([
                (
                    hex::encode(blend_conf.signer.public_key().to_bytes()),
                    Key::Ed25519(blend_conf.signer.clone()),
                ),
                (
                    hex::encode(fr_to_bytes(
                        blend_conf.secret_zk_key.to_public_key().as_fr(),
                    )),
                    Key::Zk(blend_conf.secret_zk_key.clone()),
                ),
                (
                    hex::encode(fr_to_bytes(
                        consensus_conf.leader_sk.to_public_key().as_fr(),
                    )),
                    Key::Zk(consensus_conf.leader_sk.clone().into()),
                ),
                (
                    hex::encode(fr_to_bytes(
                        consensus_conf.funding_sk.to_public_key().as_fr(),
                    )),
                    Key::Zk(consensus_conf.funding_sk.clone()),
                ),
            ]);

            for account in wallet_accounts {
                let key_id = hex::encode(fr_to_bytes(account.public_key().as_fr()));
                keys.entry(key_id)
                    .or_insert_with(|| Key::Zk(account.secret_key.clone()));
            }

            PreloadKMSBackendSettings { keys }
        })
        .collect()
}

#[derive(Debug, Error)]
pub enum TopologyResolveError {
    #[error("expected {expected} ids but got {actual}")]
    IdCountMismatch { expected: usize, actual: usize },
    #[error("expected {expected} {label} ports but got {actual}")]
    PortCountMismatch {
        label: &'static str,
        expected: usize,
        actual: usize,
    },
    #[error("failed to allocate a free UDP port for {label}")]
    PortAllocationFailed { label: &'static str },
}

pub fn resolve_ids(
    ids: Option<Vec<[u8; 32]>>,
    count: usize,
) -> Result<Vec<[u8; 32]>, TopologyResolveError> {
    match ids {
        Some(ids) => {
            if ids.len() != count {
                return Err(TopologyResolveError::IdCountMismatch {
                    expected: count,
                    actual: ids.len(),
                });
            }
            Ok(ids)
        }
        None => {
            let mut generated = vec![[0; 32]; count];
            for id in &mut generated {
                thread_rng().fill(id);
            }
            Ok(generated)
        }
    }
}

pub fn resolve_ports(
    ports: Option<Vec<u16>>,
    count: usize,
    label: &'static str,
) -> Result<Vec<u16>, TopologyResolveError> {
    let resolved = match ports {
        Some(ports) => ports,
        None => iter::repeat_with(|| {
            get_available_udp_port().ok_or(TopologyResolveError::PortAllocationFailed { label })
        })
        .take(count)
        .collect::<Result<Vec<_>, _>>()?,
    };

    if resolved.len() != count {
        return Err(TopologyResolveError::PortCountMismatch {
            label,
            expected: count,
            actual: resolved.len(),
        });
    }

    Ok(resolved)
}

pub fn multiaddr_port(addr: &lb_libp2p::Multiaddr) -> Option<u16> {
    for protocol in addr {
        match protocol {
            lb_libp2p::Protocol::Udp(port) | lb_libp2p::Protocol::Tcp(port) => {
                return Some(port);
            }
            _ => {}
        }
    }
    None
}

use std::str::FromStr;

use nomos_core::sdp::{Locator, ServiceType};
use nomos_libp2p::Multiaddr;
use testing_framework_config::topology::configs::{
    blend::GeneralBlendConfig,
    consensus::{GeneralConsensusConfig, ProviderInfo},
};
use thiserror::Error;

use crate::host::Host;

#[derive(Debug, Error)]
pub enum ProviderBuildError {
    #[error("consensus configs are empty")]
    MissingConsensusConfigs,
    #[error("config length mismatch (hosts={hosts}, blend_configs={blend_configs})")]
    HostConfigLenMismatch { hosts: usize, blend_configs: usize },
    #[error("consensus notes length mismatch, blend_notes={blend_notes})")]
    NoteLenMismatch { blend_notes: usize },
    #[error("failed to parse multiaddr '{value}': {message}")]
    InvalidMultiaddr { value: String, message: String },
}

pub fn try_create_providers(
    hosts: &[Host],
    consensus_configs: &[GeneralConsensusConfig],
    blend_configs: &[GeneralBlendConfig],
) -> Result<Vec<ProviderInfo>, ProviderBuildError> {
    let first = consensus_configs
        .first()
        .ok_or(ProviderBuildError::MissingConsensusConfigs)?;

    validate_provider_inputs(hosts, first, blend_configs)?;

    let mut providers = Vec::with_capacity(blend_configs.len());
    providers.extend(build_blend_providers(hosts, first, blend_configs)?);
    Ok(providers)
}

pub fn create_providers(
    hosts: &[Host],
    consensus_configs: &[GeneralConsensusConfig],
    blend_configs: &[GeneralBlendConfig],
) -> Result<Vec<ProviderInfo>, ProviderBuildError> {
    try_create_providers(hosts, consensus_configs, blend_configs)
}

fn validate_provider_inputs(
    hosts: &[Host],
    first: &GeneralConsensusConfig,
    blend_configs: &[GeneralBlendConfig],
) -> Result<(), ProviderBuildError> {
    if hosts.len() != blend_configs.len() {
        return Err(ProviderBuildError::HostConfigLenMismatch {
            hosts: hosts.len(),
            blend_configs: blend_configs.len(),
        });
    }

    if first.blend_notes.len() < blend_configs.len() {
        return Err(ProviderBuildError::NoteLenMismatch {
            blend_notes: first.blend_notes.len(),
        });
    }

    Ok(())
}

fn build_blend_providers(
    hosts: &[Host],
    first: &GeneralConsensusConfig,
    blend_configs: &[GeneralBlendConfig],
) -> Result<Vec<ProviderInfo>, ProviderBuildError> {
    blend_configs
        .iter()
        .enumerate()
        .map(|(i, blend_conf)| {
            let locator = locator_for_host(hosts, i, hosts[i].blend_port)?;
            Ok(ProviderInfo {
                service_type: ServiceType::BlendNetwork,
                provider_sk: blend_conf.signer.clone(),
                zk_sk: blend_conf.secret_zk_key.clone(),
                locator,
                note: first.blend_notes[i].clone(),
            })
        })
        .collect()
}

fn locator_for_host(
    hosts: &[Host],
    index: usize,
    port: u16,
) -> Result<Locator, ProviderBuildError> {
    let value = format!("/ip4/{}/udp/{port}/quic-v1", hosts[index].ip);
    let locator =
        Multiaddr::from_str(&value).map_err(|source| ProviderBuildError::InvalidMultiaddr {
            value,
            message: source.to_string(),
        })?;
    Ok(Locator(locator))
}

use std::{
    env,
    num::{NonZero, NonZeroU64},
    str::FromStr as _,
    sync::Arc,
};

use chain_leader::LeaderConfig;
use cryptarchia_engine::EpochConfig;
use groth16::CompressedGroth16Proof;
use key_management_system_service::keys::{
    Ed25519Key, UnsecuredZkKey, ZkKey, ZkPublicKey, ZkSignature,
};
use nomos_core::{
    mantle::{
        MantleTx, Note, OpProof, Utxo,
        genesis_tx::GenesisTx,
        ledger::Tx as LedgerTx,
        ops::{
            Op,
            channel::{ChannelId, Ed25519PublicKey, MsgId, inscribe::InscriptionOp},
        },
    },
    sdp::{DeclarationMessage, Locator, ProviderId, ServiceParameters, ServiceType},
};
use nomos_node::{SignedMantleTx, Transaction as _};
use nomos_utils::math::NonNegativeF64;
use num_bigint::BigUint;

use super::wallet::{WalletAccount, WalletConfig};

#[derive(Debug, thiserror::Error)]
pub enum ConsensusConfigError {
    #[error("failed to build genesis inscription signer: {message}")]
    InscriptionSigner { message: String },
    #[error("failed to build genesis transaction: {message}")]
    GenesisTx { message: String },
    #[error("failed to build ledger config: {message}")]
    LedgerConfig { message: String },
    #[error("failed to sign genesis declarations: {message}")]
    DeclarationSignature { message: String },
}

#[derive(Clone)]
pub struct ConsensusParams {
    pub n_participants: usize,
    pub security_param: NonZero<u32>,
    pub active_slot_coeff: f64,
}

impl ConsensusParams {
    const DEFAULT_ACTIVE_SLOT_COEFF: f64 = 0.9;
    const CONSENSUS_ACTIVE_SLOT_COEFF_VAR: &str = "CONSENSUS_ACTIVE_SLOT_COEFF";

    #[must_use]
    pub fn default_for_participants(n_participants: usize) -> Self {
        let active_slot_coeff = env::var(Self::CONSENSUS_ACTIVE_SLOT_COEFF_VAR)
            .ok()
            .and_then(|raw| f64::from_str(&raw).ok())
            .filter(|value| (0.0..=1.0).contains(value) && *value > 0.0)
            .unwrap_or(Self::DEFAULT_ACTIVE_SLOT_COEFF);

        Self {
            n_participants,
            // by setting the slot coeff to 1, we also increase the probability of multiple blocks
            // (forks) being produced in the same slot (epoch). Setting the security
            // parameter to some value > 1 ensures nodes have some time to sync before
            // deciding on the longest chain.
            security_param: unsafe { NonZero::new_unchecked(10) },
            // a block should be produced (on average) every slot
            active_slot_coeff,
        }
    }
}

#[derive(Clone)]
pub struct ProviderInfo {
    pub service_type: ServiceType,
    pub provider_sk: Ed25519Key,
    pub zk_sk: ZkKey,
    pub locator: Locator,
    pub note: ServiceNote,
}

impl ProviderInfo {
    #[must_use]
    pub fn provider_id(&self) -> ProviderId {
        ProviderId(self.provider_sk.public_key())
    }

    #[must_use]
    pub fn zk_id(&self) -> ZkPublicKey {
        self.zk_sk.to_public_key()
    }
}

/// General consensus configuration for a chosen participant, that later could
/// be converted into a specific service or services configuration.
#[derive(Clone)]
pub struct GeneralConsensusConfig {
    pub leader_config: LeaderConfig,
    pub ledger_config: nomos_ledger::Config,
    pub genesis_tx: GenesisTx,
    pub utxos: Vec<Utxo>,
    pub blend_notes: Vec<ServiceNote>,
    pub da_notes: Vec<ServiceNote>,
    pub wallet_accounts: Vec<WalletAccount>,
}

#[derive(Clone)]
pub struct ServiceNote {
    pub pk: ZkPublicKey,
    pub sk: ZkKey,
    pub note: Note,
    pub output_index: usize,
}

fn create_genesis_tx(utxos: &[Utxo]) -> Result<GenesisTx, ConsensusConfigError> {
    // Create a genesis inscription op (similar to config.yaml)
    let inscription = InscriptionOp {
        channel_id: ChannelId::from([0; 32]),
        inscription: vec![103, 101, 110, 101, 115, 105, 115], // "genesis" in bytes
        parent: MsgId::root(),
        signer: Ed25519PublicKey::from_bytes(&[0; 32]).map_err(|err| {
            ConsensusConfigError::InscriptionSigner {
                message: err.to_string(),
            }
        })?,
    };

    // Create ledger transaction with the utxos as outputs
    let outputs: Vec<Note> = utxos.iter().map(|u| u.note).collect();
    let ledger_tx = LedgerTx::new(vec![], outputs);

    // Create the mantle transaction
    let mantle_tx = MantleTx {
        ops: vec![Op::ChannelInscribe(inscription)],
        ledger_tx,
        execution_gas_price: 0,
        storage_gas_price: 0,
    };
    let signed_mantle_tx = SignedMantleTx {
        mantle_tx,
        ops_proofs: vec![OpProof::NoProof],
        ledger_tx_proof: ZkSignature::new(CompressedGroth16Proof::from_bytes(&[0u8; 128])),
    };

    // Wrap in GenesisTx
    GenesisTx::from_tx(signed_mantle_tx).map_err(|err| ConsensusConfigError::GenesisTx {
        message: err.to_string(),
    })
}

fn build_ledger_config(
    consensus_params: &ConsensusParams,
) -> Result<nomos_ledger::Config, ConsensusConfigError> {
    Ok(nomos_ledger::Config {
        epoch_config: EpochConfig {
            epoch_stake_distribution_stabilization: unsafe { NonZero::new_unchecked(3) },
            epoch_period_nonce_buffer: unsafe { NonZero::new_unchecked(3) },
            epoch_period_nonce_stabilization: unsafe { NonZero::new_unchecked(4) },
        },
        consensus_config: cryptarchia_engine::Config {
            security_param: consensus_params.security_param,
            active_slot_coeff: consensus_params.active_slot_coeff,
        },
        sdp_config: nomos_ledger::mantle::sdp::Config {
            service_params: Arc::new(
                [
                    (
                        ServiceType::BlendNetwork,
                        ServiceParameters {
                            lock_period: 10,
                            inactivity_period: 20,
                            retention_period: 100,
                            timestamp: 0,
                            session_duration: 1000,
                        },
                    ),
                    (
                        ServiceType::DataAvailability,
                        ServiceParameters {
                            lock_period: 10,
                            inactivity_period: 20,
                            retention_period: 100,
                            timestamp: 0,
                            session_duration: 1000,
                        },
                    ),
                ]
                .into(),
            ),
            min_stake: nomos_core::sdp::MinStake {
                threshold: 1,
                timestamp: 0,
            },
            service_rewards_params: nomos_ledger::mantle::sdp::ServiceRewardsParameters {
                blend: nomos_ledger::mantle::sdp::rewards::blend::RewardsParameters {
                    rounds_per_session: unsafe { NonZeroU64::new_unchecked(10) },
                    message_frequency_per_round: NonNegativeF64::try_from(1.0).map_err(|_| {
                        ConsensusConfigError::LedgerConfig {
                            message: "message_frequency_per_round must be non-negative".to_owned(),
                        }
                    })?,
                    num_blend_layers: unsafe { NonZeroU64::new_unchecked(3) },
                    minimum_network_size: unsafe { NonZeroU64::new_unchecked(1) },
                },
            },
        },
    })
}

#[must_use]
pub fn create_consensus_configs(
    ids: &[[u8; 32]],
    consensus_params: &ConsensusParams,
    wallet: &WalletConfig,
) -> Result<Vec<GeneralConsensusConfig>, ConsensusConfigError> {
    let mut leader_keys = Vec::new();
    let mut blend_notes = Vec::new();
    let mut da_notes = Vec::new();

    let utxos = create_utxos_for_leader_and_services(
        ids,
        &mut leader_keys,
        &mut blend_notes,
        &mut da_notes,
    );
    let utxos = append_wallet_utxos(utxos, wallet);
    let genesis_tx = create_genesis_tx(&utxos)?;
    let ledger_config = build_ledger_config(consensus_params)?;

    Ok(leader_keys
        .into_iter()
        .map(|(pk, sk)| GeneralConsensusConfig {
            leader_config: LeaderConfig { pk, sk },
            ledger_config: ledger_config.clone(),
            genesis_tx: genesis_tx.clone(),
            utxos: utxos.clone(),
            da_notes: da_notes.clone(),
            blend_notes: blend_notes.clone(),
            wallet_accounts: wallet.accounts.clone(),
        })
        .collect())
}

fn create_utxos_for_leader_and_services(
    ids: &[[u8; 32]],
    leader_keys: &mut Vec<(ZkPublicKey, UnsecuredZkKey)>,
    blend_notes: &mut Vec<ServiceNote>,
    da_notes: &mut Vec<ServiceNote>,
) -> Vec<Utxo> {
    let mut utxos = Vec::new();

    // Assume output index which will be set by the ledger tx.
    let mut output_index = 0;

    // Create notes for leader, Blend and DA declarations.
    for &id in ids {
        output_index = push_leader_utxo(id, leader_keys, &mut utxos, output_index);
        output_index = push_service_note(b"da", id, da_notes, &mut utxos, output_index);
        output_index = push_service_note(b"bn", id, blend_notes, &mut utxos, output_index);
    }

    utxos
}

fn derive_key_material(prefix: &[u8], id_bytes: &[u8; 32]) -> [u8; 16] {
    let mut sk_data = [0; 16];
    let prefix_len = prefix.len();

    sk_data[..prefix_len].copy_from_slice(prefix);
    let remaining_len = 16 - prefix_len;
    sk_data[prefix_len..].copy_from_slice(&id_bytes[..remaining_len]);

    sk_data
}

fn push_leader_utxo(
    id: [u8; 32],
    leader_keys: &mut Vec<(ZkPublicKey, UnsecuredZkKey)>,
    utxos: &mut Vec<Utxo>,
    output_index: usize,
) -> usize {
    let sk_data = derive_key_material(b"ld", &id);
    let sk = UnsecuredZkKey::from(BigUint::from_bytes_le(&sk_data));
    let pk = sk.to_public_key();
    leader_keys.push((pk, sk));
    utxos.push(Utxo {
        note: Note::new(1_000, pk),
        tx_hash: BigUint::from(0u8).into(),
        output_index: 0,
    });
    output_index + 1
}

fn push_service_note(
    prefix: &[u8],
    id: [u8; 32],
    notes: &mut Vec<ServiceNote>,
    utxos: &mut Vec<Utxo>,
    output_index: usize,
) -> usize {
    let sk_data = derive_key_material(prefix, &id);
    let sk = ZkKey::from(BigUint::from_bytes_le(&sk_data));
    let pk = sk.to_public_key();
    let note = Note::new(1, pk);
    notes.push(ServiceNote {
        pk,
        sk,
        note,
        output_index,
    });
    utxos.push(Utxo {
        note,
        tx_hash: BigUint::from(0u8).into(),
        output_index: 0,
    });
    output_index + 1
}

fn append_wallet_utxos(mut utxos: Vec<Utxo>, wallet: &WalletConfig) -> Vec<Utxo> {
    for account in &wallet.accounts {
        utxos.push(Utxo {
            note: Note::new(account.value, account.public_key()),
            tx_hash: BigUint::from(0u8).into(),
            output_index: 0,
        });
    }

    utxos
}

#[must_use]
pub fn create_genesis_tx_with_declarations(
    ledger_tx: LedgerTx,
    providers: Vec<ProviderInfo>,
) -> Result<GenesisTx, ConsensusConfigError> {
    let inscription = build_genesis_inscription()?;
    let ledger_tx_hash = ledger_tx.hash();

    let ops = build_genesis_ops(inscription, ledger_tx_hash, &providers);
    let mantle_tx = MantleTx {
        ops,
        ledger_tx,
        execution_gas_price: 0,
        storage_gas_price: 0,
    };

    let ops_proofs = build_genesis_ops_proofs(mantle_tx.hash(), providers)?;
    build_genesis_tx(mantle_tx, ops_proofs)
}

fn build_genesis_inscription() -> Result<InscriptionOp, ConsensusConfigError> {
    Ok(InscriptionOp {
        channel_id: ChannelId::from([0; 32]),
        inscription: vec![103, 101, 110, 101, 115, 105, 115], // "genesis" in bytes
        parent: MsgId::root(),
        signer: Ed25519PublicKey::from_bytes(&[0; 32]).map_err(|err| {
            ConsensusConfigError::InscriptionSigner {
                message: err.to_string(),
            }
        })?,
    })
}

fn build_genesis_ops(
    inscription: InscriptionOp,
    ledger_tx_hash: nomos_core::mantle::TxHash,
    providers: &[ProviderInfo],
) -> Vec<Op> {
    let mut ops = Vec::with_capacity(1 + providers.len());
    ops.push(Op::ChannelInscribe(inscription));

    for provider in providers {
        let utxo = Utxo {
            tx_hash: ledger_tx_hash,
            output_index: provider.note.output_index,
            note: provider.note.note,
        };
        let declaration = DeclarationMessage {
            service_type: provider.service_type,
            locators: vec![provider.locator.clone()],
            provider_id: provider.provider_id(),
            zk_id: provider.zk_id(),
            locked_note_id: utxo.id(),
        };
        ops.push(Op::SDPDeclare(declaration));
    }

    ops
}

fn build_genesis_ops_proofs(
    mantle_tx_hash: nomos_core::mantle::TxHash,
    providers: Vec<ProviderInfo>,
) -> Result<Vec<OpProof>, ConsensusConfigError> {
    let mut ops_proofs = Vec::with_capacity(1 + providers.len());
    ops_proofs.push(OpProof::NoProof);

    for provider in providers {
        let zk_sig =
            ZkKey::multi_sign(&[provider.note.sk, provider.zk_sk], mantle_tx_hash.as_ref())
                .map_err(|err| ConsensusConfigError::DeclarationSignature {
                    message: format!("{err:?}"),
                })?;
        let ed25519_sig = provider
            .provider_sk
            .sign_payload(mantle_tx_hash.as_signing_bytes().as_ref());

        ops_proofs.push(OpProof::ZkAndEd25519Sigs {
            zk_sig,
            ed25519_sig,
        });
    }

    Ok(ops_proofs)
}

fn build_genesis_tx(
    mantle_tx: MantleTx,
    ops_proofs: Vec<OpProof>,
) -> Result<GenesisTx, ConsensusConfigError> {
    let signed_mantle_tx = SignedMantleTx {
        mantle_tx,
        ops_proofs,
        ledger_tx_proof: ZkSignature::new(CompressedGroth16Proof::from_bytes(&[0u8; 128])),
    };

    GenesisTx::from_tx(signed_mantle_tx).map_err(|err| ConsensusConfigError::GenesisTx {
        message: err.to_string(),
    })
}

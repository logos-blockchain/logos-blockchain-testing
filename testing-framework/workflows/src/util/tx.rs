use lb_core::mantle::{
    MantleTx, Op, OpProof, SignedMantleTx, Transaction as _,
    ledger::Tx as LedgerTx,
    ops::channel::{ChannelId, MsgId, inscribe::InscriptionOp},
};
use lb_key_management_system_service::keys::{Ed25519Key, ZkKey};
use testing_framework_core::scenario::DynError;

/// Builds a signed inscription transaction with deterministic payload for
/// testing.
pub fn create_inscription_transaction_with_id(id: ChannelId) -> Result<SignedMantleTx, DynError> {
    let signing_key = Ed25519Key::from_bytes(&[0u8; 32]);
    let signer = signing_key.public_key();

    let inscription_op = InscriptionOp {
        channel_id: id,
        inscription: format!("Test channel inscription {id:?}").into_bytes(),
        parent: MsgId::root(),
        signer,
    };

    let mantle_tx = MantleTx {
        ops: vec![Op::ChannelInscribe(inscription_op)],
        ledger_tx: LedgerTx::new(vec![], vec![]),
        storage_gas_price: 0,
        execution_gas_price: 0,
    };

    let tx_hash = mantle_tx.hash();
    let signature = signing_key.sign_payload(tx_hash.as_signing_bytes().as_ref());
    let zk_key = ZkKey::zero();
    tracing::debug!(channel = ?id, tx_hash = ?tx_hash, "building inscription transaction");

    let zk_signature = ZkKey::multi_sign(&[zk_key], tx_hash.as_ref())
        .map_err(|err| format!("zk signature generation failed: {err}"))?;

    SignedMantleTx::new(
        mantle_tx,
        vec![OpProof::Ed25519Sig(signature)],
        zk_signature,
    )
    .map_err(|err| format!("failed to build signed mantle transaction: {err}").into())
}

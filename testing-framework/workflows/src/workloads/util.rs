use std::{sync::Arc, time::Duration};

use nomos_core::{
    block::Block,
    mantle::{
        AuthenticatedMantleTx as _, SignedMantleTx, Transaction as MantleTx,
        ops::{Op, channel::MsgId},
    },
};
use rand::{seq::SliceRandom as _, thread_rng};
use testing_framework_core::scenario::{DynError, RunContext};
use tracing::debug;

const SUBMIT_RETRIES: usize = 5;
const SUBMIT_RETRY_DELAY: Duration = Duration::from_millis(500);

/// Scans a block and invokes the matcher for every operation until it returns
/// `Some(...)`. Returns `None` when no matching operation is found.
pub fn find_channel_op<F>(block: &Block<SignedMantleTx>, matcher: &mut F) -> Option<MsgId>
where
    F: FnMut(&Op) -> Option<MsgId>,
{
    debug!(
        txs = block.transactions().len(),
        "scanning block for channel op"
    );
    for tx in block.transactions() {
        for op in &tx.mantle_tx().ops {
            if let Some(msg_id) = matcher(op) {
                return Some(msg_id);
            }
        }
    }

    None
}

/// Submits a transaction to the cluster, fanning out across clients until one
/// succeeds.
pub async fn submit_transaction_via_cluster(
    ctx: &RunContext,
    tx: Arc<SignedMantleTx>,
) -> Result<(), DynError> {
    let tx_hash = tx.hash();
    debug!(
        ?tx_hash,
        "submitting transaction via cluster (validators first)"
    );

    let node_clients = ctx.node_clients();
    let mut validator_clients = node_clients.validator_clients();
    let mut executor_clients = node_clients.executor_clients();
    validator_clients.shuffle(&mut thread_rng());
    executor_clients.shuffle(&mut thread_rng());

    let clients = validator_clients.into_iter().chain(executor_clients);
    let mut clients: Vec<_> = clients.collect();
    let mut last_err = None;

    for attempt in 0..SUBMIT_RETRIES {
        clients.shuffle(&mut thread_rng());

        for client in &clients {
            let url = client.base_url().clone();
            debug!(?tx_hash, %url, attempt, "submitting transaction to client");
            match client
                .submit_transaction(&tx)
                .await
                .map_err(|err| -> DynError { err.into() })
            {
                Ok(()) => return Ok(()),
                Err(err) => {
                    debug!(?tx_hash, %url, attempt, "transaction submission failed");
                    last_err = Some(err);
                }
            }
        }

        tokio::time::sleep(SUBMIT_RETRY_DELAY).await;
    }

    Err(last_err.unwrap_or_else(|| "cluster client exhausted all nodes".into()))
}

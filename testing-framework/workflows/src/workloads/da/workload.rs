use std::{num::NonZeroU64, sync::Arc, time::Duration};

use async_trait::async_trait;
use executor_http_client::ExecutorHttpClient;
use futures::future::try_join_all;
use key_management_system_service::keys::{Ed25519Key, Ed25519PublicKey};
use nomos_core::{
    da::BlobId,
    mantle::{
        AuthenticatedMantleTx as _,
        ops::{
            Op,
            channel::{ChannelId, MsgId},
        },
    },
};
use rand::{RngCore as _, seq::SliceRandom as _, thread_rng};
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{BlockRecord, DynError, Expectation, RunContext, Workload as ScenarioWorkload},
};
use tokio::{sync::broadcast, time::sleep};

use super::expectation::DaWorkloadExpectation;
use crate::{
    util::tx,
    workloads::util::{find_channel_op, submit_transaction_via_cluster},
};

const TEST_KEY_BYTES: [u8; 32] = [0u8; 32];
const DEFAULT_BLOB_RATE_PER_BLOCK: u64 = 1;
const DEFAULT_CHANNEL_RATE_PER_BLOCK: u64 = 1;
const BLOB_CHUNK_OPTIONS: &[usize] = &[1, 2, 4, 8];
const PUBLISH_RETRIES: usize = 5;
const PUBLISH_RETRY_DELAY: Duration = Duration::from_secs(2);
const DEFAULT_HEADROOM_PERCENT: u64 = 20;

#[derive(Clone)]
pub struct Workload {
    blob_rate_per_block: NonZeroU64,
    channel_rate_per_block: NonZeroU64,
    headroom_percent: u64,
}

impl Default for Workload {
    fn default() -> Self {
        Self::with_rate(
            NonZeroU64::new(DEFAULT_BLOB_RATE_PER_BLOCK).expect("non-zero"),
            NonZeroU64::new(DEFAULT_CHANNEL_RATE_PER_BLOCK).expect("non-zero"),
            DEFAULT_HEADROOM_PERCENT,
        )
    }
}

impl Workload {
    /// Creates a workload that targets a blobs-per-block rate and applies a
    /// headroom factor when deriving the channel count.
    #[must_use]
    pub const fn with_rate(
        blob_rate_per_block: NonZeroU64,
        channel_rate_per_block: NonZeroU64,
        headroom_percent: u64,
    ) -> Self {
        Self {
            blob_rate_per_block,
            channel_rate_per_block,
            headroom_percent,
        }
    }

    #[must_use]
    pub const fn default_headroom_percent() -> u64 {
        DEFAULT_HEADROOM_PERCENT
    }
}

#[async_trait]
impl ScenarioWorkload for Workload {
    fn name(&self) -> &'static str {
        "channel_workload"
    }

    fn expectations(&self) -> Vec<Box<dyn Expectation>> {
        vec![Box::new(DaWorkloadExpectation::new(
            self.blob_rate_per_block,
            self.channel_rate_per_block,
            self.headroom_percent,
        ))]
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        let planned_channels = planned_channel_ids(planned_channel_count(
            self.channel_rate_per_block,
            self.headroom_percent,
        ));

        let expected_blobs = planned_blob_count(
            self.blob_rate_per_block,
            ctx.run_metrics().expected_consensus_blocks(),
            ctx.descriptors()
                .config()
                .consensus_params
                .security_param
                .get()
                .into(),
        );
        let per_channel_target =
            per_channel_blob_target(expected_blobs, planned_channels.len().max(1) as u64);

        tracing::info!(
            blob_rate_per_block = self.blob_rate_per_block.get(),
            channel_rate = self.channel_rate_per_block.get(),
            headroom_percent = self.headroom_percent,
            planned_channels = planned_channels.len(),
            expected_blobs,
            per_channel_target,
            "DA workload derived planned channels"
        );

        try_join_all(planned_channels.into_iter().map(|channel_id| {
            let ctx = ctx;
            async move {
                tracing::info!(channel_id = ?channel_id, blobs = per_channel_target, "DA workload starting channel flow");
                run_channel_flow(ctx, channel_id, per_channel_target).await?;
                tracing::info!(channel_id = ?channel_id, "DA workload finished channel flow");
                Ok::<(), DynError>(())
            }
        }))
        .await?;

        tracing::info!("DA workload completed all channel flows");
        Ok(())
    }
}

async fn run_channel_flow(
    ctx: &RunContext,
    channel_id: ChannelId,
    target_blobs: u64,
) -> Result<(), DynError> {
    tracing::debug!(channel_id = ?channel_id, "DA: submitting inscription tx");
    let inscription_tx = Arc::new(tx::create_inscription_transaction_with_id(channel_id));
    submit_transaction_via_cluster(ctx, Arc::clone(&inscription_tx)).await?;

    let mut receiver = ctx.block_feed().subscribe();
    let inscription_id = wait_for_inscription(&mut receiver, channel_id).await?;

    let mut parent_id = inscription_id;
    for idx in 0..target_blobs {
        let payload = random_blob_payload();
        let published_blob_id = publish_blob(ctx, channel_id, parent_id, payload).await?;
        let (next_parent, included_blob_id) =
            wait_for_blob_with_parent(&mut receiver, channel_id, parent_id).await?;
        parent_id = next_parent;

        tracing::debug!(
            channel_id = ?channel_id,
            blob_index = idx,
            published_blob_id = ?published_blob_id,
            included_blob_id = ?included_blob_id,
            "DA: blob published"
        );
    }
    Ok(())
}

async fn wait_for_inscription(
    receiver: &mut broadcast::Receiver<Arc<BlockRecord>>,
    channel_id: ChannelId,
) -> Result<MsgId, DynError> {
    wait_for_channel_op(receiver, move |op| {
        if let Op::ChannelInscribe(inscribe) = op
            && inscribe.channel_id == channel_id
        {
            Some(inscribe.id())
        } else {
            None
        }
    })
    .await
}

async fn wait_for_blob_with_parent(
    receiver: &mut broadcast::Receiver<Arc<BlockRecord>>,
    channel_id: ChannelId,
    parent_msg: MsgId,
) -> Result<(MsgId, BlobId), DynError> {
    loop {
        match receiver.recv().await {
            Ok(record) => {
                for tx in record.block.transactions() {
                    for op in &tx.mantle_tx().ops {
                        if let Op::ChannelBlob(blob_op) = op
                            && blob_op.channel == channel_id
                            && blob_op.parent == parent_msg
                        {
                            let msg_id = blob_op.id();
                            return Ok((msg_id, blob_op.blob));
                        }
                    }
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => {
                return Err("block feed closed while waiting for channel operations".into());
            }
        }
    }
}

async fn wait_for_channel_op<F>(
    receiver: &mut broadcast::Receiver<Arc<BlockRecord>>,
    mut matcher: F,
) -> Result<MsgId, DynError>
where
    F: FnMut(&Op) -> Option<MsgId>,
{
    loop {
        match receiver.recv().await {
            Ok(record) => {
                if let Some(msg_id) = find_channel_op(record.block.as_ref(), &mut matcher) {
                    tracing::debug!(?msg_id, "DA: matched channel operation");
                    return Ok(msg_id);
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => {}
            Err(broadcast::error::RecvError::Closed) => {
                return Err("block feed closed while waiting for channel operations".into());
            }
        }
    }
}

async fn publish_blob(
    ctx: &RunContext,
    channel_id: ChannelId,
    parent_msg: MsgId,
    data: Vec<u8>,
) -> Result<BlobId, DynError> {
    let executors = ctx.node_clients().executor_clients();
    if executors.is_empty() {
        return Err("da workload requires at least one executor".into());
    }

    let signer = test_signer();
    tracing::debug!(channel = ?channel_id, payload_bytes = data.len(), "DA: prepared blob payload");
    let client = ExecutorHttpClient::new(None);

    let mut candidates: Vec<&ApiClient> = executors.iter().collect();
    let mut last_err = None;
    for attempt in 1..=PUBLISH_RETRIES {
        candidates.shuffle(&mut thread_rng());
        for executor in &candidates {
            let executor_url = executor.base_url().clone();
            match client
                .publish_blob(executor_url, channel_id, parent_msg, signer, data.clone())
                .await
            {
                Ok(blob_id) => return Ok(blob_id),
                Err(err) => {
                    tracing::debug!(attempt, executor = %executor.base_url(), %err, "DA: publish_blob failed");
                    last_err = Some(err.into())
                }
            }
        }

        if attempt < PUBLISH_RETRIES {
            sleep(PUBLISH_RETRY_DELAY).await;
        }
    }

    Err(last_err.unwrap_or_else(|| "da workload could not publish blob".into()))
}

fn test_signer() -> Ed25519PublicKey {
    Ed25519Key::from_bytes(&TEST_KEY_BYTES).public_key()
}

fn random_blob_payload() -> Vec<u8> {
    let mut rng = thread_rng();
    // KZGRS encoder expects the polynomial degree to be a power of two, which
    // effectively constrains the blob chunk count.
    let chunks = *BLOB_CHUNK_OPTIONS
        .choose(&mut rng)
        .expect("non-empty chunk options");
    let mut data = vec![0u8; 31 * chunks];
    rng.fill_bytes(&mut data);
    data
}

pub fn planned_channel_ids(total: usize) -> Vec<ChannelId> {
    (0..total as u64)
        .map(deterministic_channel_id)
        .collect::<Vec<_>>()
}

fn deterministic_channel_id(index: u64) -> ChannelId {
    let mut bytes = [0u8; 32];
    bytes[..8].copy_from_slice(b"chn_wrkd");
    bytes[24..].copy_from_slice(&index.to_be_bytes());
    ChannelId::from(bytes)
}

#[must_use]
pub fn planned_channel_count(channel_rate_per_block: NonZeroU64, headroom_percent: u64) -> usize {
    let base = channel_rate_per_block.get() as usize;
    let extra = (base.saturating_mul(headroom_percent as usize) + 99) / 100;
    let total = base.saturating_add(extra);
    total.max(1)
}

#[must_use]
pub fn planned_blob_count(
    blob_rate_per_block: NonZeroU64,
    expected_consensus_blocks: u64,
    security_param: u64,
) -> u64 {
    let expected_blocks = expected_consensus_blocks.max(1);
    let security_param = security_param.max(1);
    let inclusion_blocks = (expected_blocks / security_param).max(1);
    blob_rate_per_block.get().saturating_mul(inclusion_blocks)
}

#[must_use]
pub fn per_channel_blob_target(total_blobs: u64, channel_count: u64) -> u64 {
    if channel_count == 0 {
        return total_blobs.max(1);
    }
    let per = (total_blobs + channel_count - 1) / channel_count;
    per.max(1)
}

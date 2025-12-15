use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU64,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use nomos_core::mantle::{
    AuthenticatedMantleTx as _,
    ops::{Op, channel::ChannelId},
};
use testing_framework_core::scenario::{BlockRecord, DynError, Expectation, RunContext};
use thiserror::Error;
use tokio::sync::broadcast;

use super::workload::{planned_channel_count, planned_channel_ids};

#[derive(Debug)]
pub struct DaWorkloadExpectation {
    blob_rate_per_block: NonZeroU64,
    channel_rate_per_block: NonZeroU64,
    headroom_percent: u64,
    capture_state: Option<CaptureState>,
}

#[derive(Debug)]
struct CaptureState {
    planned: Arc<HashSet<ChannelId>>,
    inscriptions: Arc<Mutex<HashSet<ChannelId>>>,
    blobs: Arc<Mutex<HashMap<ChannelId, u64>>>,
    run_blocks: Arc<AtomicU64>,
    run_duration: Duration,
}

const MIN_INSCRIPTION_INCLUSION_RATIO: f64 = 0.8;
const MIN_BLOB_INCLUSION_RATIO: f64 = 0.7;

#[derive(Debug, Error)]
enum DaExpectationError {
    #[error("da workload expectation not started")]
    NotCaptured,
    #[error(
        "missing inscriptions: observed={observed}/{planned} required={required} missing={missing:?}"
    )]
    MissingInscriptions {
        planned: usize,
        observed: usize,
        required: usize,
        missing: Vec<ChannelId>,
    },
    #[error(
        "missing blobs: observed_total_blobs={observed_total_blobs} expected_total_blobs={expected_total_blobs} required_blobs={required_blobs} channels_with_blobs={channels_with_blobs}/{planned_channels} missing_channels={missing:?}"
    )]
    MissingBlobs {
        expected_total_blobs: u64,
        observed_total_blobs: u64,
        required_blobs: u64,
        planned_channels: usize,
        channels_with_blobs: usize,
        missing: Vec<ChannelId>,
    },
}

impl DaWorkloadExpectation {
    /// Validates that inscriptions and blobs landed for the planned channels.
    pub const fn new(
        blob_rate_per_block: NonZeroU64,
        channel_rate_per_block: NonZeroU64,
        headroom_percent: u64,
    ) -> Self {
        Self {
            blob_rate_per_block,
            channel_rate_per_block,
            headroom_percent,
            capture_state: None,
        }
    }
}

#[async_trait]
impl Expectation for DaWorkloadExpectation {
    fn name(&self) -> &'static str {
        "da_workload_inclusions"
    }

    async fn start_capture(&mut self, ctx: &RunContext) -> Result<(), DynError> {
        if self.capture_state.is_some() {
            return Ok(());
        }

        let planned_ids = planned_channel_ids(planned_channel_count(
            self.channel_rate_per_block,
            self.headroom_percent,
        ));

        let run_duration = ctx.run_metrics().run_duration();

        tracing::info!(
            planned_channels = planned_ids.len(),
            blob_rate_per_block = self.blob_rate_per_block.get(),
            headroom_percent = self.headroom_percent,
            run_duration_secs = run_duration.as_secs(),
            "DA inclusion expectation starting capture"
        );

        let planned = Arc::new(planned_ids.iter().copied().collect::<HashSet<_>>());
        let inscriptions = Arc::new(Mutex::new(HashSet::new()));
        let blobs = Arc::new(Mutex::new(HashMap::new()));
        let run_blocks = Arc::new(AtomicU64::new(0));

        {
            let run_blocks = Arc::clone(&run_blocks);
            let mut receiver = ctx.block_feed().subscribe();
            tokio::spawn(async move {
                let timer = tokio::time::sleep(run_duration);
                tokio::pin!(timer);

                loop {
                    tokio::select! {
                        _ = &mut timer => break,
                        result = receiver.recv() => match result {
                            Ok(_) => {
                                run_blocks.fetch_add(1, Ordering::Relaxed);
                            }
                            Err(broadcast::error::RecvError::Lagged(_)) => {}
                            Err(broadcast::error::RecvError::Closed) => break,
                        }
                    }
                }
            });
        }

        let mut receiver = ctx.block_feed().subscribe();
        let planned_for_task = Arc::clone(&planned);
        let inscriptions_for_task = Arc::clone(&inscriptions);
        let blobs_for_task = Arc::clone(&blobs);

        tokio::spawn(async move {
            loop {
                match receiver.recv().await {
                    Ok(record) => capture_block(
                        record.as_ref(),
                        &planned_for_task,
                        &inscriptions_for_task,
                        &blobs_for_task,
                    ),
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        tracing::debug!(skipped, "DA expectation: receiver lagged");
                    }
                    Err(broadcast::error::RecvError::Closed) => {
                        tracing::debug!("DA expectation: block feed closed");
                        break;
                    }
                }
            }
        });

        self.capture_state = Some(CaptureState {
            planned,
            inscriptions,
            blobs,
            run_blocks,
            run_duration,
        });

        Ok(())
    }

    async fn evaluate(&mut self, _ctx: &RunContext) -> Result<(), DynError> {
        let state = self
            .capture_state
            .as_ref()
            .ok_or(DaExpectationError::NotCaptured)
            .map_err(DynError::from)?;

        let planned_total = state.planned.len();
        let missing_inscriptions = {
            let inscriptions = state
                .inscriptions
                .lock()
                .expect("inscription lock poisoned");
            missing_channels(&state.planned, &inscriptions)
        };
        let required_inscriptions =
            minimum_required(planned_total, MIN_INSCRIPTION_INCLUSION_RATIO);
        let observed_inscriptions = planned_total.saturating_sub(missing_inscriptions.len());
        if observed_inscriptions < required_inscriptions {
            tracing::warn!(
                planned = planned_total,
                missing = missing_inscriptions.len(),
                required = required_inscriptions,
                "DA expectation missing inscriptions"
            );
            return Err(DaExpectationError::MissingInscriptions {
                planned: planned_total,
                observed: observed_inscriptions,
                required: required_inscriptions,
                missing: missing_inscriptions,
            }
            .into());
        }

        let observed_total_blobs = {
            let blobs = state.blobs.lock().expect("blob lock poisoned");
            blobs.values().sum::<u64>()
        };

        let channels_with_blobs: HashSet<ChannelId> = {
            let blobs = state.blobs.lock().expect("blob lock poisoned");
            blobs
                .iter()
                .filter(|(_, count)| **count > 0)
                .map(|(channel, _)| *channel)
                .collect::<HashSet<_>>()
        };

        let observed_blocks = state.run_blocks.load(Ordering::Relaxed).max(1);
        let expected_total_blobs = self
            .blob_rate_per_block
            .get()
            .saturating_mul(observed_blocks);

        let missing_blob_channels = missing_channels(&state.planned, &channels_with_blobs);
        let required_blobs = minimum_required_u64(expected_total_blobs, MIN_BLOB_INCLUSION_RATIO);
        if observed_total_blobs < required_blobs {
            tracing::warn!(
                expected_total_blobs,
                observed_total_blobs,
                required_blobs,
                observed_blocks,
                run_duration_secs = state.run_duration.as_secs(),
                missing_blob_channels = missing_blob_channels.len(),
                "DA expectation missing blobs"
            );
            return Err(DaExpectationError::MissingBlobs {
                expected_total_blobs,
                observed_total_blobs,
                required_blobs,
                planned_channels: planned_total,
                channels_with_blobs: channels_with_blobs.len(),
                // Best-effort diagnostics: which planned channels never got any
                // blob included.
                missing: missing_blob_channels,
            }
            .into());
        }

        tracing::info!(
            planned_channels = planned_total,
            channels_with_blobs = channels_with_blobs.len(),
            inscriptions_observed = planned_total - missing_inscriptions.len(),
            observed_total_blobs,
            expected_total_blobs,
            required_blobs,
            observed_blocks,
            "DA inclusion expectation satisfied"
        );

        Ok(())
    }
}

fn capture_block(
    block: &BlockRecord,
    planned: &HashSet<ChannelId>,
    inscriptions: &Arc<Mutex<HashSet<ChannelId>>>,
    blobs: &Arc<Mutex<HashMap<ChannelId, u64>>>,
) {
    let mut new_inscriptions = Vec::new();
    let mut new_blobs = Vec::new();

    for tx in block.block.transactions() {
        for op in &tx.mantle_tx().ops {
            match op {
                Op::ChannelInscribe(inscribe) if planned.contains(&inscribe.channel_id) => {
                    new_inscriptions.push(inscribe.channel_id);
                }
                Op::ChannelBlob(blob) if planned.contains(&blob.channel) => {
                    new_blobs.push(blob.channel);
                }
                _ => {}
            }
        }
    }

    if !new_inscriptions.is_empty() {
        let mut guard = inscriptions.lock().expect("inscription lock poisoned");
        guard.extend(new_inscriptions);
        tracing::debug!(count = guard.len(), "DA expectation captured inscriptions");
    }

    if !new_blobs.is_empty() {
        let mut guard = blobs.lock().expect("blob lock poisoned");
        for channel in new_blobs {
            let entry = guard.entry(channel).or_insert(0);
            *entry += 1;
        }
        tracing::debug!(
            total_blobs = guard.values().sum::<u64>(),
            "DA expectation captured blobs"
        );
    }
}

fn missing_channels(planned: &HashSet<ChannelId>, observed: &HashSet<ChannelId>) -> Vec<ChannelId> {
    planned.difference(observed).copied().collect()
}

fn minimum_required(total: usize, ratio: f64) -> usize {
    ((total as f64) * ratio).ceil() as usize
}

fn minimum_required_u64(total: u64, ratio: f64) -> u64 {
    ((total as f64) * ratio).ceil() as u64
}

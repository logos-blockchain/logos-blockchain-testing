use std::time::Duration;

use async_trait::async_trait;
use nomos_core::header::HeaderId;
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{DynError, Expectation, RunContext},
};
use thiserror::Error;
use tokio::time::sleep;

#[derive(Clone, Copy, Debug)]
/// Checks that every node reaches near the highest observed height within an
/// allowance.
pub struct ConsensusLiveness {
    lag_allowance: u64,
}

impl Default for ConsensusLiveness {
    fn default() -> Self {
        Self {
            lag_allowance: LAG_ALLOWANCE,
        }
    }
}

const LAG_ALLOWANCE: u64 = 2;
const MIN_PROGRESS_BLOCKS: u64 = 5;
const REQUEST_RETRIES: usize = 15;
const REQUEST_RETRY_DELAY: Duration = Duration::from_secs(2);
const MAX_LAG_ALLOWANCE: u64 = 5;

#[async_trait]
impl Expectation for ConsensusLiveness {
    fn name(&self) -> &'static str {
        "consensus_liveness"
    }

    async fn evaluate(&mut self, ctx: &RunContext) -> Result<(), DynError> {
        Self::ensure_participants(ctx)?;
        let target_hint = Self::target_blocks(ctx);
        tracing::info!(target_hint, "consensus liveness: collecting samples");
        let check = Self::collect_results(ctx).await;
        (*self).report(target_hint, check)
    }
}

const fn consensus_target_blocks(ctx: &RunContext) -> u64 {
    ctx.expected_blocks()
}

#[derive(Debug, Error)]
enum ConsensusLivenessIssue {
    #[error("{node} height {height} below target {target}")]
    HeightBelowTarget {
        node: String,
        height: u64,
        target: u64,
    },
    #[error("{node} consensus_info failed: {source}")]
    RequestFailed {
        node: String,
        #[source]
        source: DynError,
    },
}

#[derive(Debug, Error)]
enum ConsensusLivenessError {
    #[error("consensus liveness requires at least one validator or executor")]
    MissingParticipants,
    #[error("consensus liveness violated (target={target}):\n{details}")]
    Violations {
        target: u64,
        #[source]
        details: ViolationIssues,
    },
}

#[derive(Debug, Error)]
#[error("{message}")]
struct ViolationIssues {
    issues: Vec<ConsensusLivenessIssue>,
    message: String,
}

impl ConsensusLiveness {
    const fn target_blocks(ctx: &RunContext) -> u64 {
        consensus_target_blocks(ctx)
    }

    fn ensure_participants(ctx: &RunContext) -> Result<(), DynError> {
        if ctx.node_clients().all_clients().count() == 0 {
            Err(Box::new(ConsensusLivenessError::MissingParticipants))
        } else {
            Ok(())
        }
    }

    async fn collect_results(ctx: &RunContext) -> LivenessCheck {
        let clients: Vec<_> = ctx.node_clients().all_clients().collect();
        let mut samples = Vec::with_capacity(clients.len());
        let mut issues = Vec::new();

        for (idx, client) in clients.iter().enumerate() {
            let node = format!("node-{idx}");

            for attempt in 0..REQUEST_RETRIES {
                match Self::fetch_cluster_info(client).await {
                    Ok(sample) => {
                        tracing::debug!(
                            node = %node,
                            height = sample.height,
                            tip = ?sample.tip,
                            attempt,
                            "consensus_info collected"
                        );
                        samples.push(NodeSample {
                            label: node.clone(),
                            height: sample.height,
                            tip: sample.tip,
                        });

                        break;
                    }

                    Err(err) if attempt + 1 == REQUEST_RETRIES => {
                        tracing::warn!(node = %node, %err, "consensus_info failed after retries");

                        issues.push(ConsensusLivenessIssue::RequestFailed {
                            node: node.clone(),
                            source: err,
                        });
                    }

                    Err(_) => sleep(REQUEST_RETRY_DELAY).await,
                }
            }
        }

        LivenessCheck { samples, issues }
    }

    async fn fetch_cluster_info(client: &ApiClient) -> Result<ConsensusInfoSample, DynError> {
        client
            .consensus_info()
            .await
            .map(|info| ConsensusInfoSample {
                height: info.height,
                tip: info.tip,
            })
            .map_err(|err| -> DynError { err.into() })
    }

    #[must_use]
    /// Adjusts how many blocks behind the leader a node may be before failing.
    pub const fn with_lag_allowance(mut self, lag_allowance: u64) -> Self {
        self.lag_allowance = lag_allowance;
        self
    }

    fn effective_lag_allowance(&self, target: u64) -> u64 {
        (target / 10).clamp(self.lag_allowance, MAX_LAG_ALLOWANCE)
    }

    fn report(self, target_hint: u64, mut check: LivenessCheck) -> Result<(), DynError> {
        if check.samples.is_empty() {
            return Err(Box::new(ConsensusLivenessError::MissingParticipants));
        }

        let max_height = check
            .samples
            .iter()
            .map(|sample| sample.height)
            .max()
            .unwrap_or(0);

        let mut target = target_hint;
        if target == 0 || target > max_height {
            target = max_height;
        }
        let lag_allowance = self.effective_lag_allowance(target);

        if max_height < MIN_PROGRESS_BLOCKS {
            check
                .issues
                .push(ConsensusLivenessIssue::HeightBelowTarget {
                    node: "network".to_owned(),
                    height: max_height,
                    target: MIN_PROGRESS_BLOCKS,
                });
        }

        for sample in &check.samples {
            if sample.height + lag_allowance < target {
                check
                    .issues
                    .push(ConsensusLivenessIssue::HeightBelowTarget {
                        node: sample.label.clone(),
                        height: sample.height,
                        target,
                    });
            }
        }

        if check.issues.is_empty() {
            let observed_heights: Vec<_> = check.samples.iter().map(|s| s.height).collect();
            let observed_tips: Vec<_> = check.samples.iter().map(|s| s.tip).collect();

            tracing::info!(
                target,
                samples = check.samples.len(),
                heights = ?observed_heights,
                tips = ?observed_tips,
                "consensus liveness expectation satisfied"
            );
            Ok(())
        } else {
            for issue in &check.issues {
                tracing::warn!(?issue, "consensus liveness issue");
            }

            Err(Box::new(ConsensusLivenessError::Violations {
                target,
                details: check.issues.into(),
            }))
        }
    }
}

struct ConsensusInfoSample {
    height: u64,
    tip: HeaderId,
}

struct NodeSample {
    label: String,
    height: u64,
    tip: HeaderId,
}

struct LivenessCheck {
    samples: Vec<NodeSample>,
    issues: Vec<ConsensusLivenessIssue>,
}

impl From<Vec<ConsensusLivenessIssue>> for ViolationIssues {
    fn from(issues: Vec<ConsensusLivenessIssue>) -> Self {
        let mut message = String::new();
        for issue in &issues {
            if !message.is_empty() {
                message.push('\n');
            }
            message.push_str("- ");
            message.push_str(&issue.to_string());
        }
        Self { issues, message }
    }
}

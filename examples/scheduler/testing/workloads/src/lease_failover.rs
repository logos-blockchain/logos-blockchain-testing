use std::{collections::HashSet, time::Duration};

use async_trait::async_trait;
use scheduler_runtime_ext::SchedulerEnv;
use serde::{Deserialize, Serialize};
use testing_framework_core::scenario::{DynError, RunContext, Workload};
use tokio::time::{Instant, sleep};
use tracing::info;

#[derive(Clone)]
pub struct SchedulerLeaseFailoverWorkload {
    operations: usize,
    lease_ttl: Duration,
    rate_per_sec: Option<usize>,
    payload_prefix: String,
}

#[derive(Serialize)]
struct EnqueueRequest {
    payload: String,
}

#[derive(Deserialize)]
struct EnqueueResponse {
    id: u64,
}

#[derive(Serialize)]
struct ClaimRequest {
    worker_id: String,
    max_jobs: usize,
}

#[derive(Deserialize)]
struct ClaimedJob {
    id: u64,
}

#[derive(Deserialize)]
struct ClaimResponse {
    jobs: Vec<ClaimedJob>,
}

#[derive(Serialize)]
struct AckRequest {
    worker_id: String,
    job_id: u64,
}

#[derive(Deserialize)]
struct OperationResponse {
    ok: bool,
}

impl SchedulerLeaseFailoverWorkload {
    #[must_use]
    pub fn new() -> Self {
        Self {
            operations: 100,
            lease_ttl: Duration::from_secs(3),
            rate_per_sec: Some(25),
            payload_prefix: "scheduler-job".to_owned(),
        }
    }

    #[must_use]
    pub const fn operations(mut self, value: usize) -> Self {
        self.operations = value;
        self
    }

    #[must_use]
    pub const fn lease_ttl(mut self, value: Duration) -> Self {
        self.lease_ttl = value;
        self
    }

    #[must_use]
    pub const fn rate_per_sec(mut self, value: usize) -> Self {
        self.rate_per_sec = Some(value);
        self
    }
}

impl Default for SchedulerLeaseFailoverWorkload {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Workload<SchedulerEnv> for SchedulerLeaseFailoverWorkload {
    fn name(&self) -> &str {
        "scheduler_lease_failover_workload"
    }

    async fn start(&self, ctx: &RunContext<SchedulerEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        let Some(node_a) = clients.first() else {
            return Err("no scheduler node clients available".into());
        };
        let node_b = clients.get(1).unwrap_or(node_a);

        let interval = self.rate_per_sec.and_then(compute_interval);
        let mut enqueued_ids = Vec::with_capacity(self.operations);

        info!(
            operations = self.operations,
            "scheduler failover: enqueue phase"
        );
        for index in 0..self.operations {
            let response: EnqueueResponse = node_a
                .post(
                    "/jobs/enqueue",
                    &EnqueueRequest {
                        payload: format!("{}-{index}", self.payload_prefix),
                    },
                )
                .await?;
            enqueued_ids.push(response.id);
            if let Some(delay) = interval {
                sleep(delay).await;
            }
        }

        info!("scheduler failover: worker-a claim without ack");
        let first_claim: ClaimResponse = node_a
            .post(
                "/jobs/claim",
                &ClaimRequest {
                    worker_id: "worker-a".to_owned(),
                    max_jobs: self.operations,
                },
            )
            .await?;

        if first_claim.jobs.len() != self.operations {
            return Err(format!(
                "worker-a claimed {} jobs, expected {}",
                first_claim.jobs.len(),
                self.operations
            )
            .into());
        }

        sleep(self.lease_ttl + Duration::from_millis(500)).await;

        info!("scheduler failover: worker-b reclaim and ack");
        let mut pending_ids: HashSet<u64> = enqueued_ids.into_iter().collect();
        let reclaim_deadline = Instant::now() + Duration::from_secs(20);

        while !pending_ids.is_empty() && Instant::now() < reclaim_deadline {
            let claim: ClaimResponse = node_b
                .post(
                    "/jobs/claim",
                    &ClaimRequest {
                        worker_id: "worker-b".to_owned(),
                        max_jobs: pending_ids.len(),
                    },
                )
                .await?;

            if claim.jobs.is_empty() {
                sleep(Duration::from_millis(200)).await;
                continue;
            }

            for job in claim.jobs {
                if !pending_ids.remove(&job.id) {
                    return Err(format!("unexpected reclaimed job id {}", job.id).into());
                }

                let ack: OperationResponse = node_b
                    .post(
                        "/jobs/ack",
                        &AckRequest {
                            worker_id: "worker-b".to_owned(),
                            job_id: job.id,
                        },
                    )
                    .await?;

                if !ack.ok {
                    return Err(format!("failed to ack reclaimed job {}", job.id).into());
                }
            }
        }

        if !pending_ids.is_empty() {
            return Err(
                format!("scheduler failover left {} unacked jobs", pending_ids.len()).into(),
            );
        }

        Ok(())
    }
}

fn compute_interval(rate_per_sec: usize) -> Option<Duration> {
    if rate_per_sec == 0 {
        return None;
    }

    Some(Duration::from_millis((1000 / rate_per_sec as u64).max(1)))
}

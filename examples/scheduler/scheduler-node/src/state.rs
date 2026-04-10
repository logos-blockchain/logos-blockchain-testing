use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct Revision {
    pub version: u64,
    pub origin: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct JobRecord {
    pub id: u64,
    pub payload: String,
    pub attempt: u32,
    pub owner: Option<String>,
    pub lease_expires_at_ms: Option<u64>,
    pub done: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    pub node_id: u64,
    pub revision: Revision,
    pub next_id: u64,
    pub jobs: BTreeMap<u64, JobRecord>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct StateView {
    pub revision: Revision,
    pub next_id: u64,
    pub pending: usize,
    pub leased: usize,
    pub done: usize,
}

#[derive(Clone, Debug, Serialize)]
pub struct ClaimResult {
    pub jobs: Vec<JobRecord>,
}

#[derive(Debug, Default)]
struct Data {
    revision: Revision,
    next_id: u64,
    jobs: BTreeMap<u64, JobRecord>,
}

#[derive(Clone)]
pub struct SchedulerState {
    node_id: u64,
    ready: Arc<RwLock<bool>>,
    lease_ttl_ms: u64,
    data: Arc<RwLock<Data>>,
}

impl SchedulerState {
    pub fn new(node_id: u64, lease_ttl_ms: u64) -> Self {
        Self {
            node_id,
            ready: Arc::new(RwLock::new(false)),
            lease_ttl_ms,
            data: Arc::new(RwLock::new(Data {
                next_id: 1,
                ..Data::default()
            })),
        }
    }

    pub const fn node_id(&self) -> u64 {
        self.node_id
    }

    pub async fn set_ready(&self, value: bool) {
        *self.ready.write().await = value;
    }

    pub async fn is_ready(&self) -> bool {
        *self.ready.read().await
    }

    pub async fn enqueue(&self, payload: String) -> u64 {
        let mut data = self.data.write().await;
        reap_expired_leases(&mut data.jobs);

        let id = data.next_id;
        data.next_id = data.next_id.saturating_add(1);

        data.jobs.insert(
            id,
            JobRecord {
                id,
                payload,
                attempt: 0,
                owner: None,
                lease_expires_at_ms: None,
                done: false,
            },
        );

        bump_revision(&mut data.revision, self.node_id);
        id
    }

    pub async fn claim(&self, worker_id: String, max_jobs: usize) -> ClaimResult {
        let mut data = self.data.write().await;
        reap_expired_leases(&mut data.jobs);

        let now = unix_ms();
        let mut claimed = Vec::new();

        for job in data.jobs.values_mut() {
            if claimed.len() >= max_jobs {
                break;
            }
            if job.done || job.owner.is_some() {
                continue;
            }

            job.attempt = job.attempt.saturating_add(1);
            job.owner = Some(worker_id.clone());
            job.lease_expires_at_ms = Some(now.saturating_add(self.lease_ttl_ms));
            claimed.push(job.clone());
        }

        if !claimed.is_empty() {
            bump_revision(&mut data.revision, self.node_id);
        }

        ClaimResult { jobs: claimed }
    }

    pub async fn heartbeat(&self, worker_id: &str, job_id: u64) -> bool {
        let mut data = self.data.write().await;
        reap_expired_leases(&mut data.jobs);

        let Some(job) = data.jobs.get_mut(&job_id) else {
            return false;
        };

        if job.done || job.owner.as_deref() != Some(worker_id) {
            return false;
        }

        job.lease_expires_at_ms = Some(unix_ms().saturating_add(self.lease_ttl_ms));
        bump_revision(&mut data.revision, self.node_id);
        true
    }

    pub async fn ack(&self, worker_id: &str, job_id: u64) -> bool {
        let mut data = self.data.write().await;
        reap_expired_leases(&mut data.jobs);

        let Some(job) = data.jobs.get_mut(&job_id) else {
            return false;
        };

        if job.done || job.owner.as_deref() != Some(worker_id) {
            return false;
        }

        job.done = true;
        job.owner = None;
        job.lease_expires_at_ms = None;
        bump_revision(&mut data.revision, self.node_id);
        true
    }

    pub async fn state_view(&self) -> StateView {
        let data = self.data.read().await;
        let mut pending = 0;
        let mut leased = 0;
        let mut done = 0;

        for job in data.jobs.values() {
            if job.done {
                done += 1;
            } else if job.owner.is_some() {
                leased += 1;
            } else {
                pending += 1;
            }
        }

        StateView {
            revision: data.revision,
            next_id: data.next_id,
            pending,
            leased,
            done,
        }
    }

    pub async fn merge_snapshot(&self, snapshot: Snapshot) {
        let mut data = self.data.write().await;
        if is_newer_revision(snapshot.revision, data.revision) {
            data.revision = snapshot.revision;
            data.next_id = snapshot.next_id;
            data.jobs = snapshot.jobs;
        }
    }

    pub async fn snapshot(&self) -> Snapshot {
        let data = self.data.read().await;
        Snapshot {
            node_id: self.node_id,
            revision: data.revision,
            next_id: data.next_id,
            jobs: data.jobs.clone(),
        }
    }
}

fn reap_expired_leases(jobs: &mut BTreeMap<u64, JobRecord>) {
    let now = unix_ms();
    for job in jobs.values_mut() {
        if job.done {
            continue;
        }

        if let Some(expiry) = job.lease_expires_at_ms
            && expiry <= now
        {
            job.owner = None;
            job.lease_expires_at_ms = None;
        }
    }
}

fn bump_revision(revision: &mut Revision, node_id: u64) {
    revision.version = revision.version.saturating_add(1);
    revision.origin = node_id;
}

fn is_newer_revision(candidate: Revision, existing: Revision) -> bool {
    (candidate.version, candidate.origin) > (existing.version, existing.origin)
}

fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

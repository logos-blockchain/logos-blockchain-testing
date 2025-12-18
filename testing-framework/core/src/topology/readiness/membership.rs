use nomos_core::sdp::SessionNumber;
use nomos_da_network_service::MembershipResponse;
use reqwest::{Client, Url};
use thiserror::Error;

use super::ReadinessCheck;
use crate::topology::deployment::Topology;

#[derive(Debug, Error)]
pub enum MembershipError {
    #[error("failed to join url {base} with path {path}: {message}")]
    JoinUrl {
        base: Url,
        path: &'static str,
        message: String,
    },
    #[error(transparent)]
    Request(#[from] reqwest::Error),
}

#[derive(Debug)]
pub struct NodeMembershipStatus {
    label: String,
    result: Result<MembershipResponse, MembershipError>,
}

pub struct MembershipReadiness<'a> {
    pub(crate) topology: &'a Topology,
    pub(crate) session: SessionNumber,
    pub(crate) labels: &'a [String],
    pub(crate) expect_non_empty: bool,
}

#[async_trait::async_trait]
impl<'a> ReadinessCheck<'a> for MembershipReadiness<'a> {
    type Data = Vec<NodeMembershipStatus>;

    async fn collect(&'a self) -> Self::Data {
        let validator_futures = self
            .topology
            .validators
            .iter()
            .enumerate()
            .map(|(idx, node)| {
                let label = self
                    .labels
                    .get(idx)
                    .cloned()
                    .unwrap_or_else(|| format!("validator#{idx}"));
                async move {
                    let result = node
                        .api()
                        .da_get_membership(&self.session)
                        .await
                        .map_err(MembershipError::from);
                    NodeMembershipStatus { label, result }
                }
            });
        let offset = self.topology.validators.len();
        let executor_futures = self
            .topology
            .executors
            .iter()
            .enumerate()
            .map(|(idx, node)| {
                let global_idx = offset + idx;
                let label = self
                    .labels
                    .get(global_idx)
                    .cloned()
                    .unwrap_or_else(|| format!("executor#{idx}"));
                async move {
                    let result = node
                        .api()
                        .da_get_membership(&self.session)
                        .await
                        .map_err(MembershipError::from);
                    NodeMembershipStatus { label, result }
                }
            });

        let (validator_statuses, executor_statuses) = tokio::join!(
            futures::future::join_all(validator_futures),
            futures::future::join_all(executor_futures)
        );
        validator_statuses
            .into_iter()
            .chain(executor_statuses)
            .collect()
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        data.iter()
            .all(|entry| is_membership_ready(entry.result.as_ref(), self.expect_non_empty))
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let description = if self.expect_non_empty {
            "non-empty assignations"
        } else {
            "empty assignations"
        };
        let summary = build_membership_status_summary(&data, description, self.expect_non_empty);
        format!("timed out waiting for DA membership readiness ({description}): {summary}")
    }
}

pub struct HttpMembershipReadiness<'a> {
    pub(crate) client: &'a Client,
    pub(crate) endpoints: &'a [Url],
    pub(crate) session: SessionNumber,
    pub(crate) labels: &'a [String],
    pub(crate) expect_non_empty: bool,
}

#[async_trait::async_trait]
impl<'a> ReadinessCheck<'a> for HttpMembershipReadiness<'a> {
    type Data = Vec<NodeMembershipStatus>;

    async fn collect(&'a self) -> Self::Data {
        let futures = self.endpoints.iter().enumerate().map(|(idx, endpoint)| {
            let label = self
                .labels
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("endpoint#{idx}"));
            async move {
                let result = try_fetch_membership(self.client, endpoint, self.session).await;
                NodeMembershipStatus { label, result }
            }
        });
        futures::future::join_all(futures).await
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        data.iter()
            .all(|entry| is_membership_ready(entry.result.as_ref(), self.expect_non_empty))
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let description = if self.expect_non_empty {
            "non-empty assignations"
        } else {
            "empty assignations"
        };
        let summary = build_membership_status_summary(&data, description, self.expect_non_empty);
        format!("timed out waiting for DA membership readiness ({description}): {summary}")
    }
}

pub async fn try_fetch_membership(
    client: &Client,
    base: &Url,
    session: SessionNumber,
) -> Result<MembershipResponse, MembershipError> {
    let path = nomos_http_api_common::paths::DA_GET_MEMBERSHIP.trim_start_matches('/');
    let url = base.join(path).map_err(|source| MembershipError::JoinUrl {
        base: base.clone(),
        path: nomos_http_api_common::paths::DA_GET_MEMBERSHIP,
        message: source.to_string(),
    })?;
    client
        .post(url)
        .json(&session)
        .send()
        .await
        .map_err(MembershipError::Request)?
        .error_for_status()
        .map_err(MembershipError::Request)?
        .json()
        .await
        .map_err(MembershipError::Request)
}

#[deprecated(note = "use try_fetch_membership to avoid panics and preserve error details")]
pub async fn fetch_membership(
    client: &Client,
    base: &Url,
    session: SessionNumber,
) -> Result<MembershipResponse, reqwest::Error> {
    try_fetch_membership(client, base, session)
        .await
        .map_err(|err| match err {
            MembershipError::Request(source) => source,
            MembershipError::JoinUrl {
                base,
                path,
                message,
            } => {
                panic!("failed to join url {base} with path {path}: {message}")
            }
        })
}

fn is_membership_ready(
    result: Result<&MembershipResponse, &MembershipError>,
    expect_non_empty: bool,
) -> bool {
    match result {
        Ok(resp) => {
            let is_non_empty = !resp.assignations.is_empty();
            if expect_non_empty {
                is_non_empty
            } else {
                !is_non_empty
            }
        }
        Err(_) => false,
    }
}

fn build_membership_status_summary(
    statuses: &[NodeMembershipStatus],
    description: &str,
    expect_non_empty: bool,
) -> String {
    statuses
        .iter()
        .map(|entry| match entry.result.as_ref() {
            Ok(resp) => {
                let ready = is_membership_ready(Ok(resp), expect_non_empty);
                let status = if ready { "ready" } else { "waiting" };
                format!("{}: status={status}, expected {description}", entry.label)
            }
            Err(err) => format!("{}: error={err}, expected {description}", entry.label),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

#[deprecated(note = "use ReadinessCheck timeout_message for richer per-node errors")]
pub fn build_membership_summary(labels: &[String], statuses: &[bool], description: &str) -> String {
    statuses
        .iter()
        .zip(labels.iter())
        .map(|(ready, label)| {
            let status = if *ready { "ready" } else { "waiting" };
            format!("{label}: status={status}, expected {description}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

use nomos_core::sdp::SessionNumber;
use nomos_da_network_service::MembershipResponse;
use reqwest::{Client, Url};
use thiserror::Error;

use super::ReadinessCheck;
use crate::{nodes::ApiClientError, topology::deployment::Topology};

#[derive(Debug, Error)]
pub enum MembershipError {
    #[error("failed to join url {base} with path {path}: {message}")]
    JoinUrl {
        base: Url,
        path: &'static str,
        message: String,
    },
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    ApiClient(#[from] ApiClientError),
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
        collect_node_statuses(self).await
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

async fn collect_node_statuses(readiness: &MembershipReadiness<'_>) -> Vec<NodeMembershipStatus> {
    let node_futures = readiness
        .topology
        .nodes
        .iter()
        .enumerate()
        .map(|(idx, node)| {
            let label = readiness
                .labels
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("node#{idx}"));
            async move {
                let result = node
                    .api()
                    .da_get_membership_checked(&readiness.session)
                    .await
                    .map_err(MembershipError::from);
                NodeMembershipStatus { label, result }
            }
        });

    futures::future::join_all(node_futures).await
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
        .map_err(MembershipError::Http)?
        .error_for_status()
        .map_err(MembershipError::Http)?
        .json()
        .await
        .map_err(MembershipError::Http)
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

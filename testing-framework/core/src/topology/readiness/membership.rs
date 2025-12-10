use nomos_core::sdp::SessionNumber;
use nomos_da_network_service::MembershipResponse;
use reqwest::{Client, Url};

use super::ReadinessCheck;
use crate::topology::deployment::Topology;

pub struct MembershipReadiness<'a> {
    pub(crate) topology: &'a Topology,
    pub(crate) session: SessionNumber,
    pub(crate) labels: &'a [String],
    pub(crate) expect_non_empty: bool,
}

#[async_trait::async_trait]
impl<'a> ReadinessCheck<'a> for MembershipReadiness<'a> {
    type Data = Vec<Result<MembershipResponse, reqwest::Error>>;

    async fn collect(&'a self) -> Self::Data {
        let (validator_responses, executor_responses) = tokio::join!(
            futures::future::join_all(
                self.topology
                    .validators
                    .iter()
                    .map(|node| node.da_get_membership(self.session)),
            ),
            futures::future::join_all(
                self.topology
                    .executors
                    .iter()
                    .map(|node| node.da_get_membership(self.session)),
            )
        );

        validator_responses
            .into_iter()
            .chain(executor_responses)
            .collect()
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        self.assignation_statuses(data)
            .into_iter()
            .all(|ready| ready)
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let statuses = self.assignation_statuses(&data);
        let description = if self.expect_non_empty {
            "non-empty assignations"
        } else {
            "empty assignations"
        };
        let summary = build_membership_summary(self.labels, &statuses, description);
        format!("timed out waiting for DA membership readiness ({description}): {summary}")
    }
}

impl MembershipReadiness<'_> {
    fn assignation_statuses(
        &self,
        responses: &[Result<MembershipResponse, reqwest::Error>],
    ) -> Vec<bool> {
        responses
            .iter()
            .map(|res| {
                res.as_ref()
                    .map(|resp| {
                        let is_non_empty = !resp.assignations.is_empty();
                        if self.expect_non_empty {
                            is_non_empty
                        } else {
                            !is_non_empty
                        }
                    })
                    .unwrap_or(false)
            })
            .collect()
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
    type Data = Vec<Result<MembershipResponse, reqwest::Error>>;

    async fn collect(&'a self) -> Self::Data {
        let futures = self
            .endpoints
            .iter()
            .map(|endpoint| fetch_membership(self.client, endpoint, self.session));
        futures::future::join_all(futures).await
    }

    fn is_ready(&self, data: &Self::Data) -> bool {
        assignation_statuses(data, self.expect_non_empty)
            .into_iter()
            .all(|ready| ready)
    }

    fn timeout_message(&self, data: Self::Data) -> String {
        let statuses = assignation_statuses(&data, self.expect_non_empty);
        let description = if self.expect_non_empty {
            "non-empty assignations"
        } else {
            "empty assignations"
        };
        let summary = build_membership_summary(self.labels, &statuses, description);
        format!("timed out waiting for DA membership readiness ({description}): {summary}")
    }
}

pub async fn fetch_membership(
    client: &Client,
    base: &Url,
    session: SessionNumber,
) -> Result<MembershipResponse, reqwest::Error> {
    let url = base
        .join(nomos_http_api_common::paths::DA_GET_MEMBERSHIP.trim_start_matches('/'))
        .unwrap_or_else(|err| {
            panic!(
                "failed to join url {base} with path {}: {err}",
                nomos_http_api_common::paths::DA_GET_MEMBERSHIP
            )
        });
    client
        .post(url)
        .json(&session)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await
}

pub fn assignation_statuses(
    responses: &[Result<MembershipResponse, reqwest::Error>],
    expect_non_empty: bool,
) -> Vec<bool> {
    responses
        .iter()
        .map(|res| {
            res.as_ref()
                .map(|resp| {
                    let is_non_empty = !resp.assignations.is_empty();
                    if expect_non_empty {
                        is_non_empty
                    } else {
                        !is_non_empty
                    }
                })
                .unwrap_or(false)
        })
        .collect()
}

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

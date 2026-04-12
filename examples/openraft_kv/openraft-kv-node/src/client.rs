use std::{collections::BTreeSet, time::Duration};

use reqwest::Url;
use serde::{Serialize, de::DeserializeOwned};

use crate::types::{
    AddLearnerRequest, AddLearnerResult, ChangeMembershipRequest, ChangeMembershipResult,
    InitResult, OpenRaftKvReadRequest, OpenRaftKvReadResponse, OpenRaftKvState,
    OpenRaftKvWriteRequest, OpenRaftKvWriteResponse,
};

/// Small HTTP client for the OpenRaft example node and its admin endpoints.
#[derive(Clone)]
pub struct OpenRaftKvClient {
    base_url: Url,
    client: reqwest::Client,
}

impl OpenRaftKvClient {
    /// Builds a client for one node base URL.
    #[must_use]
    pub fn new(base_url: Url) -> Self {
        Self {
            base_url,
            client: reqwest::Client::builder()
                .timeout(Duration::from_secs(2))
                .connect_timeout(Duration::from_secs(2))
                .build()
                .expect("openraft kv client timeout configuration is valid"),
        }
    }

    /// Fetches the node's current Raft and application state.
    pub async fn state(&self) -> anyhow::Result<OpenRaftKvState> {
        self.get("state").await
    }

    /// Replicates one key/value write through the current leader.
    pub async fn write(
        &self,
        key: &str,
        value: &str,
        serial: u64,
    ) -> anyhow::Result<OpenRaftKvWriteResponse> {
        self.post_result(
            "kv/write",
            &OpenRaftKvWriteRequest {
                key: key.to_owned(),
                value: value.to_owned(),
                serial,
            },
        )
        .await
    }

    /// Reads one key from the replicated state machine.
    pub async fn read(&self, key: &str) -> anyhow::Result<Option<String>> {
        let response: OpenRaftKvReadResponse = self
            .post_result(
                "kv/read",
                &OpenRaftKvReadRequest {
                    key: key.to_owned(),
                },
            )
            .await?;
        Ok(response.value)
    }

    /// Bootstraps a one-node cluster on this node.
    pub async fn init_self(&self) -> anyhow::Result<()> {
        let _: InitResult = self.post("admin/init", &()).await?;
        Ok(())
    }

    /// Registers another node as a learner with the current leader.
    pub async fn add_learner(&self, node_id: u64, addr: &str) -> anyhow::Result<()> {
        let _: AddLearnerResult = self
            .post(
                "admin/add-learner",
                &AddLearnerRequest {
                    node_id,
                    addr: addr.to_owned(),
                },
            )
            .await?;
        Ok(())
    }

    /// Promotes the cluster to the provided voter set.
    pub async fn change_membership(
        &self,
        voters: impl IntoIterator<Item = u64>,
    ) -> anyhow::Result<()> {
        let voters = normalize_voters(voters);
        let request = ChangeMembershipRequest { voters };

        let _: ChangeMembershipResult = self.post("admin/change-membership", &request).await?;
        Ok(())
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let url = self.base_url.join(path)?;
        let response = self.client.get(url).send().await?;
        let response = response.error_for_status()?;

        Ok(response.json().await?)
    }

    async fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        let url = self.base_url.join(path)?;

        let response = self.client.post(url).json(body).send().await?;

        let response = response.error_for_status()?;

        Ok(response.json().await?)
    }

    async fn post_result<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        let result: Result<T, String> = self.post(path, body).await?;
        result.map_err(anyhow::Error::msg)
    }
}

fn normalize_voters(voters: impl IntoIterator<Item = u64>) -> Vec<u64> {
    let unique_voters = voters.into_iter().collect::<BTreeSet<_>>();
    unique_voters.into_iter().collect()
}

//! HTTP transport used by OpenRaft to replicate between example nodes.

use std::{collections::BTreeMap, sync::Arc};

use openraft::{
    RaftNetworkFactory, RaftNetworkV2,
    alias::{SnapshotOf, VoteOf},
    errors::{RPCError, StreamingError, Unreachable},
    network::RPCOption,
};
use reqwest::Url;
use tokio::sync::RwLock;

use crate::{
    TypeConfig,
    types::{InstallFullSnapshotBody, SnapshotRpcResult},
};

/// Shared node-address book used by Raft RPC clients.
#[derive(Clone, Default)]
pub struct HttpNetworkFactory {
    client: reqwest::Client,
    known_nodes: Arc<RwLock<BTreeMap<u64, String>>>,
}

/// Per-target HTTP client used for Raft replication traffic.
pub struct HttpNetworkClient {
    client: reqwest::Client,
    target: u64,
    target_addr: Option<String>,
}

impl HttpNetworkFactory {
    /// Creates a network factory backed by one shared node-address map.
    #[must_use]
    pub fn new(known_nodes: Arc<RwLock<BTreeMap<u64, String>>>) -> Self {
        Self {
            client: reqwest::Client::new(),
            known_nodes,
        }
    }
}

impl RaftNetworkFactory<TypeConfig> for HttpNetworkFactory {
    type Network = HttpNetworkClient;

    async fn new_client(&mut self, target: u64, _node: &()) -> Self::Network {
        let target_addr = self.known_nodes.read().await.get(&target).cloned();

        HttpNetworkClient {
            client: self.client.clone(),
            target,
            target_addr,
        }
    }
}

impl RaftNetworkV2<TypeConfig> for HttpNetworkClient {
    async fn append_entries(
        &mut self,
        rpc: openraft::raft::AppendEntriesRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<openraft::raft::AppendEntriesResponse<TypeConfig>, RPCError<TypeConfig>> {
        self.post_rpc("raft/append", &rpc).await
    }

    async fn vote(
        &mut self,
        rpc: openraft::raft::VoteRequest<TypeConfig>,
        _option: RPCOption,
    ) -> Result<openraft::raft::VoteResponse<TypeConfig>, RPCError<TypeConfig>> {
        self.post_rpc("raft/vote", &rpc).await
    }

    async fn full_snapshot(
        &mut self,
        vote: VoteOf<TypeConfig>,
        snapshot: SnapshotOf<TypeConfig>,
        _cancel: impl std::future::Future<Output = openraft::errors::ReplicationClosed>
        + openraft::OptionalSend
        + 'static,
        _option: RPCOption,
    ) -> Result<openraft::raft::SnapshotResponse<TypeConfig>, StreamingError<TypeConfig>> {
        let body = InstallFullSnapshotBody {
            vote,
            meta: snapshot.meta,
            data: snapshot.snapshot.into_inner(),
        };

        self.post_snapshot("raft/snapshot", &body).await
    }
}

impl HttpNetworkClient {
    async fn post_rpc<B, T>(&self, path: &str, body: &B) -> Result<T, RPCError<TypeConfig>>
    where
        B: serde::Serialize,
        T: serde::de::DeserializeOwned,
    {
        let url = self.endpoint_url(path)?;
        let response = self
            .client
            .post(url)
            .json(body)
            .send()
            .await
            .map_err(|err| RPCError::Unreachable(Unreachable::new(&err)))?
            .error_for_status()
            .map_err(|err| RPCError::Unreachable(Unreachable::new(&err)))?;

        let result: Result<T, String> = response
            .json()
            .await
            .map_err(|err| RPCError::Unreachable(Unreachable::new(&err)))?;

        result.map_err(|err| RPCError::Unreachable(Unreachable::from_string(err)))
    }

    async fn post_snapshot(
        &self,
        path: &str,
        body: &InstallFullSnapshotBody,
    ) -> Result<openraft::raft::SnapshotResponse<TypeConfig>, StreamingError<TypeConfig>> {
        let url = self
            .endpoint_url(path)
            .map_err(|err| StreamingError::Unreachable(Unreachable::new(&err)))?;
        let response = self
            .client
            .post(url)
            .json(body)
            .send()
            .await
            .map_err(|err| StreamingError::Unreachable(Unreachable::new(&err)))?
            .error_for_status()
            .map_err(|err| StreamingError::Unreachable(Unreachable::new(&err)))?;

        let result: SnapshotRpcResult = response
            .json()
            .await
            .map_err(|err| StreamingError::Unreachable(Unreachable::new(&err)))?;

        result.map_err(|err| StreamingError::Unreachable(Unreachable::from_string(err)))
    }

    fn endpoint_url(&self, path: &str) -> Result<Url, Unreachable<TypeConfig>> {
        let Some(addr) = &self.target_addr else {
            return Err(Unreachable::from_string(format!(
                "target {} has no known address",
                self.target
            )));
        };

        let mut url =
            Url::parse(&format!("http://{addr}/")).map_err(|err| Unreachable::new(&err))?;
        url.set_path(path);
        Ok(url)
    }
}

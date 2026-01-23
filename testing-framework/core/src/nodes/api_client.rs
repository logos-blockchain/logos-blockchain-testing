use std::net::SocketAddr;

use chain_service::CryptarchiaInfo;
use common_http_client::CommonHttpClient;
use hex;
use nomos_core::{block::Block, mantle::SignedMantleTx};
use nomos_http_api_common::paths::{
    CRYPTARCHIA_HEADERS, CRYPTARCHIA_INFO, MEMPOOL_ADD_TX, NETWORK_INFO, STORAGE_BLOCK,
};
use nomos_network::backends::libp2p::Libp2pInfo;
use nomos_node::HeaderId;
use reqwest::{Client, RequestBuilder, Response, Url};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use tracing::error;

pub const DA_GET_TESTING_ENDPOINT_ERROR: &str = "Failed to connect to testing endpoint. The binary was likely built without the 'testing' \
     feature. Try: cargo build --workspace --all-features";

#[derive(Debug, thiserror::Error)]
pub enum ApiClientError {
    #[error("{DA_GET_TESTING_ENDPOINT_ERROR}")]
    TestingEndpointUnavailable,
    #[error(transparent)]
    Request(#[from] reqwest::Error),
}

/// Thin async client for node HTTP/testing endpoints.
#[derive(Clone)]
pub struct ApiClient {
    pub(crate) base_url: Url,
    pub(crate) testing_url: Option<Url>,
    client: Client,
    pub(crate) http_client: CommonHttpClient,
}

impl ApiClient {
    #[must_use]
    /// Construct from socket addresses.
    pub fn new(base_addr: SocketAddr, testing_addr: Option<SocketAddr>) -> Self {
        let base_url = Url::parse(&format!("http://{base_addr}")).unwrap_or_else(|_| unsafe {
            // Safety: `SocketAddr` formatting yields a valid host:port pair.
            std::hint::unreachable_unchecked()
        });
        let testing_url = testing_addr.map(|addr| {
            Url::parse(&format!("http://{addr}")).unwrap_or_else(|_| unsafe {
                // Safety: `SocketAddr` formatting yields a valid host:port pair.
                std::hint::unreachable_unchecked()
            })
        });
        Self::from_urls(base_url, testing_url)
    }

    #[must_use]
    /// Construct from prebuilt URLs.
    pub fn from_urls(base_url: Url, testing_url: Option<Url>) -> Self {
        let client = Client::new();
        Self {
            base_url,
            testing_url,
            http_client: CommonHttpClient::new_with_client(client.clone(), None),
            client,
        }
    }

    #[must_use]
    /// Testing URL, when built with testing features.
    pub fn testing_url(&self) -> Option<Url> {
        self.testing_url.clone()
    }

    /// Build a GET request against the base API.
    pub fn get_builder(&self, path: &str) -> RequestBuilder {
        self.client.get(self.join_base(path))
    }

    /// Issue a GET request against the base API.
    pub async fn get_response(&self, path: &str) -> reqwest::Result<Response> {
        self.client.get(self.join_base(path)).send().await
    }

    /// GET and decode JSON from the base API.
    pub async fn get_json<T>(&self, path: &str) -> reqwest::Result<T>
    where
        T: DeserializeOwned,
    {
        self.get_response(path)
            .await?
            .error_for_status()?
            .json()
            .await
    }

    /// POST JSON to the base API and decode a response.
    pub async fn post_json_decode<T, R>(&self, path: &str, body: &T) -> reqwest::Result<R>
    where
        T: Serialize + Sync + ?Sized,
        R: DeserializeOwned,
    {
        self.post_json_response(path, body)
            .await?
            .error_for_status()?
            .json()
            .await
    }

    /// POST JSON to the base API and return the raw response.
    pub async fn post_json_response<T>(&self, path: &str, body: &T) -> reqwest::Result<Response>
    where
        T: Serialize + Sync + ?Sized,
    {
        self.client
            .post(self.join_base(path))
            .json(body)
            .send()
            .await
    }

    /// POST JSON to the base API and expect a success status.
    pub async fn post_json_unit<T>(&self, path: &str, body: &T) -> reqwest::Result<()>
    where
        T: Serialize + Sync + ?Sized,
    {
        self.post_json_response(path, body)
            .await?
            .error_for_status()?;
        Ok(())
    }

    /// GET and decode JSON from the testing API.
    pub async fn get_testing_json<T>(&self, path: &str) -> Result<T, ApiClientError>
    where
        T: DeserializeOwned,
    {
        self.get_testing_response_checked(path)
            .await?
            .error_for_status()
            .map_err(ApiClientError::Request)?
            .json()
            .await
            .map_err(ApiClientError::Request)
    }

    /// POST JSON to the testing API and decode a response.
    pub async fn post_testing_json_decode<T, R>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<R, ApiClientError>
    where
        T: Serialize + Sync + ?Sized,
        R: DeserializeOwned,
    {
        self.post_testing_json_response_checked(path, body)
            .await?
            .error_for_status()
            .map_err(ApiClientError::Request)?
            .json()
            .await
            .map_err(ApiClientError::Request)
    }

    /// POST JSON to the testing API and expect a success status.
    pub async fn post_testing_json_unit<T>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<(), ApiClientError>
    where
        T: Serialize + Sync + ?Sized,
    {
        self.post_testing_json_response_checked(path, body)
            .await?
            .error_for_status()
            .map_err(ApiClientError::Request)?;
        Ok(())
    }

    /// POST JSON to the testing API and return the raw response.
    pub async fn post_testing_json_response_checked<T>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<Response, ApiClientError>
    where
        T: Serialize + Sync + ?Sized,
    {
        let testing_url = self
            .testing_url
            .as_ref()
            .ok_or(ApiClientError::TestingEndpointUnavailable)?;
        self.client
            .post(Self::join_url(testing_url, path))
            .json(body)
            .send()
            .await
            .map_err(ApiClientError::Request)
    }

    pub async fn post_testing_json_response<T>(
        &self,
        path: &str,
        body: &T,
    ) -> Result<Response, ApiClientError>
    where
        T: Serialize + Sync + ?Sized,
    {
        self.post_testing_json_response_checked(path, body).await
    }

    /// GET from the testing API and return the raw response.
    pub async fn get_testing_response_checked(
        &self,
        path: &str,
    ) -> Result<Response, ApiClientError> {
        let testing_url = self
            .testing_url
            .as_ref()
            .ok_or(ApiClientError::TestingEndpointUnavailable)?;
        self.client
            .get(Self::join_url(testing_url, path))
            .send()
            .await
            .map_err(ApiClientError::Request)
    }

    pub async fn get_testing_response(&self, path: &str) -> Result<Response, ApiClientError> {
        self.get_testing_response_checked(path).await
    }

    /// Fetch consensus info from the base API.
    pub async fn consensus_info(&self) -> reqwest::Result<CryptarchiaInfo> {
        self.get_json(CRYPTARCHIA_INFO).await
    }

    /// Fetch libp2p network info.
    pub async fn network_info(&self) -> reqwest::Result<Libp2pInfo> {
        self.get_json(NETWORK_INFO).await
    }

    /// Fetch a block by hash from storage.
    pub async fn storage_block(
        &self,
        id: &HeaderId,
    ) -> reqwest::Result<Option<Block<SignedMantleTx>>> {
        self.post_json_decode(STORAGE_BLOCK, id).await
    }

    /// Fetch header ids between optional bounds.
    /// When `from` is None, defaults to tip; when `to` is None, defaults to
    /// LIB.
    pub async fn consensus_headers(
        &self,
        from: Option<HeaderId>,
        to: Option<HeaderId>,
    ) -> reqwest::Result<Vec<HeaderId>> {
        let mut url = self.join_base(CRYPTARCHIA_HEADERS);
        {
            let mut pairs = url.query_pairs_mut();
            if let Some(from) = from {
                let bytes: [u8; 32] = from.into();
                pairs.append_pair("from", &hex::encode(bytes));
            }
            if let Some(to) = to {
                let bytes: [u8; 32] = to.into();
                pairs.append_pair("to", &hex::encode(bytes));
            }
        }
        self.client
            .get(url)
            .send()
            .await?
            .error_for_status()?
            .json()
            .await
    }

    /// Submit a mantle transaction through the base API.
    pub async fn submit_transaction(&self, tx: &SignedMantleTx) -> reqwest::Result<()> {
        let res = self.post_json_response(MEMPOOL_ADD_TX, tx).await?;
        if let Err(status_err) = res.error_for_status_ref() {
            let status = res.status();
            let body = res
                .text()
                .await
                .unwrap_or_else(|_| "<unreadable body>".to_string());
            error!(%status, %body, "submit_transaction request failed");
            return Err(status_err);
        }
        Ok(())
    }

    /// Execute a custom request built by the caller.
    pub async fn get_headers_raw(&self, builder: RequestBuilder) -> reqwest::Result<Response> {
        builder.send().await
    }

    /// Fetch raw mempool metrics from the testing endpoint.
    pub async fn mempool_metrics(&self, pool: &str) -> reqwest::Result<Value> {
        self.get_json(&format!("/{pool}/metrics")).await
    }

    #[must_use]
    /// Base API URL.
    pub const fn base_url(&self) -> &Url {
        &self.base_url
    }

    #[must_use]
    /// Underlying common HTTP client wrapper.
    pub const fn http_client(&self) -> &CommonHttpClient {
        &self.http_client
    }

    fn join_base(&self, path: &str) -> Url {
        Self::join_url(&self.base_url, path)
    }

    fn join_url(base: &Url, path: &str) -> Url {
        let trimmed = path.trim_start_matches('/');
        match base.join(trimmed) {
            Ok(url) => url,
            Err(err) => {
                error!(
                    error = %err,
                    base = %base,
                    path,
                    "failed to join url; falling back to base url"
                );
                base.clone()
            }
        }
    }
}

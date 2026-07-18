use reqwest::Url;
use serde::Serialize;

#[derive(Clone)]
pub struct KvHttpClient {
    base_url: Url,
    client: reqwest::Client,
}

impl KvHttpClient {
    #[must_use]
    pub fn new(base_url: Url) -> Self {
        Self {
            base_url,
            client: reqwest::Client::new(),
        }
    }

    #[must_use]
    pub const fn base_url(&self) -> &Url {
        &self.base_url
    }

    pub async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let url = self.base_url.join(path)?;
        let response = self.client.get(url).send().await?.error_for_status()?;
        Ok(response.json().await?)
    }

    pub async fn put<B: Serialize, T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
    ) -> anyhow::Result<T> {
        let url = self.base_url.join(path)?;
        let response = self
            .client
            .put(url)
            .json(body)
            .send()
            .await?
            .error_for_status()?;
        Ok(response.json().await?)
    }
}

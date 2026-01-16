use reqwest::{Client, Response};
use serde::de::DeserializeOwned;

use crate::server::ClientIp;

#[derive(Debug)]
pub struct FetchedConfig<Config> {
    pub config: Config,
    pub raw: serde_json::Value,
}

async fn deserialize_response<Config: DeserializeOwned>(
    response: Response,
) -> Result<FetchedConfig<Config>, String> {
    let body = response
        .text()
        .await
        .map_err(|error| format!("Failed to read response body: {error}"))?;
    let raw: serde_json::Value =
        serde_json::from_str(&body).map_err(|error| format!("Failed to parse body: {error}"))?;
    let mut json_deserializer = serde_json::Deserializer::from_str(&body);
    let config = serde_path_to_error::deserialize(&mut json_deserializer)
        .map_err(|error| format!("Failed to deserialize body: {error}, raw body: {body}"))?;

    Ok(FetchedConfig { config, raw })
}

pub async fn get_config<Config: DeserializeOwned>(
    payload: ClientIp,
    url: &str,
) -> Result<FetchedConfig<Config>, String> {
    let client = Client::new();

    let response = client
        .post(url)
        .json(&payload)
        .send()
        .await
        .map_err(|err| format!("Failed to send IP announcement: {err}"))?;

    if !response.status().is_success() {
        return Err(format!("Server error: {:?}", response.status()));
    }

    deserialize_response(response).await
}

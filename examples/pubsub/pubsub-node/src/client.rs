use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq)]
pub struct PubSubEventId {
    pub origin: u64,
    pub seq: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PubSubEvent {
    pub id: PubSubEventId,
    pub topic: String,
    pub payload: String,
}

#[derive(Clone)]
pub struct PubSubClient {
    base_url: Url,
    ws_url: Url,
    client: reqwest::Client,
}

impl PubSubClient {
    #[must_use]
    pub fn new(base_url: Url) -> Self {
        let mut ws_url = base_url.clone();
        ws_url
            .set_scheme(if ws_url.scheme() == "https" {
                "wss"
            } else {
                "ws"
            })
            .ok();

        Self {
            base_url,
            ws_url,
            client: reqwest::Client::new(),
        }
    }

    #[must_use]
    pub fn ws_endpoint(&self) -> String {
        self.ws_url
            .join("/ws")
            .expect("valid ws endpoint")
            .to_string()
    }

    pub async fn connect(&self) -> anyhow::Result<PubSubSession> {
        let (ws, _) = connect_async(self.ws_endpoint()).await?;
        Ok(PubSubSession { ws })
    }

    pub async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let url = self.base_url.join(path)?;
        let response = self.client.get(url).send().await?.error_for_status()?;
        Ok(response.json().await?)
    }
}

pub struct PubSubSession {
    ws: WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>,
}

impl PubSubSession {
    pub async fn close(&mut self) -> anyhow::Result<()> {
        self.ws.close(None).await?;
        Ok(())
    }

    pub async fn subscribe(&mut self, topic: &str) -> anyhow::Result<()> {
        self.send_frame(&ClientFrame::Subscribe {
            topic: topic.to_owned(),
        })
        .await
    }

    pub async fn unsubscribe(&mut self, topic: &str) -> anyhow::Result<()> {
        self.send_frame(&ClientFrame::Unsubscribe {
            topic: topic.to_owned(),
        })
        .await
    }

    pub async fn publish(&mut self, topic: &str, payload: String) -> anyhow::Result<()> {
        self.send_frame(&ClientFrame::Publish {
            topic: topic.to_owned(),
            payload,
        })
        .await
    }

    pub async fn next_event_timeout(
        &mut self,
        timeout: Duration,
    ) -> anyhow::Result<Option<PubSubEvent>> {
        let Some(message) = tokio::time::timeout(timeout, self.ws.next())
            .await
            .ok()
            .flatten()
        else {
            return Ok(None);
        };

        self.parse_message(message?)
    }

    async fn send_frame(&mut self, frame: &ClientFrame) -> anyhow::Result<()> {
        self.ws
            .send(Message::Text(serde_json::to_string(frame)?))
            .await?;
        Ok(())
    }

    fn parse_message(&self, msg: Message) -> anyhow::Result<Option<PubSubEvent>> {
        let Message::Text(text) = msg else {
            return Ok(None);
        };

        let frame: ServerFrame = serde_json::from_str(&text)?;
        Ok(match frame {
            ServerFrame::Event { event } => Some(event),
            ServerFrame::Other
            | ServerFrame::Subscribed
            | ServerFrame::Unsubscribed
            | ServerFrame::Published
            | ServerFrame::Error => None,
        })
    }
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ClientFrame {
    Subscribe { topic: String },
    Unsubscribe { topic: String },
    Publish { topic: String, payload: String },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerFrame {
    Subscribed,
    Unsubscribed,
    Published,
    Event {
        event: PubSubEvent,
    },
    Error,
    #[serde(other)]
    Other,
}

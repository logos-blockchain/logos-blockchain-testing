use async_trait::async_trait;
use redis::{
    AsyncCommands, Client,
    aio::ConnectionManager,
    streams::{StreamAutoClaimOptions, StreamAutoClaimReply, StreamPendingReply, StreamReadReply},
};
use testing_framework_core::{
    cfgsync::{StaticNodeConfigProvider, serialize_plain_text_config},
    scenario::{Application, DynError, NodeAccess},
};

pub type RedisStreamsTopology = testing_framework_core::topology::ClusterTopology;

#[derive(Clone)]
pub struct RedisStreamsClient {
    url: String,
    client: Client,
}

impl RedisStreamsClient {
    pub fn new(url: String) -> Result<Self, DynError> {
        let client = Client::open(url.clone())?;
        Ok(Self { url, client })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    async fn connection(&self) -> Result<ConnectionManager, DynError> {
        Ok(self.client.get_connection_manager().await?)
    }

    pub async fn ping(&self) -> Result<(), DynError> {
        let mut conn = self.connection().await?;
        redis::cmd("PING").query_async::<String>(&mut conn).await?;
        Ok(())
    }

    pub async fn ensure_group(&self, stream: &str, group: &str) -> Result<(), DynError> {
        let mut conn = self.connection().await?;
        let result = redis::cmd("XGROUP")
            .arg("CREATE")
            .arg(stream)
            .arg(group)
            .arg("$")
            .arg("MKSTREAM")
            .query_async::<()>(&mut conn)
            .await;

        match result {
            Ok(()) => Ok(()),
            Err(error) if error.to_string().contains("BUSYGROUP") => Ok(()),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn append_message(&self, stream: &str, payload: &str) -> Result<String, DynError> {
        let mut conn = self.connection().await?;
        let id = redis::cmd("XADD")
            .arg(stream)
            .arg("*")
            .arg("payload")
            .arg(payload)
            .query_async::<String>(&mut conn)
            .await?;
        Ok(id)
    }

    pub async fn read_group_batch(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        count: usize,
        block_ms: usize,
    ) -> Result<Vec<String>, DynError> {
        let mut conn = self.connection().await?;

        let options = redis::streams::StreamReadOptions::default()
            .group(group, consumer)
            .count(count)
            .block(block_ms);

        let reply: StreamReadReply = conn.xread_options(&[stream], &[">"], &options).await?;

        let mut ids = Vec::new();
        for key in reply.keys {
            for entry in key.ids {
                ids.push(entry.id);
            }
        }

        Ok(ids)
    }

    pub async fn ack_messages(
        &self,
        stream: &str,
        group: &str,
        ids: &[String],
    ) -> Result<u64, DynError> {
        if ids.is_empty() {
            return Ok(0);
        }

        let mut conn = self.connection().await?;
        let acked = redis::cmd("XACK")
            .arg(stream)
            .arg(group)
            .arg(ids)
            .query_async::<u64>(&mut conn)
            .await?;
        Ok(acked)
    }

    pub async fn autoclaim_batch(
        &self,
        stream: &str,
        group: &str,
        consumer: &str,
        min_idle_ms: u64,
        start: &str,
        count: usize,
    ) -> Result<(String, Vec<String>), DynError> {
        let mut conn = self.connection().await?;
        let options = StreamAutoClaimOptions::default().count(count);
        let reply: StreamAutoClaimReply = conn
            .xautoclaim_options(stream, group, consumer, min_idle_ms, start, options)
            .await?;

        Ok((
            reply.next_stream_id,
            reply.claimed.into_iter().map(|entry| entry.id).collect(),
        ))
    }

    pub async fn pending_count(&self, stream: &str, group: &str) -> Result<u64, DynError> {
        let mut conn = self.connection().await?;
        let reply: StreamPendingReply = conn.xpending(stream, group).await?;
        Ok(reply.count() as u64)
    }
}

pub struct RedisStreamsEnv;

#[async_trait]
impl Application for RedisStreamsEnv {
    type Deployment = RedisStreamsTopology;
    type NodeClient = RedisStreamsClient;
    type NodeConfig = String;

    fn build_node_client(access: &NodeAccess) -> Result<Self::NodeClient, DynError> {
        let port = access.testing_port().unwrap_or(access.api_port());
        RedisStreamsClient::new(format!("redis://{}:{port}", access.host()))
    }
}

impl StaticNodeConfigProvider for RedisStreamsEnv {
    type Error = std::convert::Infallible;

    fn build_node_config(
        _deployment: &Self::Deployment,
        _node_index: usize,
    ) -> Result<Self::NodeConfig, Self::Error> {
        Ok("appendonly yes\nprotected-mode no\n".to_owned())
    }

    fn serialize_node_config(config: &Self::NodeConfig) -> Result<String, Self::Error> {
        serialize_plain_text_config(config)
    }
}

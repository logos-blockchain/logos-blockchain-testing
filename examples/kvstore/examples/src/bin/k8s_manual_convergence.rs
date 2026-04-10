use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use kvstore_node::KvHttpClient;
use kvstore_runtime_ext::{KvK8sDeployer, KvTopology};
use serde::{Deserialize, Serialize};
use testing_framework_runner_k8s::ManualClusterError;
use tracing::{info, warn};

#[derive(Serialize)]
struct PutRequest {
    value: String,
    expected_version: Option<u64>,
}

#[derive(Deserialize)]
struct PutResponse {
    applied: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
struct ValueRecord {
    value: String,
    version: u64,
    origin: u64,
}

#[derive(Deserialize)]
struct GetResponse {
    record: Option<ValueRecord>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let deployer = KvK8sDeployer::new();
    let cluster = match deployer
        .manual_cluster_from_descriptors(KvTopology::new(3))
        .await
    {
        Ok(cluster) => cluster,
        Err(ManualClusterError::ClientInit { source }) => {
            warn!("k8s unavailable ({source}); skipping kv k8s manual run");
            return Ok(());
        }
        Err(ManualClusterError::InstallStack { source })
            if k8s_cluster_unavailable(&source.to_string()) =>
        {
            warn!("k8s unavailable ({source}); skipping kv k8s manual run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error)).context("creating kv k8s manual cluster");
        }
    };

    let node0 = cluster.start_node("node-0").await?.client;
    let node1 = cluster.start_node("node-1").await?.client;
    let node2 = cluster.start_node("node-2").await?.client;

    cluster.wait_network_ready().await?;

    write_keys(&node0, "kv-manual", 12).await?;
    wait_for_convergence(
        &[node0.clone(), node1.clone(), node2.clone()],
        "kv-manual",
        12,
    )
    .await?;

    info!("restarting node-2 in manual cluster");
    cluster.restart_node("node-2").await?;
    cluster.wait_network_ready().await?;

    let node2 = cluster
        .node_client("node-2")
        .ok_or_else(|| anyhow!("node-2 client missing after restart"))?;
    wait_for_convergence(&[node0, node1, node2], "kv-manual", 12).await?;

    cluster.stop_all();
    Ok(())
}

async fn write_keys(client: &KvHttpClient, prefix: &str, key_count: usize) -> Result<()> {
    for index in 0..key_count {
        let key = format!("{prefix}-{index}");
        let response: PutResponse = client
            .put(
                &format!("/kv/{key}"),
                &PutRequest {
                    value: format!("value-{index}"),
                    expected_version: None,
                },
            )
            .await
            .map_err(|error| anyhow!(error.to_string()))
            .with_context(|| format!("writing key {key}"))?;

        if !response.applied {
            return Err(anyhow!("write rejected for key {key}"));
        }
    }

    Ok(())
}

async fn wait_for_convergence(
    clients: &[KvHttpClient],
    prefix: &str,
    key_count: usize,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

    while tokio::time::Instant::now() < deadline {
        if is_converged(clients, prefix, key_count).await? {
            info!(key_count, "kv manual cluster converged");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    Err(anyhow!("kv manual cluster did not converge within timeout"))
}

async fn is_converged(clients: &[KvHttpClient], prefix: &str, key_count: usize) -> Result<bool> {
    for index in 0..key_count {
        let key = format!("{prefix}-{index}");
        let first = read_key(&clients[0], &key).await?;
        for client in &clients[1..] {
            if read_key(client, &key).await? != first {
                return Ok(false);
            }
        }
    }

    Ok(true)
}

async fn read_key(client: &KvHttpClient, key: &str) -> Result<Option<ValueRecord>> {
    let response: GetResponse = client
        .get(&format!("/kv/{key}"))
        .await
        .map_err(|error| anyhow!(error.to_string()))
        .with_context(|| format!("reading key {key}"))?;
    Ok(response.record)
}

fn k8s_cluster_unavailable(message: &str) -> bool {
    message.contains("Unable to connect to the server")
        || message.contains("TLS handshake timeout")
        || message.contains("connection refused")
}

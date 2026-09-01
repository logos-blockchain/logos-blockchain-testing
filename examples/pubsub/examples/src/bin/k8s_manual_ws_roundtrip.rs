use std::{collections::HashMap, time::Duration};

use anyhow::{Context as _, Result, anyhow};
use pubsub_node::{PubSubClient, PubSubEventId, PubSubSession};
use pubsub_runtime_ext::{PubSubK8sDeployer, PubSubTopology};
use serde::Deserialize;
use testing_framework_runner_k8s::ManualClusterError;
use tracing::{info, warn};

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct Revision {
    version: u64,
    origin: u64,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
struct TopicsStateView {
    revision: Revision,
    total_events: usize,
    topic_counts: HashMap<String, usize>,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let topic = "manual.demo";
    let deployer = PubSubK8sDeployer::new();
    let cluster = match deployer
        .manual_cluster_from_descriptors(PubSubTopology::new(3))
        .await
    {
        Ok(cluster) => cluster,
        Err(ManualClusterError::ClientInit { source }) if cluster_may_be_skipped() => {
            warn!("k8s unavailable ({source}); skipping pubsub k8s manual run");
            return Ok(());
        }
        Err(ManualClusterError::InstallStack { source })
            if cluster_may_be_skipped() && k8s_cluster_unavailable(&source.to_string()) =>
        {
            warn!("k8s unavailable ({source}); skipping pubsub k8s manual run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error)).context("creating pubsub k8s manual cluster");
        }
    };

    let node0 = cluster.start_node("node-0").await?.client;
    let node1 = cluster.start_node("node-1").await?.client;
    let node2 = cluster.start_node("node-2").await?.client;

    cluster.wait_network_ready().await?;

    roundtrip_batch(&node0, &[node1.clone(), node2.clone()], topic, 24, 0).await?;
    wait_for_topic_convergence(&[node0.clone(), node1.clone(), node2.clone()], topic, 24).await?;

    info!("restarting node-2 in manual cluster");
    cluster.restart_node("node-2").await?;
    cluster.wait_network_ready().await?;

    let restarted_node2 = cluster
        .node_client("node-2")
        .ok_or_else(|| anyhow!("node-2 client missing after restart"))?;

    roundtrip_batch(
        &node0,
        &[node1.clone(), restarted_node2.clone()],
        topic,
        12,
        24,
    )
    .await?;
    wait_for_topic_convergence(&[node0, node1, restarted_node2], topic, 36).await?;

    cluster.stop_all();
    Ok(())
}

async fn roundtrip_batch(
    publisher_client: &PubSubClient,
    subscriber_clients: &[PubSubClient],
    topic: &str,
    message_count: usize,
    start_index: usize,
) -> Result<()> {
    let mut subscribers = Vec::with_capacity(subscriber_clients.len());
    for client in subscriber_clients {
        let mut session = client
            .connect()
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        session
            .subscribe(topic)
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        subscribers.push(session);
    }

    let mut publisher = publisher_client
        .connect()
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    for index in start_index..start_index + message_count {
        publisher
            .publish(topic, format!("msg-{index}"))
            .await
            .map_err(|error| anyhow!(error.to_string()))
            .with_context(|| format!("publishing message {index}"))?;
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let deliveries = collect_deliveries(&mut subscribers, message_count).await?;
    ensure_deliveries_match(&deliveries, message_count)?;

    for session in &mut subscribers {
        session
            .close()
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
    }

    publisher
        .close()
        .await
        .map_err(|error| anyhow!(error.to_string()))?;
    Ok(())
}

async fn collect_deliveries(
    subscribers: &mut [PubSubSession],
    expected_messages: usize,
) -> Result<Vec<HashMap<PubSubEventId, String>>> {
    let mut deliveries = vec![HashMap::new(); subscribers.len()];
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

    while tokio::time::Instant::now() < deadline
        && deliveries.iter().any(|seen| seen.len() < expected_messages)
    {
        for (index, session) in subscribers.iter_mut().enumerate() {
            if deliveries[index].len() >= expected_messages {
                continue;
            }

            if let Some(event) = session
                .next_event_timeout(Duration::from_millis(200))
                .await
                .map_err(|error| anyhow!(error.to_string()))?
            {
                deliveries[index].entry(event.id).or_insert(event.payload);
            }
        }
    }

    Ok(deliveries)
}

fn ensure_deliveries_match(
    deliveries: &[HashMap<PubSubEventId, String>],
    expected_messages: usize,
) -> Result<()> {
    for (index, seen) in deliveries.iter().enumerate() {
        if seen.len() != expected_messages {
            return Err(anyhow!(
                "subscriber {index} saw {}/{} messages",
                seen.len(),
                expected_messages
            ));
        }
    }

    if let Some((baseline, rest)) = deliveries.split_first() {
        for seen in rest {
            if seen != baseline {
                return Err(anyhow!("subscriber deliveries diverged"));
            }
        }
    }

    Ok(())
}

async fn wait_for_topic_convergence(
    clients: &[PubSubClient],
    topic: &str,
    expected_messages: usize,
) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

    while tokio::time::Instant::now() < deadline {
        if topic_converged(clients, topic, expected_messages).await? {
            info!(expected_messages, "pubsub manual cluster converged");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }

    Err(anyhow!(
        "pubsub manual cluster did not converge on topic state within timeout"
    ))
}

async fn topic_converged(
    clients: &[PubSubClient],
    topic: &str,
    expected_messages: usize,
) -> Result<bool> {
    let mut baseline: Option<TopicsStateView> = None;

    for client in clients {
        let state: TopicsStateView = client
            .get("/topics/state")
            .await
            .map_err(|error| anyhow!(error.to_string()))?;

        if state.total_events != expected_messages {
            return Ok(false);
        }

        if state.topic_counts.get(topic).copied().unwrap_or_default() != expected_messages {
            return Ok(false);
        }

        match &baseline {
            Some(expected)
                if expected.revision != state.revision
                    || expected.topic_counts != state.topic_counts =>
            {
                return Ok(false);
            }
            None => baseline = Some(state),
            Some(_) => {}
        }
    }

    Ok(true)
}

fn cluster_may_be_skipped() -> bool {
    std::env::var("K8S_RUNNER_REQUIRE_CLUSTER").as_deref() != Ok("1")
}

fn k8s_cluster_unavailable(message: &str) -> bool {
    message.contains("Unable to connect to the server")
        || message.contains("TLS handshake timeout")
        || message.contains("connection refused")
}

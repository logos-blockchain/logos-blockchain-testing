use std::{env, time::Duration};

use anyhow::{Context as _, Result, anyhow};
use metrics_counter_node::MetricsCounterHttpClient;
use metrics_counter_runtime_ext::{MetricsCounterK8sDeployer, MetricsCounterTopology};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use testing_framework_runner_k8s::ManualClusterError;
use tracing::{info, warn};

const DEFAULT_PROM_URL: &str = "http://127.0.0.1:30991";

#[derive(Serialize)]
struct IncrementRequest {}

#[derive(Deserialize)]
struct CounterView {
    value: u64,
}

#[derive(Deserialize)]
struct PrometheusQueryResponse {
    data: PrometheusData,
}

#[derive(Deserialize)]
struct PrometheusData {
    result: Vec<PrometheusSample>,
}

#[derive(Deserialize)]
struct PrometheusSample {
    value: (serde_json::Value, String),
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let metrics_url = env::var("LOGOS_BLOCKCHAIN_METRICS_QUERY_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_PROM_URL.to_owned());

    let deployer = MetricsCounterK8sDeployer::new();
    let cluster = match deployer
        .manual_cluster_from_descriptors(MetricsCounterTopology::new(3))
        .await
    {
        Ok(cluster) => cluster,
        Err(ManualClusterError::ClientInit { source }) if cluster_may_be_skipped() => {
            warn!("k8s unavailable ({source}); skipping metrics-counter k8s manual run");
            return Ok(());
        }
        Err(ManualClusterError::InstallStack { source })
            if cluster_may_be_skipped() && k8s_cluster_unavailable(&source.to_string()) =>
        {
            warn!("k8s unavailable ({source}); skipping metrics-counter k8s manual run");
            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error))
                .context("creating metrics-counter k8s manual cluster");
        }
    };

    let node0 = cluster.start_node("node-0").await?.client;
    let node1 = cluster.start_node("node-1").await?.client;
    let node2 = cluster.start_node("node-2").await?.client;

    cluster.wait_network_ready().await?;

    increment_many(&node0, 40).await?;
    increment_many(&node1, 30).await?;
    increment_many(&node2, 20).await?;

    wait_for_counter_value(&node0, 40).await?;
    wait_for_counter_value(&node1, 30).await?;
    wait_for_counter_value(&node2, 20).await?;
    wait_for_prometheus_sum(&metrics_url, 90.0).await?;

    info!("restarting node-1 in manual cluster");
    cluster.restart_node("node-1").await?;
    cluster.wait_network_ready().await?;

    let restarted_node1 = cluster
        .node_client("node-1")
        .ok_or_else(|| anyhow!("node-1 client missing after restart"))?;

    wait_for_counter_value(&restarted_node1, 0).await?;

    increment_many(&node0, 10).await?;
    increment_many(&restarted_node1, 5).await?;

    wait_for_counter_value(&node0, 50).await?;
    wait_for_counter_value(&restarted_node1, 5).await?;
    wait_for_counter_value(&node2, 20).await?;
    wait_for_prometheus_sum(&metrics_url, 75.0).await?;

    cluster.stop_all();
    Ok(())
}

async fn increment_many(client: &MetricsCounterHttpClient, operations: usize) -> Result<()> {
    for _ in 0..operations {
        let _: CounterView = client
            .post("/counter/inc", &IncrementRequest {})
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
    }

    Ok(())
}

async fn wait_for_counter_value(client: &MetricsCounterHttpClient, expected: u64) -> Result<()> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);

    while tokio::time::Instant::now() < deadline {
        let view: CounterView = client
            .get("/counter/value")
            .await
            .map_err(|error| anyhow!(error.to_string()))?;
        if view.value == expected {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    Err(anyhow!("counter did not reach expected value {expected}"))
}

async fn wait_for_prometheus_sum(metrics_url: &str, expected: f64) -> Result<()> {
    let base_url = Url::parse(metrics_url)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);

    while tokio::time::Instant::now() < deadline {
        let total = query_prometheus_sum(&base_url).await?;
        if (total - expected).abs() < f64::EPSILON {
            info!(total, expected, "prometheus sum reached expected value");
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Err(anyhow!(
        "prometheus sum did not reach expected value {expected}"
    ))
}

async fn query_prometheus_sum(base_url: &Url) -> Result<f64> {
    let client = reqwest::Client::new();
    let mut url = base_url.join("/api/v1/query")?;
    url.query_pairs_mut()
        .append_pair("query", "sum(metrics_counter_value)");

    let response: PrometheusQueryResponse = client
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;

    let Some(sample) = response.data.result.first() else {
        return Ok(0.0);
    };

    sample
        .value
        .1
        .parse()
        .map_err(|error| anyhow!("invalid prometheus value: {error}"))
}

fn cluster_may_be_skipped() -> bool {
    env::var("K8S_RUNNER_REQUIRE_CLUSTER").as_deref() != Ok("1")
}

fn k8s_cluster_unavailable(message: &str) -> bool {
    message.contains("Unable to connect to the server")
        || message.contains("TLS handshake timeout")
        || message.contains("connection refused")
}

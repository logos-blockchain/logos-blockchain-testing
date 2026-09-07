use std::{net::Ipv4Addr, time::Duration};

use axum::{Router, http::StatusCode, routing::get};
use kvstore_node::KvHttpClient;
use queue_node::QueueHttpClient;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = WorkerArgs::parse()?;
    let queue = QueueHttpClient::new(args.queue_url);
    let results = KvHttpClient::new(args.results_url);

    tokio::spawn(process_jobs(queue, results));

    let app = Router::new().route("/health/ready", get(|| async { StatusCode::OK }));
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::UNSPECIFIED, args.health_port)).await?;
    info!(port = args.health_port, "job worker ready");
    axum::serve(listener, app).await?;

    Ok(())
}

async fn process_jobs(queue: QueueHttpClient, results: KvHttpClient) {
    loop {
        match dequeue(&queue).await {
            Ok(Some(job)) => write_result(&results, &job).await,
            Ok(None) => tokio::time::sleep(Duration::from_millis(100)).await,
            Err(error) => {
                warn!(%error, "failed to dequeue job");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
}

async fn dequeue(queue: &QueueHttpClient) -> anyhow::Result<Option<QueueMessage>> {
    let response: DequeueResponse = queue.post("/queue/dequeue", &EmptyRequest {}).await?;
    Ok(response.message)
}

async fn write_result(results: &KvHttpClient, job: &QueueMessage) {
    loop {
        let request = KvPutRequest {
            value: "completed",
            expected_version: None,
        };
        match results
            .put::<_, KvPutResponse>(&format!("/kv/{}", job.payload), &request)
            .await
        {
            Ok(response) if response.applied => {
                info!(job_id = job.id, job = %job.payload, "job completed");
                return;
            }
            Ok(_) => warn!(job_id = job.id, "result store rejected job result"),
            Err(error) => warn!(job_id = job.id, %error, "failed to write job result"),
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
}

struct WorkerArgs {
    queue_url: Url,
    results_url: Url,
    health_port: u16,
}

impl WorkerArgs {
    fn parse() -> anyhow::Result<Self> {
        let mut queue_url = None;
        let mut results_url = None;
        let mut health_port = None;
        let mut args = std::env::args().skip(1);

        while let Some(flag) = args.next() {
            let value = args
                .next()
                .ok_or_else(|| anyhow::anyhow!("missing value for {flag}"))?;
            match flag.as_str() {
                "--queue-url" => queue_url = Some(Url::parse(&value)?),
                "--results-url" => results_url = Some(Url::parse(&value)?),
                "--health-port" => health_port = Some(value.parse()?),
                _ => anyhow::bail!("unknown argument: {flag}"),
            }
        }

        Ok(Self {
            queue_url: queue_url.ok_or_else(|| anyhow::anyhow!("missing --queue-url"))?,
            results_url: results_url.ok_or_else(|| anyhow::anyhow!("missing --results-url"))?,
            health_port: health_port.ok_or_else(|| anyhow::anyhow!("missing --health-port"))?,
        })
    }
}

#[derive(Serialize)]
struct EmptyRequest {}

#[derive(Deserialize)]
struct DequeueResponse {
    message: Option<QueueMessage>,
}

#[derive(Deserialize)]
struct QueueMessage {
    id: u64,
    payload: String,
}

#[derive(Serialize)]
struct KvPutRequest {
    value: &'static str,
    expected_version: Option<u64>,
}

#[derive(Deserialize)]
struct KvPutResponse {
    applied: bool,
}

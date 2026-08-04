use std::{path::PathBuf, time::Duration};

use async_trait::async_trait;
use kvstore_runtime_ext::{KvEnv, KvLocalApp};
use queue_runtime_ext::{QueueEnv, QueueLocalApp};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use testing_framework_app::{
    AppDeployment, AppHostEnv, AppRunContextExt, DeployContext, LocalAppCluster, LocalProcessApp,
    LocalProcessHandle,
};
use testing_framework_core::scenario::{DynError, Expectation, RunContext, Workload};
use testing_framework_runner_local::{
    BinaryProvider, BinaryProviderRef, BuildBinaryProvider, BuildCommand, EnvBinaryProvider,
    FallbackBinaryProvider, LaunchSpec, NodeEndpoints, allocate_available_port,
};
use tokio::time::Instant;
use tracing::info;

#[derive(Clone)]
pub struct JobStackApp {
    queue: QueueLocalApp,
    results: KvLocalApp,
}

impl JobStackApp {
    #[must_use]
    pub fn new() -> Self {
        Self::with_cluster_sizes(2, 2)
    }

    #[must_use]
    pub fn with_cluster_sizes(queue_nodes: usize, result_nodes: usize) -> Self {
        Self {
            queue: QueueLocalApp::nodes(queue_nodes),
            results: KvLocalApp::nodes(result_nodes),
        }
    }
}

impl Default for JobStackApp {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl AppDeployment<AppHostEnv> for JobStackApp {
    type Handle = JobStackHandle;

    async fn deploy(self, ctx: &mut DeployContext<AppHostEnv>) -> Result<Self::Handle, DynError> {
        let queue = ctx.deploy_and_expose(self.queue).await?;
        let results = ctx.deploy_and_expose(self.results).await?;

        let queue_url = queue
            .first_client()
            .ok_or("queue cluster has no clients")?
            .base_url()
            .clone();
        let results_url = results
            .first_client()
            .ok_or("result store has no clients")?
            .base_url()
            .clone();
        let worker = ctx
            .deploy_and_expose(JobWorkerApp::new(queue_url, results_url))
            .await?;

        let stack = JobStackHandle {
            queue,
            results,
            worker,
        };
        ctx.expose(stack.clone())?;

        Ok(stack)
    }
}

#[derive(Clone)]
pub struct JobStackHandle {
    queue: LocalAppCluster<QueueEnv>,
    results: LocalAppCluster<KvEnv>,
    worker: LocalProcessHandle<WorkerClient>,
}

impl JobStackHandle {
    #[must_use]
    pub const fn queue(&self) -> &LocalAppCluster<QueueEnv> {
        &self.queue
    }

    #[must_use]
    pub const fn results(&self) -> &LocalAppCluster<KvEnv> {
        &self.results
    }

    #[must_use]
    pub const fn worker(&self) -> &LocalProcessHandle<WorkerClient> {
        &self.worker
    }
}

struct JobWorkerApp {
    queue_url: Url,
    results_url: Url,
}

impl JobWorkerApp {
    const fn new(queue_url: Url, results_url: Url) -> Self {
        Self {
            queue_url,
            results_url,
        }
    }
}

#[async_trait]
impl AppDeployment<AppHostEnv> for JobWorkerApp {
    type Handle = LocalProcessHandle<WorkerClient>;

    async fn deploy(self, ctx: &mut DeployContext<AppHostEnv>) -> Result<Self::Handle, DynError> {
        let health_port = allocate_available_port()?;
        let client = WorkerClient::new(health_port)?;
        let launch = LaunchSpec {
            binary: worker_binary_provider().resolve().await?,
            args: vec![
                "--queue-url".to_owned(),
                self.queue_url.to_string(),
                "--results-url".to_owned(),
                self.results_url.to_string(),
                "--health-port".to_owned(),
                health_port.to_string(),
            ],
            ..LaunchSpec::default()
        };
        let process = LocalProcessApp::new(
            "job-worker",
            launch,
            NodeEndpoints::from_api_port(health_port),
            client,
        )
        .with_readiness(|_, client| async move { client.wait_ready().await });

        ctx.deploy(process).await
    }
}

#[derive(Clone)]
pub struct WorkerClient {
    health_url: Url,
    client: reqwest::Client,
}

impl WorkerClient {
    fn new(port: u16) -> Result<Self, DynError> {
        Ok(Self {
            health_url: Url::parse(&format!("http://127.0.0.1:{port}/health/ready"))?,
            client: reqwest::Client::new(),
        })
    }

    async fn wait_ready(&self) -> Result<(), DynError> {
        let deadline = Instant::now() + Duration::from_secs(10);

        while Instant::now() < deadline {
            if self
                .client
                .get(self.health_url.clone())
                .send()
                .await
                .is_ok_and(|response| response.status().is_success())
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }

        Err("job worker did not become ready".into())
    }
}

fn worker_binary_provider() -> FallbackBinaryProvider {
    let workspace = workspace_root();
    let providers: [BinaryProviderRef; 2] = [
        std::sync::Arc::new(EnvBinaryProvider::new("MULTI_APP_JOB_WORKER_BIN")),
        std::sync::Arc::new(BuildBinaryProvider {
            command: BuildCommand::new("cargo").with_args([
                "build",
                "-p",
                "multi-app-job-worker",
                "--bin",
                "multi-app-job-worker",
            ]),
            output_path: PathBuf::from(format!(
                "target/debug/multi-app-job-worker{}",
                std::env::consts::EXE_SUFFIX
            )),
            working_dir: Some(workspace),
            lock_dir: None,
        }),
    ];

    FallbackBinaryProvider::new(providers)
}

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

#[derive(Clone)]
pub struct EnqueueJobs {
    count: usize,
}

impl EnqueueJobs {
    #[must_use]
    pub const fn new(count: usize) -> Self {
        Self { count }
    }
}

#[async_trait]
impl Workload<AppHostEnv> for EnqueueJobs {
    fn name(&self) -> &str {
        "enqueue_jobs"
    }

    async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let stack = ctx.require_app::<JobStackHandle>()?;
        let queue = stack
            .queue
            .first_client()
            .ok_or("queue cluster has no clients")?;

        for index in 0..self.count {
            let response: EnqueueResponse = queue
                .post(
                    "/queue/enqueue",
                    &EnqueueRequest {
                        payload: job_key(index),
                    },
                )
                .await?;
            if !response.accepted {
                return Err(format!("queue rejected job {index}").into());
            }
        }

        info!(jobs = self.count, "jobs enqueued");
        Ok(())
    }
}

#[derive(Clone)]
pub struct AllJobsCompleted {
    count: usize,
    timeout: Duration,
}

impl AllJobsCompleted {
    #[must_use]
    pub const fn new(count: usize) -> Self {
        Self {
            count,
            timeout: Duration::from_secs(20),
        }
    }

    #[must_use]
    pub const fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }
}

#[async_trait]
impl Expectation<AppHostEnv> for AllJobsCompleted {
    fn name(&self) -> &str {
        "all_jobs_completed"
    }

    async fn evaluate(&mut self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let stack = ctx.require_app::<JobStackHandle>()?;
        let clients = stack.results.clients();
        if clients.is_empty() {
            return Err("result store has no clients".into());
        }

        let deadline = Instant::now() + self.timeout;
        while Instant::now() < deadline {
            if all_results_are_visible(&clients, self.count).await? {
                if !stack.worker.is_running().await {
                    return Err("job worker stopped before evaluation".into());
                }
                info!(jobs = self.count, "all job results converged");
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }

        Err(format!("job results did not converge within {:?}", self.timeout).into())
    }
}

async fn all_results_are_visible(
    clients: &[kvstore_node::KvHttpClient],
    count: usize,
) -> Result<bool, DynError> {
    for index in 0..count {
        for client in clients {
            let response: KvGetResponse = client.get(&format!("/kv/{}", job_key(index))).await?;
            if response
                .record
                .as_ref()
                .is_none_or(|record| record.value != "completed")
            {
                return Ok(false);
            }
        }
    }

    Ok(true)
}

fn job_key(index: usize) -> String {
    format!("job-{index}")
}

#[derive(Serialize)]
struct EnqueueRequest {
    payload: String,
}

#[derive(Deserialize)]
struct EnqueueResponse {
    accepted: bool,
}

#[derive(Deserialize)]
struct KvGetResponse {
    record: Option<ValueRecord>,
}

#[derive(Deserialize)]
struct ValueRecord {
    value: String,
}

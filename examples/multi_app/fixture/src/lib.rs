use std::{env, path::PathBuf, time::Duration};

use async_trait::async_trait;
use kvstore_node::KvHttpClient;
use kvstore_runtime_ext::{KvEnv, KvLocalApp, KvTopology};
use queue_node::QueueHttpClient;
use queue_runtime_ext::{QueueEnv, QueueLocalApp, QueueTopology};
use reqwest::Url;
use serde::{Deserialize, Serialize};
use testing_framework_app::{
    AppDeployment, AppHostEnv, AppRunContextExt, DeployContext, LocalProcessApp, LocalProcessHandle,
};
use testing_framework_container::{
    ContainerFile, ContainerPort, ContainerReadiness, ContainerServiceHandle, ContainerServiceSpec,
    ContainerStackHandle, ContainerStackProvisioner, ContainerStackRequest,
};
use testing_framework_core::{
    cfgsync::StaticArtifactRenderer,
    scenario::{DynError, Expectation, RunContext, Workload},
};
use testing_framework_runner_local::{
    BinaryProvider, BinaryProviderRef, BuildBinaryProvider, BuildCommand, EnvBinaryProvider,
    FallbackBinaryProvider, LaunchSpec, NodeEndpoints, allocate_available_port,
};
use tokio::time::Instant;
use tracing::info;

#[derive(Clone)]
pub struct JobStackApp {
    queue_nodes: usize,
    result_nodes: usize,
}

impl JobStackApp {
    #[must_use]
    pub fn new() -> Self {
        Self::with_cluster_sizes(2, 2)
    }

    #[must_use]
    pub fn with_cluster_sizes(queue_nodes: usize, result_nodes: usize) -> Self {
        Self {
            queue_nodes,
            result_nodes,
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
        let queue = ctx
            .deploy_and_expose(QueueLocalApp::nodes(self.queue_nodes))
            .await?;
        let results = ctx
            .deploy_and_expose(KvLocalApp::nodes(self.result_nodes))
            .await?;

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
            queue_clients: queue.clients(),
            result_clients: results.clients(),
            worker: worker.client(),
            worker_control: WorkerControl::Local(worker),
        };
        ctx.expose(stack.clone())?;

        Ok(stack)
    }
}

#[derive(Clone)]
pub struct JobStackHandle {
    queue_clients: Vec<QueueHttpClient>,
    result_clients: Vec<KvHttpClient>,
    worker: WorkerClient,
    worker_control: WorkerControl,
}

#[derive(Clone)]
enum WorkerControl {
    Local(LocalProcessHandle<WorkerClient>),
    Service(ContainerServiceHandle),
}

impl JobStackHandle {
    fn first_queue_client(&self) -> Option<QueueHttpClient> {
        self.queue_clients.first().cloned()
    }

    fn result_clients(&self) -> &[KvHttpClient] {
        &self.result_clients
    }

    /// Returns the job worker client.
    #[must_use]
    pub const fn worker(&self) -> &WorkerClient {
        &self.worker
    }

    /// Returns whether the worker is running on the selected backend.
    pub async fn worker_is_running(&self) -> Result<bool, DynError> {
        match &self.worker_control {
            WorkerControl::Local(worker) => Ok(worker.is_running().await),
            WorkerControl::Service(worker) => worker.is_running().await,
        }
    }

    /// Stops only the worker.
    pub async fn stop_worker(&self) -> Result<(), DynError> {
        match &self.worker_control {
            WorkerControl::Local(worker) => worker.stop().await,
            WorkerControl::Service(worker) => worker.stop().await,
        }
    }

    /// Starts only the worker and waits for readiness.
    pub async fn start_worker(&self) -> Result<(), DynError> {
        match &self.worker_control {
            WorkerControl::Local(worker) => worker.start().await,
            WorkerControl::Service(worker) => worker.start().await,
        }
    }

    /// Restarts only the worker and waits for readiness.
    pub async fn restart_worker(&self) -> Result<(), DynError> {
        match &self.worker_control {
            WorkerControl::Local(worker) => worker.restart().await,
            WorkerControl::Service(worker) => worker.restart().await,
        }
    }

    /// Waits for the worker readiness condition.
    pub async fn wait_worker_ready(&self) -> Result<(), DynError> {
        match &self.worker_control {
            WorkerControl::Local(worker) => worker.client().wait_ready().await,
            WorkerControl::Service(worker) => worker.wait_ready().await,
        }
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
    type Handle = testing_framework_app::LocalProcessHandle<WorkerClient>;

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
        Self::from_host_port("127.0.0.1", port)
    }

    fn from_host_port(host: &str, port: u16) -> Result<Self, DynError> {
        Ok(Self {
            health_url: Url::parse(&format!("http://{host}:{port}/health/ready"))?,
            client: reqwest::Client::new(),
        })
    }

    /// Waits until the worker reports ready.
    pub async fn wait_ready(&self) -> Result<(), DynError> {
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

/// Containerized form of the job stack, portable across service-stack
/// provisioners such as Compose and Kubernetes.
#[derive(Clone)]
pub struct JobStackContainerApp {
    queue_nodes: usize,
    result_nodes: usize,
}

impl JobStackContainerApp {
    /// Creates the default two-cluster job stack.
    #[must_use]
    pub const fn new() -> Self {
        Self::with_cluster_sizes(2, 2)
    }

    /// Selects queue and result-store replica counts.
    #[must_use]
    pub const fn with_cluster_sizes(queue_nodes: usize, result_nodes: usize) -> Self {
        Self {
            queue_nodes,
            result_nodes,
        }
    }
}

impl Default for JobStackContainerApp {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl<P> AppDeployment<AppHostEnv, P> for JobStackContainerApp
where
    P: ContainerStackProvisioner,
{
    type Handle = JobStackHandle;

    async fn deploy(
        self,
        ctx: &mut DeployContext<AppHostEnv, P>,
    ) -> Result<Self::Handle, DynError> {
        let queue_topology = QueueTopology::new(self.queue_nodes);
        let result_topology = KvTopology::new(self.result_nodes);

        let mut services = ctx
            .deploy_container_stack(cluster_service_request::<QueueEnv>(
                "multi-app-queue",
                "queue-node",
                &queue_topology,
                image("QUEUE_IMAGE", "queue-node:local"),
                "/usr/local/bin/queue-node",
                "/etc/queue/config.yaml",
            )?)
            .await?;
        services.merge(
            ctx.deploy_container_stack(cluster_service_request::<KvEnv>(
                "multi-app-results",
                "result-node",
                &result_topology,
                image("KVSTORE_IMAGE", "kvstore-node:local"),
                "/usr/local/bin/kvstore-node",
                "/etc/kvstore/config.yaml",
            )?)
            .await?,
        )?;

        let queue_url = internal_service_url(&services, "queue-node-0", "api")?;
        let results_url = internal_service_url(&services, "result-node-0", "api")?;
        services.merge(
            ctx.deploy_container_stack(worker_service_request(queue_url, results_url))
                .await?,
        )?;

        let queue_clients = service_clients::<QueueHttpClient>(
            &services,
            "queue-node",
            self.queue_nodes,
            QueueHttpClient::new,
        )?;
        let result_clients = service_clients::<KvHttpClient>(
            &services,
            "result-node",
            self.result_nodes,
            KvHttpClient::new,
        )?;
        let worker_endpoint = services
            .require_service("job-worker")?
            .endpoint("health")
            .ok_or("job worker health endpoint is missing")?;
        let worker = WorkerClient::from_host_port(worker_endpoint.host(), worker_endpoint.port())?;

        let worker_control =
            WorkerControl::Service(services.require_service("job-worker")?.clone());
        let stack = JobStackHandle {
            queue_clients,
            result_clients,
            worker,
            worker_control,
        };
        ctx.expose(services)?;
        ctx.expose(stack.clone())?;
        Ok(stack)
    }
}

fn cluster_service_request<E>(
    name: &str,
    prefix: &str,
    topology: &<E as StaticArtifactRenderer>::Deployment,
    image: String,
    binary: &str,
    config_path: &str,
) -> Result<ContainerStackRequest, DynError>
where
    E: StaticArtifactRenderer,
    <E as StaticArtifactRenderer>::Deployment:
        testing_framework_core::topology::DeploymentDescriptor,
{
    Ok(
        cluster_services::<E>(prefix, topology, image, binary, config_path)?
            .into_iter()
            .fold(ContainerStackRequest::new(name), |request, service| {
                request.with_service(service)
            }),
    )
}

fn worker_service_request(queue_url: Url, results_url: Url) -> ContainerStackRequest {
    let worker = ContainerServiceSpec::new(
        "job-worker",
        image("MULTI_APP_JOB_WORKER_IMAGE", "multi-app-job-worker:local"),
    )
    .with_command(vec![
        "/usr/local/bin/multi-app-job-worker".to_owned(),
        "--queue-url".to_owned(),
        queue_url.to_string(),
        "--results-url".to_owned(),
        results_url.to_string(),
        "--health-port".to_owned(),
        "8080".to_owned(),
    ])
    .with_env("RUST_LOG", "multi_app_job_worker=info")
    .with_port(ContainerPort::new("health", 8080).published())
    .with_readiness(
        ContainerReadiness::http("health", "/health/ready").with_timeout(Duration::from_secs(15)),
    );
    ContainerStackRequest::new("multi-app-worker").with_service(worker)
}

fn internal_service_url(
    services: &ContainerStackHandle,
    service: &str,
    port: &str,
) -> Result<Url, DynError> {
    let endpoint = services
        .require_service(service)?
        .internal_endpoint(port)
        .ok_or_else(|| format!("service '{service}' internal endpoint '{port}' is missing"))?;
    Ok(Url::parse(&format!("http://{}", endpoint.authority()))?)
}

fn cluster_services<E>(
    prefix: &str,
    topology: &<E as StaticArtifactRenderer>::Deployment,
    image: String,
    binary: &str,
    config_path: &str,
) -> Result<Vec<ContainerServiceSpec>, DynError>
where
    E: StaticArtifactRenderer,
    <E as StaticArtifactRenderer>::Deployment:
        testing_framework_core::topology::DeploymentDescriptor,
{
    let node_count = <<E as StaticArtifactRenderer>::Deployment as testing_framework_core::topology::DeploymentDescriptor>::node_count(topology);
    let hostnames = (0..node_count)
        .map(|index| format!("{prefix}-{index}"))
        .collect::<Vec<_>>();

    (0..node_count)
        .map(|index| {
            let mut config = E::build_node_config(topology, index)?;
            E::rewrite_for_hostnames(topology, index, &hostnames, &mut config)?;
            let rendered = E::serialize_node_config(&config)?;
            Ok(ContainerServiceSpec::new(&hostnames[index], image.clone())
                .with_command([binary, "--config", config_path])
                .with_env("RUST_LOG", "info")
                .with_port(ContainerPort::new("api", 8080).published())
                .with_port(ContainerPort::new("testing", 8081).published())
                .with_file(ContainerFile::new("config.yaml", config_path, rendered))
                .with_readiness(ContainerReadiness::http("api", "/health/ready")))
        })
        .collect()
}

fn service_clients<C>(
    services: &ContainerStackHandle,
    prefix: &str,
    count: usize,
    build: impl Fn(Url) -> C,
) -> Result<Vec<C>, DynError> {
    (0..count)
        .map(|index| {
            let name = format!("{prefix}-{index}");
            let endpoint = services
                .require_service(&name)?
                .endpoint("api")
                .ok_or_else(|| format!("service '{name}' API endpoint is missing"))?;
            let url = Url::parse(&format!("http://{}", endpoint.authority()))?;
            Ok(build(url))
        })
        .collect()
}

fn image(variable: &str, default: &str) -> String {
    env::var(variable).unwrap_or_else(|_| default.to_owned())
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
            .first_queue_client()
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
        let clients = stack.result_clients();
        if clients.is_empty() {
            return Err("result store has no clients".into());
        }

        let deadline = Instant::now() + self.timeout;
        while Instant::now() < deadline {
            if all_results_are_visible(clients, self.count).await? {
                if !stack.worker_is_running().await? {
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

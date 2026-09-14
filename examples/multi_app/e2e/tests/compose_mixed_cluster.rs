use std::time::Duration;

use async_trait::async_trait;
use queue_node::QueueHttpClient;
use queue_runtime_ext::{QueueEnv, QueueTopology};
use serde::{Deserialize, Serialize};
use testing_framework_app::{
    AppDeployment, AppHost, AppHostDeployer, AppHostEnv, AppRunContextExt, AppScenarioBuilderExt,
    DeployContext,
};
use testing_framework_container::{
    ContainerPort, ContainerReadiness, ContainerServiceHandle, ContainerServiceSpec,
    ContainerStackRequest,
};
use testing_framework_core::scenario::{
    ClusterControlRequest, ClusterHandle, ClusterProvisioner as _, ClusterRequest, DynError,
    RunContext, Workload,
};
use testing_framework_runner_compose::ComposeProvisioner;

const PROBE_SERVICE: &str = "cluster-probe";
const DUAL_PROBE_SERVICE: &str = "dual-cluster-probe";
const QUEUE_INTERNAL_API_PORT: u16 = 8080;

#[tokio::test]
async fn container_stack_reaches_cluster_nodes_over_compose_dns() -> Result<(), DynError> {
    let mut scenario = AppHost::scenario()
        .with_app_using(MixedClusterApp, ComposeProvisioner::default())
        .with_run_duration(Duration::from_secs(3))
        .with_workload(EnqueueThroughSharedCluster)
        .build()?;

    let runner = AppHostDeployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;

    Ok(())
}

#[tokio::test]
async fn cluster_extends_a_container_established_session() -> Result<(), DynError> {
    let mut scenario = AppHost::scenario()
        .with_app_using(ReversedMixedApp, ComposeProvisioner::default())
        .with_run_duration(Duration::from_secs(3))
        .with_workload(EnqueueThroughSharedCluster)
        .build()?;

    let runner = AppHostDeployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;

    Ok(())
}

#[tokio::test]
async fn two_named_clusters_share_one_compose_session() -> Result<(), DynError> {
    let mut scenario = AppHost::scenario()
        .with_app_using(DualNamedClusterApp, ComposeProvisioner::default())
        .with_run_duration(Duration::from_secs(3))
        .with_workload(EnqueueThroughBothClusters)
        .build()?;

    let runner = AppHostDeployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;

    Ok(())
}

#[tokio::test]
async fn named_and_unnamed_clusters_share_one_session_across_a_restart() -> Result<(), DynError> {
    let mut scenario = AppHost::scenario()
        .with_app_using(NamedAndUnnamedClusterApp, ComposeProvisioner::default())
        .with_run_duration(Duration::from_secs(3))
        .with_workload(RestartNamedThenEnqueueBoth)
        .build()?;

    let runner = AppHostDeployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;

    Ok(())
}

#[tokio::test]
async fn partial_teardown_leaves_the_sibling_cluster_running() -> Result<(), DynError> {
    let provisioner = ComposeProvisioner::default();

    let mut gamma = provisioner
        .provision_cluster(
            ClusterRequest::<QueueEnv>::managed(QueueTopology::new(1))
                .with_name("gamma")
                .with_control(ClusterControlRequest::Full),
        )
        .await?;
    let gamma_handle = gamma.handle();
    let project = gamma
        .attachment()
        .ok_or("gamma cluster recorded no attachment")?
        .compose_project()
        .ok_or("gamma cluster recorded no compose project")?
        .to_owned();

    let mut delta = provisioner
        .provision_cluster(
            ClusterRequest::<QueueEnv>::managed(QueueTopology::new(1)).with_name("delta"),
        )
        .await?;
    let delta_project = delta
        .attachment()
        .ok_or("delta cluster recorded no attachment")?
        .compose_project()
        .ok_or("delta cluster recorded no compose project")?
        .to_owned();

    let result = async {
        assert!(
            compose_project_containers(&delta_project)
                .await?
                .iter()
                .any(|name| name.contains("delta-node-0")),
            "delta cluster must be running before its partial teardown"
        );

        delta
            .take_cleanup()
            .ok_or("delta cluster must own a cleanup guard")?
            .cleanup();

        let delta_leftovers = compose_project_containers(&delta_project).await?;
        assert!(
            delta_leftovers.is_empty(),
            "partial teardown must remove the delta cluster's containers, got {delta_leftovers:?}"
        );
        let survivors = compose_project_containers(&project).await?;
        assert!(
            survivors.iter().any(|name| name.contains("gamma-node-0")),
            "partial teardown must leave the sibling cluster running, got {survivors:?}"
        );

        enqueue_expecting_acceptance(&gamma_handle, "gamma-after-partial-teardown").await?;

        gamma_handle.restart_node("gamma-node-0").await?;
        let restarted = wait_for_node_after_restart(&project, "gamma-node-0").await?;
        enqueue_with_client(&restarted, "gamma-after-restart").await?;

        Ok::<(), DynError>(())
    }
    .await;

    if let Some(cleanup) = gamma.take_cleanup() {
        cleanup.cleanup();
    }
    result?;

    let leftovers = compose_project_containers(&project).await?;
    assert!(
        leftovers.is_empty(),
        "session teardown must remove every project container, got {leftovers:?}"
    );

    Ok(())
}

async fn enqueue_expecting_acceptance(
    cluster: &ClusterHandle<QueueEnv>,
    payload: &str,
) -> Result<(), DynError> {
    let client = cluster.first_client().ok_or("cluster has no clients")?;
    enqueue_with_client(&client, payload).await
}

async fn enqueue_with_client(client: &QueueHttpClient, payload: &str) -> Result<(), DynError> {
    let response: EnqueueResponse = client
        .post(
            "/queue/enqueue",
            &EnqueueRequest {
                payload: payload.to_owned(),
            },
        )
        .await?;
    assert!(response.accepted, "queue rejected the enqueued job");
    Ok(())
}

/// Rebuilds a node client after a compose restart: the ephemeral loopback
/// port bindings are reassigned when the container restarts, so the old host
/// port dies and the new one has to be discovered through `docker port`.
async fn wait_for_node_after_restart(
    project: &str,
    service: &str,
) -> Result<QueueHttpClient, DynError> {
    let container = format!("{project}-{service}-1");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    loop {
        if let Some(port) = container_host_port(&container, QUEUE_INTERNAL_API_PORT).await? {
            let client = QueueHttpClient::new(format!("http://127.0.0.1:{port}/").parse()?);
            if let Ok(health) = client.get::<HealthResponse>("/health/ready").await
                && health.status == "ready"
            {
                return Ok(client);
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(format!("node '{service}' did not become ready after its restart").into());
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

async fn container_host_port(container: &str, port: u16) -> Result<Option<u16>, DynError> {
    let output = tokio::process::Command::new("docker")
        .args(["port", container, &format!("{port}/tcp")])
        .output()
        .await?;
    if !output.status.success() {
        return Ok(None);
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .find_map(|line| line.rsplit(':').next()?.trim().parse().ok()))
}

#[derive(Deserialize)]
struct HealthResponse {
    status: String,
}

async fn compose_project_containers(project: &str) -> Result<Vec<String>, DynError> {
    let output = tokio::process::Command::new("docker")
        .args([
            "ps",
            "-a",
            "--filter",
            &format!("label=com.docker.compose.project={project}"),
            "--format",
            "{{.Names}}",
        ])
        .output()
        .await?;
    if !output.status.success() {
        return Err(format!(
            "docker ps failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )
        .into());
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::to_owned)
        .collect())
}

#[derive(Clone)]
struct MixedHandle {
    queue: ClusterHandle<QueueEnv>,
    probe: ContainerServiceHandle,
}

#[derive(Clone)]
struct MixedClusterApp;

#[async_trait]
impl AppDeployment<AppHostEnv, ComposeProvisioner> for MixedClusterApp {
    type Handle = MixedHandle;

    async fn deploy(
        self,
        ctx: &mut DeployContext<AppHostEnv, ComposeProvisioner>,
    ) -> Result<Self::Handle, DynError> {
        let queue = ctx
            .deploy_cluster(ClusterRequest::<QueueEnv>::managed(QueueTopology::new(2)))
            .await?;
        let node_service = queue
            .node_names()
            .first()
            .cloned()
            .ok_or("queue cluster reported no node services")?;

        let probe = ctx
            .deploy_container_stack(probe_request(&node_service))
            .await?;
        let probe = probe.require_service(PROBE_SERVICE)?.clone();

        let handle = MixedHandle { queue, probe };
        ctx.expose(handle.clone())?;
        Ok(handle)
    }
}

#[derive(Clone)]
struct ReversedMixedApp;

#[async_trait]
impl AppDeployment<AppHostEnv, ComposeProvisioner> for ReversedMixedApp {
    type Handle = MixedHandle;

    async fn deploy(
        self,
        ctx: &mut DeployContext<AppHostEnv, ComposeProvisioner>,
    ) -> Result<Self::Handle, DynError> {
        let sidecar = ctx.deploy_container_stack(sidecar_request()).await?;
        let sidecar = sidecar.require_service("sidecar")?.clone();

        let queue = ctx
            .deploy_cluster(ClusterRequest::<QueueEnv>::managed(QueueTopology::new(2)))
            .await?;
        assert!(
            sidecar.is_running().await?,
            "cluster extension must not disturb running container services"
        );
        let node_service = queue
            .node_names()
            .first()
            .cloned()
            .ok_or("queue cluster reported no node services")?;

        let probe = ctx
            .deploy_container_stack(probe_request(&node_service))
            .await?;
        let probe = probe.require_service(PROBE_SERVICE)?.clone();

        let handle = MixedHandle { queue, probe };
        ctx.expose(handle.clone())?;
        Ok(handle)
    }
}

fn sidecar_request() -> ContainerStackRequest {
    let sidecar = ContainerServiceSpec::new("sidecar", "python:3-alpine")
        .with_command(["python3", "-c", &serve_script()])
        .with_port(ContainerPort::new("health", 8080).published())
        .with_readiness(
            ContainerReadiness::http("health", "/").with_timeout(Duration::from_secs(30)),
        );
    ContainerStackRequest::new("mixed-sidecar").with_service(sidecar)
}

fn serve_script() -> String {
    r#"
from http.server import BaseHTTPRequestHandler, HTTPServer

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"sidecar ready")

HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
"#
    .to_owned()
}

fn probe_request(node_service: &str) -> ContainerStackRequest {
    let readiness_url = format!("http://{node_service}:{QUEUE_INTERNAL_API_PORT}/health/ready");
    let probe = ContainerServiceSpec::new(PROBE_SERVICE, "python:3-alpine")
        .with_command(["python3", "-c", &probe_script(&readiness_url)])
        .with_port(ContainerPort::new("health", 8080).published())
        .with_readiness(
            ContainerReadiness::http("health", "/").with_timeout(Duration::from_secs(60)),
        );
    ContainerStackRequest::new("mixed-probe").with_service(probe)
}

fn probe_script(target: &str) -> String {
    format!(
        r#"
import time, urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer

deadline = time.time() + 60
while True:
    try:
        with urllib.request.urlopen("{target}", timeout=2) as response:
            if response.status == 200:
                break
    except Exception:
        pass
    if time.time() > deadline:
        raise SystemExit(1)
    time.sleep(0.5)

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"reached cluster node over compose dns")

HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
"#
    )
}

#[derive(Clone)]
struct EnqueueThroughSharedCluster;

#[async_trait]
impl Workload<AppHostEnv> for EnqueueThroughSharedCluster {
    fn name(&self) -> &str {
        "enqueue_through_shared_cluster"
    }

    async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let handle = ctx.require_app::<MixedHandle>()?;

        assert!(
            handle.probe.is_running().await?,
            "probe must keep serving after reaching the cluster node internally"
        );

        let client = handle
            .queue
            .first_client()
            .ok_or("queue cluster has no clients")?;
        let response: EnqueueResponse = client
            .post(
                "/queue/enqueue",
                &EnqueueRequest {
                    payload: "shared-session-job".to_owned(),
                },
            )
            .await?;
        assert!(response.accepted, "queue rejected the enqueued job");

        Ok(())
    }
}

#[derive(Serialize)]
struct EnqueueRequest {
    payload: String,
}

#[derive(Deserialize)]
struct EnqueueResponse {
    accepted: bool,
}

#[derive(Clone)]
struct DualClusterHandle {
    alpha: ClusterHandle<QueueEnv>,
    beta: ClusterHandle<QueueEnv>,
    probe: ContainerServiceHandle,
}

#[derive(Clone)]
struct DualNamedClusterApp;

#[async_trait]
impl AppDeployment<AppHostEnv, ComposeProvisioner> for DualNamedClusterApp {
    type Handle = DualClusterHandle;

    async fn deploy(
        self,
        ctx: &mut DeployContext<AppHostEnv, ComposeProvisioner>,
    ) -> Result<Self::Handle, DynError> {
        let alpha = ctx
            .deploy_cluster(
                ClusterRequest::<QueueEnv>::managed(QueueTopology::new(1))
                    .with_name("alpha")
                    .with_control(ClusterControlRequest::Full),
            )
            .await?;
        let beta = ctx
            .deploy_cluster(
                ClusterRequest::<QueueEnv>::managed(QueueTopology::new(1))
                    .with_name("beta")
                    .with_control(ClusterControlRequest::Full),
            )
            .await?;

        let alpha_service = alpha
            .node_names()
            .first()
            .cloned()
            .ok_or("alpha cluster reported no node services")?;
        let beta_service = beta
            .node_names()
            .first()
            .cloned()
            .ok_or("beta cluster reported no node services")?;
        assert_eq!(
            alpha_service, "alpha-node-0",
            "named clusters must namespace their compose services"
        );
        assert_eq!(
            beta_service, "beta-node-0",
            "named clusters must namespace their compose services"
        );

        let probe = ctx
            .deploy_container_stack(dual_probe_request(&alpha_service, &beta_service))
            .await?;
        let probe = probe.require_service(DUAL_PROBE_SERVICE)?.clone();

        let handle = DualClusterHandle { alpha, beta, probe };
        ctx.expose(handle.clone())?;
        Ok(handle)
    }
}

fn dual_probe_request(alpha_service: &str, beta_service: &str) -> ContainerStackRequest {
    let alpha_url = format!("http://{alpha_service}:{QUEUE_INTERNAL_API_PORT}/health/ready");
    let beta_url = format!("http://{beta_service}:{QUEUE_INTERNAL_API_PORT}/health/ready");
    let probe = ContainerServiceSpec::new(DUAL_PROBE_SERVICE, "python:3-alpine")
        .with_command(["python3", "-c", &dual_probe_script(&alpha_url, &beta_url)])
        .with_port(ContainerPort::new("health", 8080).published())
        .with_readiness(
            ContainerReadiness::http("health", "/").with_timeout(Duration::from_secs(60)),
        );
    ContainerStackRequest::new("dual-cluster-probe").with_service(probe)
}

fn dual_probe_script(alpha_url: &str, beta_url: &str) -> String {
    format!(
        r#"
import time, urllib.request
from http.server import BaseHTTPRequestHandler, HTTPServer

targets = ["{alpha_url}", "{beta_url}"]
deadline = time.time() + 60
for target in targets:
    while True:
        try:
            with urllib.request.urlopen(target, timeout=2) as response:
                if response.status == 200:
                    break
        except Exception:
            pass
        if time.time() > deadline:
            raise SystemExit(1)
        time.sleep(0.5)

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        self.send_response(200)
        self.end_headers()
        self.wfile.write(b"reached both named clusters over compose dns")

HTTPServer(("0.0.0.0", 8080), Handler).serve_forever()
"#
    )
}

#[derive(Clone)]
struct NamedUnnamedHandle {
    named: ClusterHandle<QueueEnv>,
    unnamed: ClusterHandle<QueueEnv>,
    probe: ContainerServiceHandle,
}

#[derive(Clone)]
struct NamedAndUnnamedClusterApp;

#[async_trait]
impl AppDeployment<AppHostEnv, ComposeProvisioner> for NamedAndUnnamedClusterApp {
    type Handle = NamedUnnamedHandle;

    async fn deploy(
        self,
        ctx: &mut DeployContext<AppHostEnv, ComposeProvisioner>,
    ) -> Result<Self::Handle, DynError> {
        let named = ctx
            .deploy_cluster(
                ClusterRequest::<QueueEnv>::managed(QueueTopology::new(1))
                    .with_name("alpha")
                    .with_control(ClusterControlRequest::Full),
            )
            .await?;
        let unnamed = ctx
            .deploy_cluster(ClusterRequest::<QueueEnv>::managed(QueueTopology::new(1)))
            .await?;

        let named_service = named
            .node_names()
            .first()
            .cloned()
            .ok_or("named cluster reported no node services")?;
        let unnamed_service = unnamed
            .node_names()
            .first()
            .cloned()
            .ok_or("unnamed cluster reported no node services")?;
        assert_eq!(
            named_service, "alpha-node-0",
            "the named cluster must namespace its compose services"
        );
        assert_eq!(
            unnamed_service, "node-0",
            "the unnamed cluster must keep the plain service names"
        );

        let probe = ctx
            .deploy_container_stack(dual_probe_request(&named_service, &unnamed_service))
            .await?;
        let probe = probe.require_service(DUAL_PROBE_SERVICE)?.clone();

        let handle = NamedUnnamedHandle {
            named,
            unnamed,
            probe,
        };
        ctx.expose(handle.clone())?;
        Ok(handle)
    }
}

#[derive(Clone)]
struct RestartNamedThenEnqueueBoth;

#[async_trait]
impl Workload<AppHostEnv> for RestartNamedThenEnqueueBoth {
    fn name(&self) -> &str {
        "restart_named_then_enqueue_both"
    }

    async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let handle = ctx.require_app::<NamedUnnamedHandle>()?;

        assert!(
            handle.probe.is_running().await?,
            "probe must keep serving after reaching both clusters over compose dns"
        );

        let project = handle
            .named
            .attachment()
            .ok_or("named cluster recorded no attachment")?
            .compose_project()
            .ok_or("named cluster recorded no compose project")?
            .to_owned();

        handle.named.restart_node("alpha-node-0").await?;
        let restarted = wait_for_node_after_restart(&project, "alpha-node-0").await?;

        enqueue_with_client(&restarted, "alpha-after-restart-job").await?;
        enqueue_expecting_acceptance(&handle.unnamed, "unnamed-survivor-job").await?;

        assert!(
            handle.probe.is_running().await?,
            "probe must keep serving after the named-cluster restart"
        );

        Ok(())
    }
}

#[derive(Clone)]
struct EnqueueThroughBothClusters;

#[async_trait]
impl Workload<AppHostEnv> for EnqueueThroughBothClusters {
    fn name(&self) -> &str {
        "enqueue_through_both_named_clusters"
    }

    async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let handle = ctx.require_app::<DualClusterHandle>()?;

        assert!(
            handle.probe.is_running().await?,
            "probe must keep serving after reaching both cluster nodes internally"
        );

        for (cluster, payload) in [
            (&handle.alpha, "alpha-session-job"),
            (&handle.beta, "beta-session-job"),
        ] {
            let client = cluster.first_client().ok_or("cluster has no clients")?;
            let response: EnqueueResponse = client
                .post(
                    "/queue/enqueue",
                    &EnqueueRequest {
                        payload: payload.to_owned(),
                    },
                )
                .await?;
            assert!(response.accepted, "queue rejected the enqueued job");
        }

        Ok(())
    }
}

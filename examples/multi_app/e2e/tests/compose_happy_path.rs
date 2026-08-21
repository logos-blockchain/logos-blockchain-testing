use std::{env, time::Duration};

use async_trait::async_trait;
use multi_app_fixture::{AllJobsCompleted, EnqueueJobs, JobStackContainerApp, JobStackHandle};
use testing_framework_app::{
    AppDeployment, AppHost, AppHostDeployer, AppHostEnv, AppRunContextExt, AppScenarioBuilderExt,
    DeployContext,
};
use testing_framework_container::{
    ContainerFile, ContainerPort, ContainerReadiness, ContainerServiceSpec, ContainerStackHandle,
    ContainerStackRequest,
};
use testing_framework_core::scenario::{Deployer, DynError, RunContext, Workload};
use testing_framework_runner_compose::ComposeContainerProvisioner;
use tokio::time::Instant;

const JOB_COUNT: usize = 10;

#[tokio::test]
async fn containerized_services_process_jobs_and_converge() -> Result<(), DynError> {
    let mut scenario = AppHost::scenario()
        .with_app_using(
            JobStackContainerApp::new(),
            ComposeContainerProvisioner::default(),
        )
        .with_run_duration(Duration::from_secs(10))
        .with_workload(LifecycleThenEnqueue)
        .with_expectation(AllJobsCompleted::new(JOB_COUNT))
        .build()?;

    let runner = AppHostDeployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;

    Ok(())
}

#[tokio::test]
async fn extensions_preserve_stopped_services_and_roll_back_failures() -> Result<(), DynError> {
    if env::var_os("COMPOSE_RUNNER_PRESERVE").is_some()
        || env::var_os("TESTNET_RUNNER_PRESERVE").is_some()
    {
        eprintln!("skipping rollback hardening test while Compose preservation is enabled");
        return Ok(());
    }

    let mut scenario = AppHost::scenario()
        .with_app_using(
            ExtensionHardeningApp,
            ComposeContainerProvisioner::default(),
        )
        .with_run_duration(Duration::from_secs(1))
        .build()?;

    let runner = AppHostDeployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;

    Ok(())
}

#[derive(Clone)]
struct ExtensionHardeningApp;

#[async_trait]
impl AppDeployment<AppHostEnv, ComposeContainerProvisioner> for ExtensionHardeningApp {
    type Handle = ContainerStackHandle;

    async fn deploy(
        self,
        ctx: &mut DeployContext<AppHostEnv, ComposeContainerProvisioner>,
    ) -> Result<Self::Handle, DynError> {
        let mut services = ctx
            .deploy_container_stack(service_request("worker-a", "/health/ready"))
            .await?;
        let worker_a = services.require_service("worker-a")?.clone();
        worker_a.stop().await?;

        services.merge(
            ctx.deploy_container_stack(service_request("worker-b", "/health/ready"))
                .await?,
        )?;
        assert!(!worker_a.is_running().await?);
        worker_a.start().await?;

        let failed = ctx
            .deploy_container_stack(service_request("worker-c", "/missing"))
            .await;
        assert!(failed.is_err());
        assert!(worker_a.is_running().await?);
        assert!(services.require_service("worker-b")?.is_running().await?);

        services.merge(
            ctx.deploy_container_stack(service_request("worker-c", "/health/ready"))
                .await?,
        )?;

        let crashed = ctx
            .deploy_container_stack(crashing_service_request())
            .await?;
        let crash = crashed.require_service("crash-probe")?.clone();
        let deadline = Instant::now() + Duration::from_secs(2);
        while crash.is_running().await? && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(!crash.is_running().await?);
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(!crash.is_running().await?);
        services.merge(crashed)?;

        Ok(services)
    }
}

fn service_request(name: &str, readiness_path: &str) -> ContainerStackRequest {
    let service = ContainerServiceSpec::new(name, "multi-app-job-worker:local")
        .with_command([
            "/bin/sh",
            "-c",
            "test -f /app/retry-marker && exec /usr/local/bin/multi-app-job-worker \
             --queue-url http://127.0.0.1:1 --results-url http://127.0.0.1:1 \
             --health-port 8080",
        ])
        .with_file(ContainerFile::new(
            "retry-marker",
            "/app/retry-marker",
            b"present".to_vec(),
        ))
        .with_port(ContainerPort::new("health", 8080).published())
        .with_readiness(
            ContainerReadiness::http("health", readiness_path).with_timeout(Duration::from_secs(1)),
        );
    ContainerStackRequest::new(name).with_service(service)
}

fn crashing_service_request() -> ContainerStackRequest {
    ContainerStackRequest::new("crash-probe").with_service(
        ContainerServiceSpec::new("crash-probe", "multi-app-job-worker:local")
            .with_command(["/bin/sh", "-c", "exit 17"]),
    )
}

#[derive(Clone)]
struct LifecycleThenEnqueue;

#[async_trait]
impl Workload<AppHostEnv> for LifecycleThenEnqueue {
    fn name(&self) -> &str {
        "service_lifecycle_then_enqueue"
    }

    async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let stack = ctx.require_app::<JobStackHandle>()?;

        assert!(stack.worker_is_running().await?);
        stack.stop_worker().await?;
        assert!(!stack.worker_is_running().await?);
        stack.start_worker().await?;
        assert!(stack.worker_is_running().await?);
        stack.restart_worker().await?;
        stack.wait_worker_ready().await?;

        EnqueueJobs::new(JOB_COUNT).start(ctx).await
    }
}

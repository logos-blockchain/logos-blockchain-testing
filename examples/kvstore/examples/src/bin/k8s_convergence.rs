use std::time::Duration;

use anyhow::{Context as _, Result};
use kvstore_runtime_workloads::{KvConverges, KvEnv, KvTopology, KvWriteWorkload};
use testing_framework_app::{
    AppHost, AppHostDeployError, AppHostDeployer, AppScenarioBuilderExt as _, ClusterApp,
};
use testing_framework_runner_k8s::{K8sClusterProvisioner, ManualClusterError};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = AppHost::scenario()
        .with_app_using(
            ClusterApp::<KvEnv>::new(KvTopology::new(3)),
            K8sClusterProvisioner,
        )
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            KvWriteWorkload::new()
                .operations(200)
                .key_count(20)
                .rate_per_sec(20),
        )
        .with_expectation(KvConverges::new("kv-demo", 20).timeout(Duration::from_secs(25)))
        .build()?;

    let runner = match AppHostDeployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(error) => {
            if cluster_may_be_skipped()
                && let Some(reason) = k8s_unavailable_reason(&error)
            {
                warn!("k8s unavailable ({reason}); skipping kv k8s run");
                return Ok(());
            }
            return Err(anyhow::Error::new(error)).context("deploying kv k8s stack");
        }
    };

    info!("running kv k8s convergence scenario");
    runner
        .run(&mut scenario)
        .await
        .context("running kv k8s scenario")?;

    Ok(())
}

fn cluster_may_be_skipped() -> bool {
    std::env::var("K8S_RUNNER_REQUIRE_CLUSTER").as_deref() != Ok("1")
}

fn k8s_unavailable_reason(error: &AppHostDeployError) -> Option<String> {
    let AppHostDeployError::RuntimeExtensions { source } = error else {
        return None;
    };
    match source.downcast_ref::<ManualClusterError>() {
        Some(ManualClusterError::ClientInit { source }) => Some(source.to_string()),
        Some(ManualClusterError::InstallStack { source })
            if k8s_cluster_unavailable(&source.to_string()) =>
        {
            Some(source.to_string())
        }
        _ => None,
    }
}

fn k8s_cluster_unavailable(message: &str) -> bool {
    message.contains("Unable to connect to the server")
        || message.contains("TLS handshake timeout")
        || message.contains("connection refused")
}

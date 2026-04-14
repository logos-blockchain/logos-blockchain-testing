use std::time::Duration;

use anyhow::{Context as _, Result};
use kvstore_runtime_ext::KvK8sDeployer;
use kvstore_runtime_workloads::{
    KvBuilderExt, KvConverges, KvScenarioBuilder, KvTopology, KvWriteWorkload,
};
use testing_framework_core::scenario::Deployer;
use testing_framework_runner_k8s::K8sRunnerError;
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = KvScenarioBuilder::deployment_with(|_| KvTopology::new(3))
        .with_run_duration(Duration::from_secs(30))
        .with_workload(
            KvWriteWorkload::new()
                .operations(200)
                .key_count(20)
                .rate_per_sec(20),
        )
        .with_expectation(KvConverges::new("kv-demo", 20).timeout(Duration::from_secs(25)))
        .build()?;

    let deployer = KvK8sDeployer::new();
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(K8sRunnerError::ClientInit { source }) => {
            warn!("k8s unavailable ({source}); skipping kv k8s run");
            return Ok(());
        }
        Err(K8sRunnerError::InstallStack { source })
            if k8s_cluster_unavailable(&source.to_string()) =>
        {
            warn!("k8s unavailable ({source}); skipping kv k8s run");
            return Ok(());
        }
        Err(error) => return Err(anyhow::Error::new(error)).context("deploying kv k8s stack"),
    };

    info!("running kv k8s convergence scenario");
    runner
        .run(&mut scenario)
        .await
        .context("running kv k8s scenario")?;

    Ok(())
}

fn k8s_cluster_unavailable(message: &str) -> bool {
    message.contains("Unable to connect to the server")
        || message.contains("TLS handshake timeout")
        || message.contains("connection refused")
}

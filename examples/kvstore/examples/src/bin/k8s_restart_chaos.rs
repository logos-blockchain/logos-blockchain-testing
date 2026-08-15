//! Verifies managed k8s node control end-to-end: deploys a three-node kvstore
//! cluster, restarts two nodes mid-write-load through the scenario's node
//! control handle, and expects all keys to converge across the cluster.

use std::time::Duration;

use anyhow::{Context as _, Result};
use async_trait::async_trait;
use kvstore_runtime_ext::KvK8sDeployer;
use kvstore_runtime_workloads::{
    KvBuilderExt, KvConverges, KvEnv, KvScenarioBuilder, KvTopology, KvWriteWorkload,
};
use testing_framework_core::scenario::{Deployer, DynError, RunContext, Workload};
use testing_framework_runner_k8s::K8sRunnerError;
use tracing::{info, warn};

/// Workload that restarts the given nodes one by one with a fixed delay,
/// waiting for each to become ready again before moving on.
#[derive(Clone)]
struct FixedRestartChaosWorkload {
    targets: Vec<String>,
    delay: Duration,
}

impl FixedRestartChaosWorkload {
    fn new(targets: Vec<String>, delay: Duration) -> Self {
        Self { targets, delay }
    }
}

#[async_trait]
impl Workload<KvEnv> for FixedRestartChaosWorkload {
    fn name(&self) -> &str {
        "fixed_restart_chaos"
    }

    async fn start(&self, ctx: &RunContext<KvEnv>) -> Result<(), DynError> {
        let Some(control) = ctx.node_control() else {
            return Err("fixed restart chaos requires node control".into());
        };

        for target in &self.targets {
            tokio::time::sleep(self.delay).await;
            info!(%target, "restarting node via managed k8s node control");
            control.restart_node(target).await?;
            control.wait_node_ready(target).await?;
            info!(%target, "node restarted and ready");
        }

        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = KvScenarioBuilder::deployment_with(|_| KvTopology::new(3))
        .enable_node_control()
        .with_run_duration(Duration::from_secs(90))
        .with_workload(FixedRestartChaosWorkload::new(
            vec!["node-1".to_owned(), "node-2".to_owned()],
            Duration::from_secs(10),
        ))
        .with_workload(
            KvWriteWorkload::new()
                .operations(300)
                .key_count(20)
                .rate_per_sec(10)
                .key_prefix("kv-chaos"),
        )
        .with_expectation(KvConverges::new("kv-chaos", 20).timeout(Duration::from_secs(60)))
        .build()?;

    let deployer = KvK8sDeployer::new();
    let runner = match deployer.deploy(&scenario).await {
        Ok(runner) => runner,
        Err(K8sRunnerError::ClientInit { source }) => {
            warn!("k8s unavailable ({source}); skipping kv k8s restart chaos run");
            return Ok(());
        }
        Err(K8sRunnerError::InstallStack { source })
            if k8s_cluster_unavailable(&source.to_string()) =>
        {
            warn!("k8s unavailable ({source}); skipping kv k8s restart chaos run");
            return Ok(());
        }
        Err(error) => return Err(anyhow::Error::new(error)).context("deploying kv k8s stack"),
    };

    info!("running kv k8s restart chaos scenario");
    runner
        .run(&mut scenario)
        .await
        .context("running kv k8s restart chaos scenario")?;

    info!("kv k8s restart chaos scenario completed");
    Ok(())
}

/// Returns whether the error message indicates an unreachable k8s cluster,
/// in which case the run is skipped instead of failing.
fn k8s_cluster_unavailable(message: &str) -> bool {
    message.contains("Unable to connect to the server")
        || message.contains("TLS handshake timeout")
        || message.contains("connection refused")
}

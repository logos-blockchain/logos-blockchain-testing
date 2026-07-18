use std::time::Duration;

use async_trait::async_trait;
use kvstore_runtime_ext::{KvEnv, KvLocalApp};
use serde::{Deserialize, Serialize};
use testing_framework_app::{
    AppHost, AppHostEnv, AppHostLocalDeployer, AppRunContextExt, AppScenarioBuilderExt,
    LocalAppCluster,
};
use testing_framework_core::scenario::{Deployer, DynError, RunContext, Workload};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = AppHost::scenario()
        .with_app(KvLocalApp::nodes(3))
        .with_run_duration(Duration::from_secs(5))
        .with_workload(KvAppHostConvergence::new(3))
        .build()?;

    let deployer = AppHostLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;

    Ok(())
}

#[derive(Clone)]
struct KvAppHostConvergence {
    expected_nodes: usize,
}

impl KvAppHostConvergence {
    const fn new(expected_nodes: usize) -> Self {
        Self { expected_nodes }
    }
}

#[async_trait]
impl Workload<AppHostEnv> for KvAppHostConvergence {
    fn name(&self) -> &str {
        "kv_app_host_convergence"
    }

    async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let cluster = ctx.require_app::<LocalAppCluster<KvEnv>>()?;

        ensure_cluster_shape(&cluster, self.expected_nodes)?;
        put_value(&cluster, "before-restart").await?;
        cluster.restart_node("node-0").await?;
        cluster.wait_node_ready("node-0").await?;
        put_value(&cluster, "after-restart").await?;

        info!(
            nodes = self.expected_nodes,
            "kv app host cluster exposes app-local node control and clients"
        );

        Ok(())
    }
}

fn ensure_cluster_shape(
    cluster: &LocalAppCluster<KvEnv>,
    expected_nodes: usize,
) -> Result<(), DynError> {
    if cluster.node_count() != expected_nodes || cluster.clients().len() != expected_nodes {
        return Err(format!("kv app host expected {expected_nodes} nodes").into());
    }

    if cluster.node_client("node-0").is_none() {
        return Err("kv app host cannot access node-0 client".into());
    }

    if cluster.node_pid("node-0").is_none() {
        return Err("kv app host cannot access node-0 process id".into());
    }

    Ok(())
}

async fn put_value(cluster: &LocalAppCluster<KvEnv>, value: &str) -> Result<(), DynError> {
    let Some(client) = cluster.first_client() else {
        return Err("kv app host has no clients".into());
    };

    let response: KvPutResponse = client
        .put(
            "/kv/app-host-smoke",
            &KvPutRequest {
                value: value.to_owned(),
                expected_version: None,
            },
        )
        .await?;

    if !response.applied {
        return Err(format!("kv app host write was rejected for value {value}").into());
    }

    Ok(())
}

#[derive(Serialize)]
struct KvPutRequest {
    value: String,
    expected_version: Option<u64>,
}

#[derive(Deserialize)]
struct KvPutResponse {
    applied: bool,
}

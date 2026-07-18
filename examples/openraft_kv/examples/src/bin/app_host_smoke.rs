use std::time::Duration;

use async_trait::async_trait;
use openraft_kv_runtime_ext::{OpenRaftKvEnv, OpenRaftKvLocalApp};
use openraft_kv_runtime_workloads::{
    OpenRaftMembership, ensure_cluster_size, expected_kv, resolve_client_for_node, wait_for_leader,
    wait_for_membership, wait_for_replication, write_batch,
};
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
        .with_app(OpenRaftKvLocalApp::nodes(3))
        .with_run_duration(Duration::from_secs(5))
        .with_workload(OpenRaftKvAppHostSmoke::new(3))
        .build()?;

    let deployer = AppHostLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;

    Ok(())
}

#[derive(Clone)]
struct OpenRaftKvAppHostSmoke {
    expected_nodes: usize,
    timeout: Duration,
}

impl OpenRaftKvAppHostSmoke {
    const fn new(expected_nodes: usize) -> Self {
        Self {
            expected_nodes,
            timeout: Duration::from_secs(30),
        }
    }
}

#[async_trait]
impl Workload<AppHostEnv> for OpenRaftKvAppHostSmoke {
    fn name(&self) -> &str {
        "openraft_kv_app_host_smoke"
    }

    async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let cluster = ctx.require_app::<LocalAppCluster<OpenRaftKvEnv>>()?;

        ensure_cluster_shape(&cluster, self.expected_nodes)?;
        bootstrap_voter_cluster(&cluster, self.timeout).await?;

        info!(
            nodes = self.expected_nodes,
            "openraft app host cluster exposes app-local clients and process access"
        );

        Ok(())
    }
}

fn ensure_cluster_shape(
    cluster: &LocalAppCluster<OpenRaftKvEnv>,
    expected_nodes: usize,
) -> Result<(), DynError> {
    if cluster.node_count() != expected_nodes {
        return Err(format!("openraft app host expected {expected_nodes} nodes").into());
    }

    if cluster.node_client("node-0").is_none() {
        return Err("openraft app host cannot access node-0 client".into());
    }

    if cluster.node_pid("node-0").is_none() {
        return Err("openraft app host cannot access node-0 process id".into());
    }

    ensure_cluster_size(&cluster.clients(), expected_nodes)?;

    Ok(())
}

async fn bootstrap_voter_cluster(
    cluster: &LocalAppCluster<OpenRaftKvEnv>,
    timeout: Duration,
) -> Result<(), DynError> {
    let clients = cluster.clients();

    clients[0].init_self().await?;

    let initial_leader = wait_for_leader(&clients, timeout, None).await?;
    let membership = OpenRaftMembership::discover(&clients).await?;
    let leader = resolve_client_for_node(&clients, initial_leader, timeout).await?;

    for learner in membership.learner_targets(initial_leader) {
        leader
            .add_learner(learner.node_id, &learner.public_addr)
            .await?;
    }

    let voter_ids = membership.voter_ids();
    leader.change_membership(voter_ids.iter().copied()).await?;
    wait_for_membership(&clients, &voter_ids, timeout).await?;

    write_batch(&leader, "app-host-raft-key", 0, 3).await?;
    wait_for_replication(&clients, &expected_kv("app-host-raft-key", 3), timeout).await?;

    Ok(())
}

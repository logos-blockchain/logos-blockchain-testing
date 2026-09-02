use std::{sync::Arc, time::Duration};

use anyhow::{Context as _, Result, anyhow};
use openraft_kv_examples::{
    INITIAL_WRITE_BATCH, RAFT_KEY_PREFIX, SECOND_WRITE_BATCH, TOTAL_WRITES,
};
use openraft_kv_node::OpenRaftKvClient;
use openraft_kv_runtime_ext::{
    OpenRaftClusterObserver, OpenRaftKvEnv, OpenRaftKvK8sDeployer, OpenRaftKvTopology,
    OpenRaftManualClusterSourceProvider,
};
use openraft_kv_runtime_workloads::{
    OpenRaftMembership, expected_kv, wait_for_observed_leader, wait_for_observed_membership,
    wait_for_observed_replication, write_batch,
};
use testing_framework_core::observation::{ObservationHandle, ObservationRuntime};
use testing_framework_runner_k8s::{ManualCluster, ManualClusterError};
use tracing::{info, warn};

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let deployer = OpenRaftKvK8sDeployer::new();
    let cluster = match deployer
        .manual_cluster_from_descriptors(OpenRaftKvTopology::new(3))
        .await
    {
        Ok(cluster) => cluster,
        Err(ManualClusterError::ClientInit { source }) if cluster_may_be_skipped() => {
            warn!("k8s unavailable ({source}); skipping openraft k8s run");

            return Ok(());
        }
        Err(ManualClusterError::InstallStack { source })
            if cluster_may_be_skipped() && k8s_cluster_unavailable(&source.to_string()) =>
        {
            warn!("k8s unavailable ({source}); skipping openraft k8s run");

            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error)).context("creating openraft k8s cluster");
        }
    };

    run_failover(Arc::new(cluster), Duration::from_secs(40)).await
}

async fn run_failover(cluster: Arc<ManualCluster<OpenRaftKvEnv>>, timeout: Duration) -> Result<()> {
    start_cluster(cluster.as_ref()).await?;

    let observation_runtime = start_observer(Arc::clone(&cluster)).await?;
    let observer = observation_runtime.handle();

    client_for_node(cluster.as_ref(), 0)?.init_self().await?;

    let initial_leader = wait_for_observed_leader(&observer, timeout, None).await?;
    let membership = current_membership(&observer)?;

    add_learners_and_promote(
        cluster.as_ref(),
        &observer,
        initial_leader,
        &membership,
        timeout,
    )
    .await?;
    write_initial_batch(cluster.as_ref(), initial_leader).await?;

    restart_leader(cluster.as_ref(), initial_leader).await?;

    let new_leader = wait_for_observed_leader(&observer, timeout, Some(initial_leader)).await?;
    write_second_batch(cluster.as_ref(), new_leader).await?;

    let expected = expected_kv(RAFT_KEY_PREFIX, TOTAL_WRITES);
    wait_for_observed_replication(&observer, &expected, timeout).await?;

    cluster.stop_all();

    Ok(())
}

async fn start_cluster(cluster: &ManualCluster<OpenRaftKvEnv>) -> Result<()> {
    cluster.start_node("node-0").await?;
    cluster.start_node("node-1").await?;
    cluster.start_node("node-2").await?;

    cluster.wait_network_ready().await?;

    Ok(())
}

async fn start_observer(
    cluster: Arc<ManualCluster<OpenRaftKvEnv>>,
) -> Result<ObservationRuntime<OpenRaftClusterObserver>> {
    let provider = OpenRaftManualClusterSourceProvider::new(cluster, 3);

    ObservationRuntime::start(
        provider,
        OpenRaftClusterObserver,
        OpenRaftClusterObserver::config(),
    )
    .await
    .map_err(anyhow::Error::new)
    .context("starting openraft k8s observer")
}

async fn add_learners_and_promote(
    cluster: &ManualCluster<OpenRaftKvEnv>,
    observer: &ObservationHandle<OpenRaftClusterObserver>,
    leader_id: u64,
    membership: &OpenRaftMembership,
    timeout: Duration,
) -> Result<()> {
    let leader = client_for_node(cluster, leader_id)?;

    for learner in membership.learner_targets(leader_id) {
        info!(
            target = learner.node_id,
            addr = %learner.public_addr,
            "adding learner"
        );

        leader
            .add_learner(learner.node_id, &learner.public_addr)
            .await?;
    }

    let voter_ids = membership.voter_ids();
    leader.change_membership(voter_ids.iter().copied()).await?;

    wait_for_observed_membership(observer, &voter_ids, timeout).await?;

    Ok(())
}

async fn write_initial_batch(cluster: &ManualCluster<OpenRaftKvEnv>, leader_id: u64) -> Result<()> {
    let leader = client_for_node(cluster, leader_id)?;

    write_batch(&leader, RAFT_KEY_PREFIX, 0, INITIAL_WRITE_BATCH).await?;

    Ok(())
}

async fn write_second_batch(cluster: &ManualCluster<OpenRaftKvEnv>, leader_id: u64) -> Result<()> {
    let leader = client_for_node(cluster, leader_id)?;

    write_batch(
        &leader,
        RAFT_KEY_PREFIX,
        INITIAL_WRITE_BATCH,
        SECOND_WRITE_BATCH,
    )
    .await?;

    Ok(())
}

async fn restart_leader(cluster: &ManualCluster<OpenRaftKvEnv>, leader_id: u64) -> Result<()> {
    let leader_name = format!("node-{leader_id}");
    info!(%leader_name, "restarting current leader");

    cluster.restart_node(&leader_name).await?;
    cluster.wait_network_ready().await?;

    Ok(())
}

fn current_membership(
    observer: &ObservationHandle<OpenRaftClusterObserver>,
) -> Result<OpenRaftMembership> {
    let snapshot = observer
        .latest_snapshot()
        .ok_or_else(|| anyhow!("openraft observer has not produced a snapshot yet"))?;

    Ok(OpenRaftMembership::from_states(snapshot.value.states()))
}

fn client_for_node(
    cluster: &ManualCluster<OpenRaftKvEnv>,
    node_id: u64,
) -> Result<OpenRaftKvClient> {
    cluster
        .node_client(&format!("node-{node_id}"))
        .ok_or_else(|| anyhow!("node-{node_id} client missing"))
}

fn cluster_may_be_skipped() -> bool {
    std::env::var("K8S_RUNNER_REQUIRE_CLUSTER").as_deref() != Ok("1")
}

fn k8s_cluster_unavailable(message: &str) -> bool {
    message.contains("Unable to connect to the server")
        || message.contains("TLS handshake timeout")
        || message.contains("connection refused")
}

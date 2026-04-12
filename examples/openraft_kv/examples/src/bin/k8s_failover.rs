use std::time::Duration;

use anyhow::{Context as _, Result, anyhow};
use openraft_kv_examples::{
    INITIAL_WRITE_BATCH, RAFT_KEY_PREFIX, SECOND_WRITE_BATCH, TOTAL_WRITES,
};
use openraft_kv_node::OpenRaftKvClient;
use openraft_kv_runtime_ext::{OpenRaftKvEnv, OpenRaftKvK8sDeployer, OpenRaftKvTopology};
use openraft_kv_runtime_workloads::{
    OpenRaftMembership, resolve_client_for_node, wait_for_leader, wait_for_membership,
    wait_for_replication, write_batch,
};
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
        Err(ManualClusterError::ClientInit { source }) => {
            warn!("k8s unavailable ({source}); skipping openraft k8s run");

            return Ok(());
        }
        Err(ManualClusterError::InstallStack { source })
            if k8s_cluster_unavailable(&source.to_string()) =>
        {
            warn!("k8s unavailable ({source}); skipping openraft k8s run");

            return Ok(());
        }
        Err(error) => {
            return Err(anyhow::Error::new(error)).context("creating openraft k8s cluster");
        }
    };

    run_failover(cluster, Duration::from_secs(40)).await
}

async fn run_failover(cluster: ManualCluster<OpenRaftKvEnv>, timeout: Duration) -> Result<()> {
    let mut clients = start_cluster(&cluster).await?;

    clients[0].init_self().await?;

    let initial_leader = wait_for_leader(&clients, timeout, None).await?;
    let membership = OpenRaftMembership::discover(&clients).await?;

    add_learners_and_promote(&clients, initial_leader, &membership, timeout).await?;
    write_initial_batch(&clients, initial_leader, timeout).await?;

    restart_leader(&cluster, initial_leader).await?;
    refresh_clients(&cluster, &mut clients)?;

    let new_leader = wait_for_leader(&clients, timeout, Some(initial_leader)).await?;
    write_second_batch(&clients, new_leader, timeout).await?;

    let expected = openraft_kv_runtime_workloads::expected_kv(RAFT_KEY_PREFIX, TOTAL_WRITES);
    wait_for_replication(&clients, &expected, timeout).await?;

    cluster.stop_all();

    Ok(())
}

async fn start_cluster(cluster: &ManualCluster<OpenRaftKvEnv>) -> Result<Vec<OpenRaftKvClient>> {
    let node0 = cluster.start_node("node-0").await?.client;
    let node1 = cluster.start_node("node-1").await?.client;
    let node2 = cluster.start_node("node-2").await?.client;

    cluster.wait_network_ready().await?;

    Ok(vec![node0, node1, node2])
}

async fn add_learners_and_promote(
    clients: &[OpenRaftKvClient],
    leader_id: u64,
    membership: &OpenRaftMembership,
    timeout: Duration,
) -> Result<()> {
    let leader = resolve_client_for_node(clients, leader_id, timeout).await?;

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

    wait_for_membership(clients, &voter_ids, timeout).await?;

    Ok(())
}

async fn write_initial_batch(
    clients: &[OpenRaftKvClient],
    leader_id: u64,
    timeout: Duration,
) -> Result<()> {
    let leader = resolve_client_for_node(clients, leader_id, timeout).await?;
    write_batch(&leader, RAFT_KEY_PREFIX, 0, INITIAL_WRITE_BATCH).await?;

    Ok(())
}

async fn write_second_batch(
    clients: &[OpenRaftKvClient],
    leader_id: u64,
    timeout: Duration,
) -> Result<()> {
    let leader = resolve_client_for_node(clients, leader_id, timeout).await?;
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

fn refresh_clients(
    cluster: &ManualCluster<OpenRaftKvEnv>,
    clients: &mut [OpenRaftKvClient],
) -> Result<()> {
    for (index, slot) in clients.iter_mut().enumerate() {
        *slot = cluster
            .node_client(&format!("node-{index}"))
            .ok_or_else(|| anyhow!("node-{index} client missing after restart"))?;
    }

    Ok(())
}

fn k8s_cluster_unavailable(message: &str) -> bool {
    message.contains("Unable to connect to the server")
        || message.contains("TLS handshake timeout")
        || message.contains("connection refused")
}

use std::time::Duration;

use testing_framework_core::{
    scenario::{Deployer, ScenarioBuilder},
    topology::config::TopologyConfig,
};
use testing_framework_runner_local::LocalDeployer;

#[tokio::test]
#[ignore = "requires local node binary and open ports"]
async fn local_restart_node() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let mut scenario = ScenarioBuilder::topology_with(|t| t.nodes(1))
        .enable_node_control()
        .with_run_duration(Duration::from_secs(1))
        .build()?;

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    let context = runner.context();

    let control = context.node_control().ok_or("node control not available")?;

    let old_pid = control.node_pid(0).ok_or("missing node pid")?;

    control.restart_node(0).await?;

    let new_pid = control.node_pid(0).ok_or("missing node pid")?;
    assert_ne!(old_pid, new_pid, "expected a new process after restart");

    let client = context
        .node_clients()
        .any_client()
        .ok_or("no node clients available")?;
    client.consensus_info().await?;

    let _handle = runner.run(&mut scenario).await?;

    Ok(())
}

#[tokio::test]
#[ignore = "requires local node binary and open ports"]
async fn manual_cluster_restart_node() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let deployer = LocalDeployer::default();
    let cluster = deployer.manual_cluster(TopologyConfig::with_node_numbers(1))?;

    cluster.start_node("a").await?;

    let old_pid = cluster.node_pid(0).ok_or("missing node pid")?;

    cluster.restart_node(0).await?;

    let new_pid = cluster.node_pid(0).ok_or("missing node pid")?;
    assert_ne!(old_pid, new_pid, "expected a new process after restart");

    let client = cluster.node_client("node-a").ok_or("missing node client")?;
    client.consensus_info().await?;

    Ok(())
}

use std::time::Duration;

use testing_framework_core::{
    scenario::{Deployer, ScenarioBuilder},
    topology::config::TopologyConfig,
};
use testing_framework_runner_local::LocalDeployer;
use tracing_subscriber::fmt::try_init;

#[tokio::test]
#[ignore = "requires local node binary and open ports"]
async fn local_restart_node() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = try_init();
    let mut scenario = ScenarioBuilder::topology_with(|t| t.nodes(1))
        .enable_node_control()
        .with_run_duration(Duration::from_secs(1))
        .build()?;

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    let context = runner.context();

    let control = context.node_control().ok_or("node control not available")?;

    let node_name = "node-0";
    let old_pid = control.node_pid(node_name).ok_or("missing node pid")?;

    control.restart_node(node_name).await?;

    let new_pid = control.node_pid(node_name).ok_or("missing node pid")?;
    assert_ne!(old_pid, new_pid, "expected a new process after restart");

    control.stop_node(node_name).await?;
    assert!(
        control.node_pid(node_name).is_none(),
        "expected node pid to be absent after stop"
    );

    let _handle = runner.run(&mut scenario).await?;

    Ok(())
}

#[tokio::test]
#[ignore = "requires local node binary and open ports"]
async fn manual_cluster_restart_node() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let _ = try_init();
    let deployer = LocalDeployer::default();
    let cluster = deployer.manual_cluster(TopologyConfig::with_node_numbers(1))?;

    let node_name = cluster.start_node("a").await?.name;

    let old_pid = cluster.node_pid(&node_name).ok_or("missing node pid")?;

    cluster.restart_node(&node_name).await?;

    let new_pid = cluster.node_pid(&node_name).ok_or("missing node pid")?;
    assert_ne!(old_pid, new_pid, "expected a new process after restart");

    cluster.stop_node(&node_name).await?;
    assert!(
        cluster.node_pid(&node_name).is_none(),
        "expected node pid to be absent after stop"
    );

    Ok(())
}

use std::time::Duration;

use anyhow::{Result, anyhow};
use lb_framework::{
    CoreBuilderExt as _, DeploymentBuilder, LbcLocalDeployer, ScenarioBuilder, TopologyConfig,
};
use testing_framework_core::scenario::Deployer;
use tracing_subscriber::fmt::try_init;

#[tokio::test]
#[ignore = "requires local node binary and open ports"]
async fn local_restart_node() -> Result<()> {
    let _ = try_init();
    let mut scenario = ScenarioBuilder::deployment_with(|t| t.with_node_count(1))
        .with_node_control()
        .with_run_duration(Duration::from_secs(1))
        .build()?;

    let deployer = LbcLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    let context = runner.context();

    let control = context
        .node_control()
        .ok_or_else(|| anyhow!("node control not available"))?;

    let node_name = "node-0";
    let old_pid = control
        .node_pid(node_name)
        .ok_or_else(|| anyhow!("missing node pid"))?;

    control
        .restart_node(node_name)
        .await
        .map_err(|error| anyhow!("failed to restart {node_name}: {error}"))?;

    let new_pid = control
        .node_pid(node_name)
        .ok_or_else(|| anyhow!("missing node pid"))?;
    assert_ne!(old_pid, new_pid, "expected a new process after restart");

    control
        .stop_node(node_name)
        .await
        .map_err(|error| anyhow!("failed to stop {node_name}: {error}"))?;
    assert!(
        control.node_pid(node_name).is_none(),
        "expected node pid to be absent after stop"
    );

    let _handle = runner.run(&mut scenario).await?;

    Ok(())
}

#[tokio::test]
#[ignore = "requires local node binary and open ports"]
async fn manual_cluster_restart_node() -> Result<()> {
    let _ = try_init();
    let deployer = LbcLocalDeployer::default();
    let descriptors = DeploymentBuilder::new(TopologyConfig::with_node_numbers(1)).build()?;
    let cluster = deployer.manual_cluster_from_descriptors(descriptors);

    let node_name = cluster.start_node("a").await?.name;

    let old_pid = cluster
        .node_pid(&node_name)
        .ok_or_else(|| anyhow!("missing node pid"))?;

    cluster
        .restart_node(&node_name)
        .await
        .map_err(|error| anyhow!("failed to restart {node_name}: {error}"))?;

    let new_pid = cluster
        .node_pid(&node_name)
        .ok_or_else(|| anyhow!("missing node pid"))?;
    assert_ne!(old_pid, new_pid, "expected a new process after restart");

    cluster
        .stop_node(&node_name)
        .await
        .map_err(|error| anyhow!("failed to stop {node_name}: {error}"))?;
    assert!(
        cluster.node_pid(&node_name).is_none(),
        "expected node pid to be absent after stop"
    );

    Ok(())
}

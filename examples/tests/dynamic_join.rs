use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use testing_framework_core::scenario::{
    Deployer, DynError, PeerSelection, RunContext, ScenarioBuilder, StartNodeOptions, Workload,
};
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::ScenarioBuilderExt;
use tokio::time::{sleep, timeout};
use tracing_subscriber::fmt::try_init;

const START_DELAY: Duration = Duration::from_secs(5);
const READY_TIMEOUT: Duration = Duration::from_secs(60);
const READY_POLL_INTERVAL: Duration = Duration::from_secs(2);

struct JoinNodeWorkload {
    name: String,
}

impl JoinNodeWorkload {
    fn new(name: impl Into<String>) -> Self {
        Self { name: name.into() }
    }
}

#[async_trait]
impl Workload for JoinNodeWorkload {
    fn name(&self) -> &str {
        "dynamic_join"
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        let handle = ctx
            .node_control()
            .ok_or_else(|| "dynamic join workload requires node control".to_owned())?;

        sleep(START_DELAY).await;

        let node = handle.start_node(&self.name).await?;
        let client = node.api;

        timeout(READY_TIMEOUT, async {
            loop {
                match client.consensus_info().await {
                    Ok(info) if info.height > 0 => break,
                    Ok(_) | Err(_) => sleep(READY_POLL_INTERVAL).await,
                }
            }
        })
        .await
        .map_err(|_| "dynamic join node did not become ready in time")?;

        sleep(ctx.run_duration()).await;
        Ok(())
    }
}

struct JoinNodeWithPeersWorkload {
    name: String,
    peers: Vec<String>,
}

impl JoinNodeWithPeersWorkload {
    fn new(name: impl Into<String>, peers: Vec<String>) -> Self {
        Self {
            name: name.into(),
            peers,
        }
    }
}

#[async_trait]
impl Workload for JoinNodeWithPeersWorkload {
    fn name(&self) -> &str {
        "dynamic_join_with_peers"
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        let handle = ctx
            .node_control()
            .ok_or_else(|| "dynamic join workload requires node control".to_owned())?;

        sleep(START_DELAY).await;

        let options = StartNodeOptions {
            peers: PeerSelection::Named(self.peers.clone()),
            config_patch: None,
        };
        let node = handle.start_node_with(&self.name, options).await?;
        let client = node.api;

        timeout(READY_TIMEOUT, async {
            loop {
                match client.consensus_info().await {
                    Ok(info) if info.height > 0 => break,
                    Ok(_) | Err(_) => sleep(READY_POLL_INTERVAL).await,
                }
            }
        })
        .await
        .map_err(|_| "dynamic join node did not become ready in time")?;

        sleep(ctx.run_duration()).await;
        Ok(())
    }
}

#[tokio::test]
#[ignore = "run manually with `cargo test -p runner-examples -- --ignored`"]
async fn dynamic_join_reaches_consensus_liveness() -> Result<()> {
    let _ = try_init();

    let mut scenario = ScenarioBuilder::topology_with(|t| t.network_star().nodes(2))
        .enable_node_control()
        .with_workload(JoinNodeWorkload::new("joiner"))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(60))
        .build()?;

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    let _handle = runner.run(&mut scenario).await?;

    Ok(())
}

#[tokio::test]
#[ignore = "run manually with `cargo test -p runner-examples -- --ignored`"]
async fn dynamic_join_with_peers_reaches_consensus_liveness() -> Result<()> {
    let mut scenario = ScenarioBuilder::topology_with(|t| t.network_star().nodes(2))
        .enable_node_control()
        .with_workload(JoinNodeWithPeersWorkload::new(
            "joiner",
            vec!["node-0".to_string()],
        ))
        .expect_consensus_liveness()
        .with_run_duration(Duration::from_secs(60))
        .build()?;

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    let _handle = runner.run(&mut scenario).await?;

    Ok(())
}

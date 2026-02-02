use std::{
    net::{SocketAddr, TcpListener},
    time::Duration,
};

use anyhow::Result;
use lb_framework::{
    DeploymentBuilder, LbcEnv, LbcLocalDeployer, NodeHttpClient, ScenarioBuilder, TopologyConfig,
    configs::build_node_run_config,
};
use testing_framework_core::scenario::{Deployer, PeerSelection, StartNodeOptions};
use tracing_subscriber::fmt::try_init;

#[tokio::test]
#[ignore = "run manually with `cargo test -p runner-examples -- --ignored manual_cluster_api_port_override`"]
async fn manual_cluster_api_port_override() -> Result<()> {
    let _ = try_init();
    // Required env vars (set on the command line when running this test):
    // - `POL_PROOF_DEV_MODE=true`
    // - `LOGOS_BLOCKCHAIN_NODE_BIN=...`
    // - `LOGOS_BLOCKCHAIN_CIRCUITS=...`
    // - `RUST_LOG=info` (optional)

    let api_port = random_api_port();

    let deployer = LbcLocalDeployer::new();
    let descriptors = DeploymentBuilder::new(TopologyConfig::with_node_numbers(1)).build()?;
    let cluster = deployer.manual_cluster_from_descriptors(descriptors.clone());

    let node = cluster
        .start_node_with(
            "override-api",
            StartNodeOptions::<LbcEnv>::default()
                .with_peers(PeerSelection::None)
                .create_patch(move |mut run_config| {
                    println!("overriding API port to {api_port}");
                    let current_addr = run_config.user.api.backend.listen_address;
                    run_config.user.api.backend.listen_address =
                        SocketAddr::new(current_addr.ip(), api_port);
                    Ok(run_config)
                }),
        )
        .await?
        .client;

    cluster.wait_network_ready().await?;

    wait_until_consensus_ready(&node).await?;

    assert_eq!(resolved_port(&node), api_port);

    Ok(())
}

#[tokio::test]
#[ignore = "run manually with `cargo test -p runner-examples -- --ignored scenario_builder_api_port_override`"]
async fn scenario_builder_api_port_override() -> Result<()> {
    let _ = try_init();
    // Required env vars (set on the command line when running this test):
    // - `POL_PROOF_DEV_MODE=true`
    // - `LOGOS_BLOCKCHAIN_NODE_BIN=...`
    // - `LOGOS_BLOCKCHAIN_CIRCUITS=...`
    // - `RUST_LOG=info` (optional)
    let api_port = random_api_port();

    let base_builder = DeploymentBuilder::new(TopologyConfig::with_node_numbers(1));
    let base_descriptors = base_builder.clone().build()?;
    let base_node = base_descriptors.nodes().first().expect("node 0 descriptor");
    let mut run_config = build_node_run_config(
        base_node,
        base_descriptors
            .config()
            .node_config_override(base_node.index()),
    )
    .expect("build run config");
    println!("overriding API port to {api_port}");
    let current_addr = run_config.user.api.backend.listen_address;
    run_config.user.api.backend.listen_address = SocketAddr::new(current_addr.ip(), api_port);

    let mut scenario = ScenarioBuilder::new(Box::new(
        base_builder.with_node_config_override(0, run_config),
    ))
    .with_run_duration(Duration::from_secs(1))
    .build()?;

    let deployer = LbcLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    let handle = runner.run(&mut scenario).await?;

    let client = handle
        .context()
        .random_node_client()
        .ok_or_else(|| anyhow::anyhow!("scenario did not expose any node clients"))?;

    client
        .consensus_info()
        .await
        .expect("consensus_info should succeed");

    assert_eq!(resolved_port(&client), api_port);

    Ok(())
}

fn random_api_port() -> u16 {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind random API port");
    listener.local_addr().expect("read API port").port()
}

fn resolved_port(client: &NodeHttpClient) -> u16 {
    client.base_url().port().unwrap_or_default()
}

async fn wait_until_consensus_ready(client: &NodeHttpClient) -> Result<()> {
    const RETRIES: usize = 120;
    const DELAY_MS: u64 = 500;

    for _ in 0..RETRIES {
        if client.consensus_info().await.is_ok() {
            return Ok(());
        }

        tokio::time::sleep(Duration::from_millis(DELAY_MS)).await;
    }

    anyhow::bail!("consensus_info did not become ready in time")
}

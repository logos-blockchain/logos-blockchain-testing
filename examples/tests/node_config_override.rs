use std::{
    net::{SocketAddr, TcpListener},
    time::Duration,
};

use anyhow::Result;
use testing_framework_core::{
    nodes::ApiClient,
    scenario::{Deployer, PeerSelection, ScenarioBuilder, StartNodeOptions},
    topology::config::TopologyConfig,
};
use testing_framework_runner_local::LocalDeployer;
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

    let deployer = LocalDeployer::new();
    let cluster = deployer.manual_cluster(TopologyConfig::with_node_numbers(1))?;

    let node = cluster
        .start_node_with(
            "override-api",
            StartNodeOptions {
                peers: PeerSelection::None,
                config_patch: None,
            }
            .create_patch(move |mut config| {
                println!("overriding API port to {api_port}");

                let current_addr = config.user.http.backend_settings.address;

                config.user.http.backend_settings.address =
                    SocketAddr::new(current_addr.ip(), api_port);

                Ok(config)
            }),
        )
        .await?
        .api;

    node.consensus_info()
        .await
        .expect("consensus_info should succeed");

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

    let mut scenario = ScenarioBuilder::topology_with(|t| {
        t.network_star()
            .nodes(1)
            .node_config_patch_with(0, move |mut config| {
                println!("overriding API port to {api_port}");

                let current_addr = config.user.http.backend_settings.address;

                config.user.http.backend_settings.address =
                    SocketAddr::new(current_addr.ip(), api_port);

                Ok(config)
            })
    })
    .with_run_duration(Duration::from_secs(1))
    .build()?;

    let deployer = LocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    let handle = runner.run(&mut scenario).await?;

    let client = handle
        .context()
        .node_clients()
        .any_client()
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

fn resolved_port(client: &ApiClient) -> u16 {
    client.base_url().port().unwrap_or_default()
}

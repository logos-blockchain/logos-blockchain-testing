use std::{collections::HashSet, time::Duration};

use anyhow::Result;
use lb_ext::{LbcExtEnv, ScenarioBuilder};
use lb_framework::{
    DeploymentBuilder, LbcEnv, LbcLocalDeployer, LbcManualCluster, NodeHttpClient, TopologyConfig,
    configs::build_node_run_config,
};
use testing_framework_core::scenario::{
    Deployer as _, ExternalNodeSource, PeerSelection, StartNodeOptions,
};
use testing_framework_runner_local::ProcessDeployer;
use tokio::time::sleep;

struct SeedCluster {
    _cluster: LbcManualCluster,
    node_a: NodeHttpClient,
    node_b: NodeHttpClient,
    bootstrap_peer_addresses: Vec<String>,
}

impl SeedCluster {
    fn external_sources(&self) -> [ExternalNodeSource; 2] {
        [
            ExternalNodeSource::new("external-a".to_owned(), self.node_a.base_url().to_string()),
            ExternalNodeSource::new("external-b".to_owned(), self.node_b.base_url().to_string()),
        ]
    }
}

#[tokio::test]
#[ignore = "run manually with `cargo test -p runner-examples --test external_sources_local -- --ignored`"]
async fn managed_local_plus_external_sources_are_orchestrated() -> Result<()> {
    let seed_cluster = start_seed_cluster().await?;
    let second_cluster_bootstrap_peers =
        parse_peer_addresses(&seed_cluster.bootstrap_peer_addresses)?;

    let second_topology = DeploymentBuilder::new(TopologyConfig::with_node_numbers(2)).build()?;
    let second_cluster = LbcLocalDeployer::new().manual_cluster_from_descriptors(second_topology);
    let second_c = second_cluster
        .start_node_with(
            "c",
            StartNodeOptions::<LbcEnv>::default()
                .with_peers(PeerSelection::None)
                .create_patch({
                    let peers = second_cluster_bootstrap_peers.clone();
                    move |mut run_config| {
                        run_config
                            .user
                            .network
                            .backend
                            .initial_peers
                            .extend(peers.clone());
                        Ok(run_config)
                    }
                }),
        )
        .await?
        .client;

    let second_d = second_cluster
        .start_node_with(
            "d",
            StartNodeOptions::<LbcEnv>::default()
                .with_peers(PeerSelection::Named(vec!["node-c".to_owned()]))
                .create_patch({
                    let peers = second_cluster_bootstrap_peers.clone();
                    move |mut run_config| {
                        run_config
                            .user
                            .network
                            .backend
                            .initial_peers
                            .extend(peers.clone());
                        Ok(run_config)
                    }
                }),
        )
        .await?
        .client;

    second_cluster.wait_network_ready().await?;

    wait_until_has_peers(&second_c, Duration::from_secs(30)).await?;
    wait_until_has_peers(&second_d, Duration::from_secs(30)).await?;

    second_cluster.add_external_clients([seed_cluster.node_a.clone(), seed_cluster.node_b.clone()]);
    let orchestrated = second_cluster.node_clients();

    assert_eq!(
        orchestrated.len(),
        4,
        "expected 2 managed + 2 external clients"
    );

    let expected_endpoints: HashSet<String> = [
        seed_cluster.node_a.base_url().to_string(),
        seed_cluster.node_b.base_url().to_string(),
        second_c.base_url().to_string(),
        second_d.base_url().to_string(),
    ]
    .into_iter()
    .collect();

    let actual_endpoints: HashSet<String> = orchestrated
        .snapshot()
        .into_iter()
        .map(|client| client.base_url().to_string())
        .collect();

    assert_eq!(actual_endpoints, expected_endpoints);

    for client in orchestrated.snapshot() {
        let _ = client.consensus_info().await?;
    }

    Ok(())
}

#[tokio::test]
#[ignore = "run manually with `cargo test -p runner-examples --test external_sources_local -- --ignored`"]
async fn scenario_managed_plus_external_sources_are_orchestrated() -> Result<()> {
    let seed_cluster = start_seed_cluster().await?;

    let base_builder = DeploymentBuilder::new(TopologyConfig::with_node_numbers(2));
    let base_descriptors = base_builder.clone().build()?;
    let mut deployment_builder = base_builder;
    let parsed_peers = parse_peer_addresses(&seed_cluster.bootstrap_peer_addresses)?;

    for node in base_descriptors.nodes() {
        let mut run_config = build_node_run_config(
            &base_descriptors,
            node,
            base_descriptors.config().node_config_override(node.index()),
        )
        .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        run_config
            .user
            .network
            .backend
            .initial_peers
            .extend(parsed_peers.clone());
        deployment_builder = deployment_builder.with_node_config_override(node.index(), run_config);
    }

    let mut scenario = ScenarioBuilder::new(Box::new(deployment_builder))
        .with_run_duration(Duration::from_secs(5))
        .with_external_nodes(seed_cluster.external_sources().to_vec())
        .build()?;

    let deployer = ProcessDeployer::<LbcExtEnv>::default();
    let runner = deployer.deploy(&scenario).await?;
    let run_handle = runner.run(&mut scenario).await?;

    let clients = run_handle.context().node_clients().snapshot();

    assert_eq!(clients.len(), 4, "expected 2 managed + 2 external clients");

    let first_a_endpoint = seed_cluster.node_a.base_url().to_string();
    let first_b_endpoint = seed_cluster.node_b.base_url().to_string();

    for client in clients.iter().filter(|client| {
        let endpoint = client.base_url().to_string();
        endpoint != first_a_endpoint && endpoint != first_b_endpoint
    }) {
        wait_until_has_peers(client, Duration::from_secs(30)).await?;
    }

    let expected_endpoints: HashSet<String> = [
        seed_cluster.node_a.base_url().to_string(),
        seed_cluster.node_b.base_url().to_string(),
    ]
    .into_iter()
    .collect();

    let actual_endpoints: HashSet<String> = clients
        .iter()
        .map(|client| client.base_url().to_string())
        .collect();

    assert!(
        expected_endpoints.is_subset(&actual_endpoints),
        "scenario context should include external endpoints"
    );

    for client in clients {
        let _ = client.consensus_info().await?;
    }

    Ok(())
}

async fn start_seed_cluster() -> Result<SeedCluster> {
    let topology = DeploymentBuilder::new(TopologyConfig::with_node_numbers(2)).build()?;
    let cluster = LbcLocalDeployer::new().manual_cluster_from_descriptors(topology);
    let node_a = cluster
        .start_node_with("a", node_start_options(PeerSelection::None))
        .await?
        .client;
    let node_b = cluster
        .start_node_with(
            "b",
            node_start_options(PeerSelection::Named(vec!["node-a".to_owned()])),
        )
        .await?
        .client;
    cluster.wait_network_ready().await?;
    let bootstrap_peer_addresses = collect_loopback_peer_addresses(&node_a, &node_b).await?;

    Ok(SeedCluster {
        _cluster: cluster,
        node_a,
        node_b,
        bootstrap_peer_addresses,
    })
}

fn node_start_options(peers: PeerSelection) -> StartNodeOptions<LbcEnv> {
    let mut options = StartNodeOptions::<LbcEnv>::default();
    options.peers = peers;
    options
}

async fn collect_loopback_peer_addresses(
    node_a: &lb_framework::NodeHttpClient,
    node_b: &lb_framework::NodeHttpClient,
) -> Result<Vec<String>> {
    let mut peers = Vec::new();

    for info in [node_a.network_info().await?, node_b.network_info().await?] {
        let addresses: Vec<String> = info
            .listen_addresses
            .into_iter()
            .map(|addr| addr.to_string())
            .collect();

        let mut loopback: Vec<String> = addresses
            .iter()
            .filter(|addr| addr.contains("/127.0.0.1/"))
            .cloned()
            .collect();

        if loopback.is_empty() {
            loopback = addresses;
        }

        peers.extend(loopback);
    }

    Ok(peers)
}

fn parse_peer_addresses<T>(addresses: &[String]) -> Result<Vec<T>>
where
    T: std::str::FromStr,
    T::Err: std::error::Error + Send + Sync + 'static,
{
    addresses
        .iter()
        .map(|address| address.parse::<T>().map_err(Into::into))
        .collect()
}

async fn wait_until_has_peers(client: &NodeHttpClient, timeout: Duration) -> Result<()> {
    let start = tokio::time::Instant::now();
    loop {
        if let Ok(network_info) = client.network_info().await {
            if network_info.n_peers > 0 {
                return Ok(());
            }
        }

        if start.elapsed() >= timeout {
            anyhow::bail!(
                "node {} did not report non-zero peer count within {:?}",
                client.base_url(),
                timeout
            );
        }

        sleep(Duration::from_millis(500)).await;
    }
}

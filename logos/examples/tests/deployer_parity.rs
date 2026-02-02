use std::{env, time::Duration};

use anyhow::Result;
use lb_ext::{
    CoreBuilderExt as _, LbcComposeDeployer, LbcExtEnv, LbcK8sDeployer,
    ScenarioBuilder as ExtScenarioBuilder, ScenarioBuilderExt as _,
};
use lb_framework::{
    CoreBuilderExt as _, LbcEnv, LbcLocalDeployer, ScenarioBuilder as LocalScenarioBuilder,
    ScenarioBuilderExt as _, configs::network::NetworkLayout,
};
use testing_framework_core::{
    scenario::{Deployer as _, Runner},
    topology::DeploymentDescriptor,
};

#[derive(Clone, Copy)]
struct ScenarioSpec {
    nodes: usize,
    run_secs: u64,
    tx_rate: u64,
    tx_users: usize,
    total_wallets: usize,
}

fn shared_spec() -> ScenarioSpec {
    ScenarioSpec {
        nodes: 2,
        run_secs: 30,
        tx_rate: 5,
        tx_users: 500,
        total_wallets: 1000,
    }
}

fn build_local_scenario(
    spec: ScenarioSpec,
) -> Result<testing_framework_core::scenario::Scenario<LbcEnv>> {
    LocalScenarioBuilder::deployment_with(|d| {
        d.with_network_layout(NetworkLayout::Star)
            .with_node_count(spec.nodes)
    })
    .with_run_duration(Duration::from_secs(spec.run_secs))
    .initialize_wallet(spec.total_wallets as u64 * 100, spec.total_wallets)
    .transactions_with(|txs| txs.rate(spec.tx_rate).users(spec.tx_users))
    .expect_consensus_liveness()
    .build()
    .map_err(Into::into)
}

fn build_ext_scenario(
    spec: ScenarioSpec,
) -> Result<testing_framework_core::scenario::Scenario<LbcExtEnv>> {
    ExtScenarioBuilder::deployment_with(|d| {
        d.with_network_layout(NetworkLayout::Star)
            .with_node_count(spec.nodes)
    })
    .with_run_duration(Duration::from_secs(spec.run_secs))
    .initialize_wallet(spec.total_wallets as u64 * 100, spec.total_wallets)
    .transactions_with(|txs| txs.rate(spec.tx_rate).users(spec.tx_users))
    .expect_consensus_liveness()
    .build()
    .map_err(Into::into)
}

#[test]
fn parity_builds_have_same_shape() -> Result<()> {
    let spec = shared_spec();
    let local = build_local_scenario(spec)?;
    let ext = build_ext_scenario(spec)?;

    assert_eq!(
        local.deployment().node_count(),
        ext.deployment().node_count()
    );
    assert_eq!(local.duration(), ext.duration());
    assert_eq!(local.workloads().len(), ext.workloads().len());
    assert_eq!(local.expectations().len(), ext.expectations().len());

    Ok(())
}

#[tokio::test]
async fn local_parity_smoke_opt_in() -> Result<()> {
    if env::var("TF_RUN_LOCAL_PARITY").is_err() {
        return Ok(());
    }

    let mut scenario = build_local_scenario(shared_spec())?;
    let deployer = LbcLocalDeployer::default();
    let runner: Runner<LbcEnv> = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}

#[tokio::test]
async fn compose_parity_smoke_opt_in() -> Result<()> {
    if env::var("TF_RUN_COMPOSE_PARITY").is_err() {
        return Ok(());
    }

    let mut scenario = build_ext_scenario(shared_spec())?;
    let deployer = LbcComposeDeployer::default();
    let runner: Runner<LbcExtEnv> = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}

#[tokio::test]
async fn k8s_parity_smoke_opt_in() -> Result<()> {
    if env::var("TF_RUN_K8S_PARITY").is_err() {
        return Ok(());
    }

    let mut scenario = build_ext_scenario(shared_spec())?;
    let deployer = LbcK8sDeployer::default();
    let runner: Runner<LbcExtEnv> = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;
    Ok(())
}

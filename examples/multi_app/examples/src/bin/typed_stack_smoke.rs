use std::time::Duration;

use async_trait::async_trait;
use kvstore_runtime_ext::{KvEnv, KvLocalApp};
use openraft_kv_runtime_ext::{OpenRaftKvEnv, OpenRaftKvLocalApp};
use openraft_kv_runtime_workloads::{
    OpenRaftMembership, ensure_cluster_size, expected_kv, resolve_client_for_node, wait_for_leader,
    wait_for_membership, wait_for_replication, write_batch,
};
use serde::{Deserialize, Serialize};
use testing_framework_app::{
    AppDeployment, AppHost, AppHostEnv, AppHostLocalDeployer, AppRunContextExt,
    AppScenarioBuilderExt, DeployContext, LocalAppCluster,
};
use testing_framework_core::scenario::{Deployer, DynError, RunContext, Workload};
use tracing::info;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let mut scenario = AppHost::scenario()
        .with_app(ExampleStackApp::new())
        .with_run_duration(Duration::from_secs(5))
        .with_workload(ExampleStackWorkload::new())
        .build()?;

    let deployer = AppHostLocalDeployer::default();
    let runner = deployer.deploy(&scenario).await?;
    runner.run(&mut scenario).await?;

    Ok(())
}

#[derive(Clone)]
struct ExampleStackApp {
    store: KvLocalApp,
    consensus: OpenRaftKvLocalApp,
}

impl ExampleStackApp {
    fn new() -> Self {
        Self {
            store: KvLocalApp::nodes(2),
            consensus: OpenRaftKvLocalApp::nodes(3),
        }
    }
}

#[async_trait]
impl AppDeployment<AppHostEnv> for ExampleStackApp {
    type Handle = ExampleStackHandle;

    async fn deploy(self, ctx: &mut DeployContext<AppHostEnv>) -> Result<Self::Handle, DynError> {
        let store = StoreHandle::new(ctx.deploy_and_expose(self.store).await?);
        let consensus = ConsensusHandle::new(ctx.deploy_and_expose(self.consensus).await?);
        let wallet = WalletHandle::new(store.clone(), consensus.clone());
        let stack = ExampleStackHandle::new(store.clone(), consensus.clone(), wallet.clone());

        ctx.expose(store)?;
        ctx.expose(consensus)?;
        ctx.expose(wallet)?;
        ctx.expose(stack.clone())?;

        Ok(stack)
    }
}

#[derive(Clone)]
struct ExampleStackHandle {
    store: StoreHandle,
    consensus: ConsensusHandle,
    wallet: WalletHandle,
}

impl ExampleStackHandle {
    const fn new(store: StoreHandle, consensus: ConsensusHandle, wallet: WalletHandle) -> Self {
        Self {
            store,
            consensus,
            wallet,
        }
    }

    const fn store(&self) -> &StoreHandle {
        &self.store
    }

    const fn consensus(&self) -> &ConsensusHandle {
        &self.consensus
    }

    const fn wallet(&self) -> &WalletHandle {
        &self.wallet
    }
}

#[derive(Clone)]
struct StoreHandle {
    cluster: LocalAppCluster<KvEnv>,
}

impl StoreHandle {
    const fn new(cluster: LocalAppCluster<KvEnv>) -> Self {
        Self { cluster }
    }

    fn node_count(&self) -> usize {
        self.cluster.node_count()
    }

    async fn put(&self, key: &str, value: &str) -> Result<(), DynError> {
        let Some(client) = self.cluster.first_client() else {
            return Err("store handle has no kv clients".into());
        };

        let response: KvPutResponse = client
            .put(
                key,
                &KvPutRequest {
                    value: value.to_owned(),
                    expected_version: None,
                },
            )
            .await?;

        if !response.applied {
            return Err(format!("store write for {key} was rejected").into());
        }

        Ok(())
    }
}

#[derive(Clone)]
struct ConsensusHandle {
    cluster: LocalAppCluster<OpenRaftKvEnv>,
}

impl ConsensusHandle {
    const fn new(cluster: LocalAppCluster<OpenRaftKvEnv>) -> Self {
        Self { cluster }
    }

    fn node_count(&self) -> usize {
        self.cluster.node_count()
    }

    async fn bootstrap_and_write(&self, prefix: &str, writes: usize) -> Result<(), DynError> {
        let clients = self.cluster.clients();

        ensure_cluster_size(&clients, self.node_count())?;
        clients[0].init_self().await?;

        let leader_id = wait_for_leader(&clients, Duration::from_secs(30), None).await?;
        let membership = OpenRaftMembership::discover(&clients).await?;
        let leader = resolve_client_for_node(&clients, leader_id, Duration::from_secs(30)).await?;

        for learner in membership.learner_targets(leader_id) {
            leader
                .add_learner(learner.node_id, &learner.public_addr)
                .await?;
        }

        let voter_ids = membership.voter_ids();
        leader.change_membership(voter_ids.iter().copied()).await?;
        wait_for_membership(&clients, &voter_ids, Duration::from_secs(30)).await?;

        write_batch(&leader, prefix, 0, writes).await?;
        wait_for_replication(
            &clients,
            &expected_kv(prefix, writes),
            Duration::from_secs(30),
        )
        .await?;

        Ok(())
    }
}

#[derive(Clone)]
struct WalletHandle {
    store: StoreHandle,
    consensus: ConsensusHandle,
}

impl WalletHandle {
    const fn new(store: StoreHandle, consensus: ConsensusHandle) -> Self {
        Self { store, consensus }
    }

    async fn submit_transfer(&self, id: &str) -> Result<(), DynError> {
        self.store.put("/kv/wallet-transfer", id).await?;
        self.consensus
            .bootstrap_and_write("wallet-transfer", 3)
            .await?;

        Ok(())
    }
}

#[derive(Clone)]
struct ExampleStackWorkload;

impl ExampleStackWorkload {
    const fn new() -> Self {
        Self
    }
}

#[async_trait]
impl Workload<AppHostEnv> for ExampleStackWorkload {
    fn name(&self) -> &str {
        "multi_app_typed_stack_smoke"
    }

    async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let stack = ctx.require_app::<ExampleStackHandle>()?;
        let store = ctx.require_app::<StoreHandle>()?;
        let consensus = ctx.require_app::<ConsensusHandle>()?;
        let wallet = ctx.require_app::<WalletHandle>()?;

        ensure_stack_handles_match(&stack, &store, &consensus, &wallet)?;
        store.put("/kv/typed-stack-smoke", "store-ready").await?;
        wallet.submit_transfer("transfer-1").await?;

        info!(
            store_nodes = store.node_count(),
            consensus_nodes = consensus.node_count(),
            "typed multi-app stack handles are available to workloads"
        );

        Ok(())
    }
}

fn ensure_stack_handles_match(
    stack: &ExampleStackHandle,
    store: &StoreHandle,
    consensus: &ConsensusHandle,
    wallet: &WalletHandle,
) -> Result<(), DynError> {
    if stack.store().node_count() != store.node_count() {
        return Err("stack store handle does not match exposed store handle".into());
    }

    if stack.consensus().node_count() != consensus.node_count() {
        return Err("stack consensus handle does not match exposed consensus handle".into());
    }

    if stack.wallet().store.node_count() != wallet.store.node_count() {
        return Err("stack wallet handle does not match exposed wallet handle".into());
    }

    Ok(())
}

#[derive(Serialize)]
struct KvPutRequest {
    value: String,
    expected_version: Option<u64>,
}

#[derive(Deserialize)]
struct KvPutResponse {
    applied: bool,
}

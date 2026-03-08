use std::num::{NonZeroU64, NonZeroUsize};

use lb_framework::{
    configs::{
        deployment::{DeploymentBuilder, TopologyConfig},
        wallet::{WalletConfig, wallet_config_for_users},
    },
    internal::{DeploymentPlan, apply_wallet_config_to_deployment},
};
pub use testing_framework_core::scenario::ObservabilityBuilderExt;
use testing_framework_core::{
    scenario::internal::{NodeControlScenarioBuilder, ObservabilityScenarioBuilder},
    topology::{DeploymentProvider, DeploymentSeed, DynTopologyError},
};
use tracing::warn;

use crate::LbcExtEnv;

pub type ScenarioBuilder = testing_framework_core::scenario::ScenarioBuilder<LbcExtEnv>;
pub type ScenarioBuilderWith<Caps = ()> =
    testing_framework_core::scenario::internal::CoreBuilder<LbcExtEnv, Caps>;

pub trait CoreBuilderExt: Sized {
    fn deployment_with(f: impl FnOnce(DeploymentBuilder) -> DeploymentBuilder) -> Self;

    fn with_wallet_config(self, wallet: WalletConfig) -> Self;

    fn wallets(self, users: usize) -> Self;
}

pub trait ScenarioBuilderExt: Sized {
    fn transactions(self) -> TransactionFlowBuilder<Self>;

    fn transactions_with(
        self,
        f: impl FnOnce(TransactionFlowBuilder<Self>) -> TransactionFlowBuilder<Self>,
    ) -> Self;

    fn expect_consensus_liveness(self) -> Self;

    fn initialize_wallet(self, total_funds: u64, users: usize) -> Self;
}

impl CoreBuilderExt for ScenarioBuilder {
    fn deployment_with(f: impl FnOnce(DeploymentBuilder) -> DeploymentBuilder) -> Self {
        let topology = f(DeploymentBuilder::new(TopologyConfig::empty()));
        ScenarioBuilder::new(Box::new(topology))
    }

    fn with_wallet_config(self, wallet: WalletConfig) -> Self {
        self.map_deployment_provider(|provider| {
            Box::new(WalletConfigProvider {
                inner: provider,
                wallet,
            })
        })
    }

    fn wallets(self, users: usize) -> Self {
        with_wallets_or_warn(self, users, CoreBuilderExt::with_wallet_config)
    }
}

impl CoreBuilderExt for NodeControlScenarioBuilder<LbcExtEnv> {
    fn deployment_with(f: impl FnOnce(DeploymentBuilder) -> DeploymentBuilder) -> Self {
        ScenarioBuilder::deployment_with(f).with_node_control()
    }

    fn with_wallet_config(self, wallet: WalletConfig) -> Self {
        self.map_deployment_provider(|provider| {
            Box::new(WalletConfigProvider {
                inner: provider,
                wallet,
            })
        })
    }

    fn wallets(self, users: usize) -> Self {
        with_wallets_or_warn(self, users, CoreBuilderExt::with_wallet_config)
    }
}

impl CoreBuilderExt for ObservabilityScenarioBuilder<LbcExtEnv> {
    fn deployment_with(f: impl FnOnce(DeploymentBuilder) -> DeploymentBuilder) -> Self {
        ScenarioBuilder::deployment_with(f).with_observability()
    }

    fn with_wallet_config(self, wallet: WalletConfig) -> Self {
        self.map_deployment_provider(|provider| {
            Box::new(WalletConfigProvider {
                inner: provider,
                wallet,
            })
        })
    }

    fn wallets(self, users: usize) -> Self {
        with_wallets_or_warn(self, users, CoreBuilderExt::with_wallet_config)
    }
}

impl<B> ScenarioBuilderExt for B
where
    B: CoreBuilderExt + testing_framework_core::scenario::CoreBuilderExt<Env = LbcExtEnv> + Sized,
{
    fn transactions(self) -> TransactionFlowBuilder<Self> {
        TransactionFlowBuilder {
            builder: self,
            rate: NonZeroU64::MIN,
            users: None,
        }
    }

    fn transactions_with(
        self,
        f: impl FnOnce(TransactionFlowBuilder<Self>) -> TransactionFlowBuilder<Self>,
    ) -> Self {
        f(self.transactions()).apply()
    }

    fn expect_consensus_liveness(self) -> Self {
        self.with_expectation(lb_framework::workloads::ConsensusLiveness::<LbcExtEnv>::default())
    }

    fn initialize_wallet(self, total_funds: u64, users: usize) -> Self {
        let Some(user_count) = NonZeroUsize::new(users) else {
            warn!(
                users,
                "wallet user count must be non-zero; ignoring initialize_wallet"
            );
            return self;
        };

        match WalletConfig::uniform(total_funds, user_count) {
            Ok(wallet) => self.with_wallet_config(wallet),
            Err(error) => {
                warn!(
                    users,
                    total_funds,
                    error = %error,
                    "invalid initialize_wallet input; ignoring initialize_wallet"
                );
                self
            }
        }
    }
}

pub struct TransactionFlowBuilder<B> {
    builder: B,
    rate: NonZeroU64,
    users: Option<NonZeroUsize>,
}

impl<B> TransactionFlowBuilder<B>
where
    B: testing_framework_core::scenario::CoreBuilderExt<Env = LbcExtEnv> + Sized,
{
    pub fn rate(mut self, rate: u64) -> Self {
        match NonZeroU64::new(rate) {
            Some(rate) => self.rate = rate,
            None => warn!(
                rate,
                "transaction rate must be non-zero; keeping previous rate"
            ),
        }
        self
    }

    pub fn users(mut self, users: usize) -> Self {
        match NonZeroUsize::new(users) {
            Some(value) => self.users = Some(value),
            None => warn!(
                users,
                "transaction user count must be non-zero; keeping previous setting"
            ),
        }
        self
    }

    pub fn apply(self) -> B {
        let workload = lb_framework::workloads::transaction::Workload::<LbcExtEnv>::new(self.rate)
            .with_user_limit(self.users);
        self.builder.with_workload(workload)
    }
}

struct WalletConfigProvider {
    inner: Box<dyn DeploymentProvider<DeploymentPlan>>,
    wallet: WalletConfig,
}

impl DeploymentProvider<DeploymentPlan> for WalletConfigProvider {
    fn build(&self, seed: Option<&DeploymentSeed>) -> Result<DeploymentPlan, DynTopologyError> {
        let mut deployment = self.inner.build(seed)?;
        apply_wallet_config_to_deployment(&mut deployment, &self.wallet);
        Ok(deployment)
    }
}

fn with_wallets_or_warn<B>(builder: B, users: usize, apply: impl FnOnce(B, WalletConfig) -> B) -> B
where
    B: CoreBuilderExt,
{
    match wallet_config_for_users(users) {
        Ok(wallet) => apply(builder, wallet),
        Err(error) => {
            warn!(users, error = %error, "invalid wallets input; ignoring wallets");
            builder
        }
    }
}

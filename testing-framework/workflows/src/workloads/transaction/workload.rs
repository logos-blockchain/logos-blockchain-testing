use std::{
    collections::{HashMap, VecDeque},
    num::{NonZeroU64, NonZeroUsize},
    sync::Arc,
    time::Duration,
};

use async_trait::async_trait;
use key_management_system_service::keys::{ZkKey, ZkPublicKey};
use nomos_core::mantle::{
    GenesisTx as _, Note, SignedMantleTx, Transaction as _, Utxo, tx_builder::MantleTxBuilder,
};
use testing_framework_config::topology::configs::wallet::WalletAccount;
use testing_framework_core::{
    scenario::{DynError, Expectation, RunContext, RunMetrics, Workload as ScenarioWorkload},
    topology::generation::{GeneratedNodeConfig, GeneratedTopology},
};

/// Submission timing plan for transaction workload execution
#[derive(Debug, Clone, Copy)]
pub(super) struct SubmissionPlan {
    /// Number of transactions to submit
    pub transaction_count: usize,
    /// Time interval between submissions  
    pub submission_interval: Duration,
}
use tokio::time::sleep;

use super::expectation::TxInclusionExpectation;
use crate::workloads::util::submit_transaction_via_cluster;

const MAX_SUBMISSION_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone)]
pub struct Workload {
    txs_per_block: NonZeroU64,
    user_limit: Option<NonZeroUsize>,
    accounts: Vec<WalletInput>,
}

#[derive(Clone)]
struct WalletInput {
    account: WalletAccount,
    utxo: Utxo,
}

#[async_trait]
impl ScenarioWorkload for Workload {
    fn name(&self) -> &'static str {
        "tx_workload"
    }

    fn expectations(&self) -> Vec<Box<dyn Expectation>> {
        vec![Box::new(TxInclusionExpectation::new(
            self.txs_per_block,
            self.user_limit,
        ))]
    }

    fn init(
        &mut self,
        descriptors: &GeneratedTopology,
        _run_metrics: &RunMetrics,
    ) -> Result<(), DynError> {
        tracing::info!("initializing transaction workload");

        let wallet_accounts = descriptors.config().wallet().accounts.clone();
        if wallet_accounts.is_empty() {
            return Err(
                "Transaction workload initialization failed: no seeded wallet accounts configured"
                    .into(),
            );
        }

        let reference_node = descriptors
            .nodes()
            .first()
            .ok_or("transaction workload requires at least one node in the topology")?;

        let utxo_map = wallet_utxo_map(reference_node);

        fn match_account_to_utxo(
            account: WalletAccount,
            utxo_map: &HashMap<ZkPublicKey, Utxo>,
        ) -> Option<WalletInput> {
            utxo_map
                .get(&account.public_key())
                .copied()
                .map(|utxo| WalletInput { account, utxo })
        }

        let mut accounts = wallet_accounts
            .into_iter()
            .filter_map(|account| match_account_to_utxo(account, &utxo_map))
            .collect::<Vec<_>>();

        apply_user_limit(&mut accounts, self.user_limit);

        if accounts.is_empty() {
            return Err(
                "Transaction workload initialization failed: could not match any wallet accounts to genesis UTXOs".into(),
            );
        }

        tracing::info!(
            available_accounts = accounts.len(),
            user_limit = self.user_limit.map(|u| u.get()),
            "transaction workload accounts prepared"
        );

        self.accounts = accounts;
        Ok(())
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        tracing::info!(
            txs_per_block = self.txs_per_block.get(),
            users = self.user_limit.map(|u| u.get()),
            "starting transaction workload submission"
        );
        Submission::new(self, ctx)?.execute().await
    }
}

impl Workload {
    /// Creates a workload that targets the provided transactions per block
    /// rate.
    #[must_use]
    pub const fn new(txs_per_block: NonZeroU64) -> Self {
        Self {
            txs_per_block,
            user_limit: None,
            accounts: Vec::new(),
        }
    }

    /// Creates a workload from a raw rate, returning `None` when zero is given.
    #[must_use]
    pub fn with_rate(txs_per_block: u64) -> Option<Self> {
        NonZeroU64::new(txs_per_block).map(Self::new)
    }

    /// Returns the configured transactions per block rate.
    #[must_use]
    pub const fn txs_per_block(&self) -> NonZeroU64 {
        self.txs_per_block
    }

    /// Limits the number of distinct users that will submit transactions.
    #[must_use]
    pub const fn with_user_limit(mut self, user_limit: Option<NonZeroUsize>) -> Self {
        self.user_limit = user_limit;
        self
    }
}

impl Default for Workload {
    fn default() -> Self {
        Self::new(NonZeroU64::MIN)
    }
}

struct Submission<'a> {
    plan: VecDeque<WalletInput>,
    ctx: &'a RunContext,
    interval: Duration,
}

impl<'a> Submission<'a> {
    fn new(workload: &Workload, ctx: &'a RunContext) -> Result<Self, DynError> {
        if workload.accounts.is_empty() {
            return Err("Transaction workload submission failed: no available accounts for transaction creation".into());
        }

        let submission_plan =
            submission_plan(workload.txs_per_block, ctx, workload.accounts.len())?;

        let plan = workload
            .accounts
            .iter()
            .take(submission_plan.transaction_count)
            .cloned()
            .collect::<VecDeque<_>>();

        tracing::info!(
            planned = submission_plan.transaction_count,
            interval_ms = submission_plan.submission_interval.as_millis(),
            accounts_available = workload.accounts.len(),
            "transaction workload submission plan"
        );

        Ok(Self {
            plan,
            ctx,
            interval: submission_plan.submission_interval,
        })
    }

    async fn execute(mut self) -> Result<(), DynError> {
        let total = self.plan.len();
        tracing::info!(
            total,
            interval_ms = self.interval.as_millis(),
            "begin transaction submissions"
        );

        while let Some(input) = self.plan.pop_front() {
            submit_wallet_transaction(self.ctx, &input).await?;

            if !self.interval.is_zero() {
                sleep(self.interval).await;
            }
        }

        tracing::info!("transaction submissions finished");

        Ok(())
    }
}

async fn submit_wallet_transaction(ctx: &RunContext, input: &WalletInput) -> Result<(), DynError> {
    let signed_tx = Arc::new(build_wallet_transaction(input)?);
    tracing::debug!(
        tx_hash = ?signed_tx.hash(),
        user = ?input.account.public_key(),
        "submitting wallet transaction"
    );
    submit_transaction_via_cluster(ctx, signed_tx).await
}

fn build_wallet_transaction(input: &WalletInput) -> Result<SignedMantleTx, DynError> {
    let builder = MantleTxBuilder::new()
        .add_ledger_input(input.utxo)
        .add_ledger_output(Note::new(input.utxo.note.value, input.account.public_key()));

    let mantle_tx = builder.build();

    let tx_hash = mantle_tx.hash();

    let signature = ZkKey::multi_sign(
        std::slice::from_ref(&input.account.secret_key),
        tx_hash.as_ref(),
    )
    .map_err(|err| {
        format!("Transaction workload signing failed: could not sign transaction: {err}")
    })?;

    SignedMantleTx::new(mantle_tx, Vec::new(), signature).map_err(|err| {
        format!("Transaction workload construction failed: invalid transaction structure: {err}")
            .into()
    })
}

fn wallet_utxo_map(node: &GeneratedNodeConfig) -> HashMap<ZkPublicKey, Utxo> {
    let genesis_tx = node.general.consensus_config.genesis_tx.clone();
    let ledger_tx = genesis_tx.mantle_tx().ledger_tx.clone();

    let tx_hash = ledger_tx.hash();

    ledger_tx
        .outputs
        .iter()
        .enumerate()
        .map(|(idx, note)| (note.pk, Utxo::new(tx_hash, idx, *note)))
        .collect()
}

fn apply_user_limit<T>(items: &mut Vec<T>, user_limit: Option<NonZeroUsize>) {
    if let Some(limit) = user_limit {
        let allowed = limit.get().min(items.len());

        items.truncate(allowed);
    }
}

pub(super) fn limited_user_count(user_limit: Option<NonZeroUsize>, available: usize) -> usize {
    user_limit.map_or(available, |limit| limit.get().min(available))
}

pub(super) fn submission_plan(
    txs_per_block: NonZeroU64,
    ctx: &RunContext,
    available_accounts: usize,
) -> Result<SubmissionPlan, DynError> {
    if available_accounts == 0 {
        return Err("Transaction workload planning failed: no accounts available for transaction scheduling".into());
    }

    let run_secs = ctx.run_duration().as_secs_f64();
    let block_secs = ctx
        .run_metrics()
        .block_interval_hint()
        .unwrap_or_else(|| ctx.run_duration())
        .as_secs_f64();

    let estimated_blocks_in_run = run_secs / block_secs;
    let target_transaction_count = (estimated_blocks_in_run * txs_per_block.get() as f64)
        .floor()
        .clamp(0.0, u64::MAX as f64) as u64;

    let actual_transactions_to_submit =
        target_transaction_count.min(available_accounts as u64) as usize;

    if actual_transactions_to_submit == 0 {
        return Err("Transaction workload planning failed: calculated zero transactions to submit based on run duration and target rate".into());
    }

    let mut submission_interval =
        Duration::from_secs_f64(run_secs / actual_transactions_to_submit as f64);
    if submission_interval > MAX_SUBMISSION_INTERVAL {
        submission_interval = MAX_SUBMISSION_INTERVAL;
    }
    Ok(SubmissionPlan {
        transaction_count: actual_transactions_to_submit,
        submission_interval,
    })
}

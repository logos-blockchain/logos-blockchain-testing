use cucumber::given;

use crate::world::{StepResult, TestingFrameworkWorld};

#[given(expr = "wallets total funds is {int} split across {int} users")]
async fn wallets_total_funds(
    world: &mut TestingFrameworkWorld,
    total_funds: u64,
    users: usize,
) -> StepResult {
    world.set_wallets(total_funds, users)
}

#[given(expr = "run duration is {int} seconds")]
async fn run_duration(world: &mut TestingFrameworkWorld, seconds: u64) -> StepResult {
    world.set_run_duration(seconds)
}

#[given(expr = "transactions rate is {int} per block")]
async fn tx_rate(world: &mut TestingFrameworkWorld, rate: u64) -> StepResult {
    world.set_transactions_rate(rate, None)
}

#[given(expr = "transactions rate is {int} per block using {int} users")]
async fn tx_rate_with_users(
    world: &mut TestingFrameworkWorld,
    rate: u64,
    users: usize,
) -> StepResult {
    world.set_transactions_rate(rate, Some(users))
}

#[given(
    expr = "data availability channel rate is {int} per block and blob rate is {int} per block"
)]
async fn da_rates(
    world: &mut TestingFrameworkWorld,
    channel_rate: u64,
    blob_rate: u64,
) -> StepResult {
    world.set_data_availability_rates(channel_rate, blob_rate)
}

#[given(expr = "expect consensus liveness")]
async fn expect_consensus_liveness(world: &mut TestingFrameworkWorld) -> StepResult {
    world.enable_consensus_liveness()
}

#[given(expr = "consensus liveness lag allowance is {int}")]
async fn liveness_lag_allowance(world: &mut TestingFrameworkWorld, blocks: u64) -> StepResult {
    world.set_consensus_liveness_lag_allowance(blocks)
}

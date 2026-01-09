use cucumber::given;

use crate::world::{NetworkKind, StepError, StepResult, TestingFrameworkWorld, parse_deployer};

#[given(expr = "deployer is {string}")]
async fn deployer_is(world: &mut TestingFrameworkWorld, deployer: String) -> StepResult {
    world.set_deployer(parse_deployer(&deployer)?)
}

#[given(expr = "we have a CLI deployer specified")]
async fn auto_deployer(world: &mut TestingFrameworkWorld) -> StepResult {
    let _unused = world
        .deployer
        .ok_or(StepError::MissingDeployer)
        .inspect_err(|e| {
            println!(
                "CLI deployer mode not specified, use '--deployer=compose' or '--deployer=local': {}",
                e
            )
        })?;
    Ok(())
}

#[given(expr = "topology has {int} validators and {int} executors")]
async fn topology_has(
    world: &mut TestingFrameworkWorld,
    validators: usize,
    executors: usize,
) -> StepResult {
    world.set_topology(validators, executors, NetworkKind::Star)
}

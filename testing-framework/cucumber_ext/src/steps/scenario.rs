use cucumber::given;

use crate::world::{NetworkKind, StepResult, TestingFrameworkWorld, parse_deployer};

#[given(expr = "deployer is {string}")]
async fn deployer_is(world: &mut TestingFrameworkWorld, deployer: String) -> StepResult {
    world.set_deployer(parse_deployer(&deployer)?)
}

#[given(expr = "topology has {int} validators and {int} executors")]
async fn topology_has(
    world: &mut TestingFrameworkWorld,
    validators: usize,
    executors: usize,
) -> StepResult {
    world.set_topology(validators, executors, NetworkKind::Star)
}

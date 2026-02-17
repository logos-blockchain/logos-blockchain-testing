use async_trait::async_trait;
use testing_framework_core::scenario::{Deployer, Runner, Scenario};

#[derive(Debug)]
pub struct YourError;

pub struct YourDeployer;

#[async_trait]
impl Deployer for YourDeployer {
    type Error = YourError;

    async fn deploy(&self, _scenario: &Scenario<()>) -> Result<Runner, Self::Error> {
        // Provision infrastructure
        // Wait for readiness
        // Return Runner
        todo!()
    }
}

use async_trait::async_trait;
use testing_framework_core::scenario::{DynError, Expectation, RunContext};

pub struct YourExpectation;

#[async_trait]
impl Expectation for YourExpectation {
    fn name(&self) -> &'static str {
        "your_expectation"
    }

    async fn evaluate(&mut self, _ctx: &RunContext) -> Result<(), DynError> {
        // implementation
        Ok(())
    }
}

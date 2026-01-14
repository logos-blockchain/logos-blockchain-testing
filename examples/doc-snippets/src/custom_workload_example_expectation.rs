use async_trait::async_trait;
use testing_framework_core::scenario::{DynError, Expectation, RunContext};

pub struct ReachabilityExpectation {
    target_idx: usize,
}

impl ReachabilityExpectation {
    pub fn new(target_idx: usize) -> Self {
        Self { target_idx }
    }
}

#[async_trait]
impl Expectation for ReachabilityExpectation {
    fn name(&self) -> &str {
        "target_reachable"
    }

    async fn evaluate(&mut self, ctx: &RunContext) -> Result<(), DynError> {
        let validators = ctx.node_clients().validator_clients();
        let client = validators.get(self.target_idx).ok_or_else(|| {
            Box::new(std::io::Error::new(
                std::io::ErrorKind::Other,
                "missing target client",
            )) as DynError
        })?;

        client
            .consensus_info()
            .await
            .map(|_| ())
            .map_err(|e| e.into())
    }
}

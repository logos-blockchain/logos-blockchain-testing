use std::time::Duration;

use async_trait::async_trait;
use openraft_kv_runtime_ext::{OpenRaftClusterObserver, OpenRaftKvEnv};
use testing_framework_core::{
    observation::ObservationHandle,
    scenario::{DynError, Expectation, RunContext},
};

use crate::support::{expected_kv, wait_for_observed_replication};

/// Expectation that waits for the full voter set and the writes from this run
/// to converge on every node.
#[derive(Clone)]
pub struct OpenRaftKvConverges {
    total_writes: usize,
    timeout: Duration,
    key_prefix: String,
}

impl OpenRaftKvConverges {
    /// Creates a convergence check for the given number of replicated writes.
    #[must_use]
    pub fn new(total_writes: usize) -> Self {
        Self {
            total_writes,
            timeout: Duration::from_secs(30),
            key_prefix: "raft-key".to_owned(),
        }
    }

    /// Overrides the key prefix used to derive expected writes.
    #[must_use]
    pub fn key_prefix(mut self, value: &str) -> Self {
        self.key_prefix = value.to_owned();
        self
    }

    /// Overrides the convergence timeout.
    #[must_use]
    pub const fn timeout(mut self, value: Duration) -> Self {
        self.timeout = value;
        self
    }
}

#[async_trait]
impl Expectation<OpenRaftKvEnv> for OpenRaftKvConverges {
    fn name(&self) -> &str {
        "openraft_kv_converges"
    }

    async fn evaluate(&mut self, ctx: &RunContext<OpenRaftKvEnv>) -> Result<(), DynError> {
        let expected = expected_kv(&self.key_prefix, self.total_writes);
        let observer = ctx.require_extension::<ObservationHandle<OpenRaftClusterObserver>>()?;

        wait_for_observed_replication(&observer, &expected, self.timeout).await?;

        Ok(())
    }
}

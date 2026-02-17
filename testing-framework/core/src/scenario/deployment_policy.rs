use std::time::Duration;

use super::HttpReadinessRequirement;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RetryPolicy {
    pub max_attempts: usize,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl RetryPolicy {
    #[must_use]
    pub const fn new(max_attempts: usize, base_delay: Duration, max_delay: Duration) -> Self {
        Self {
            max_attempts,
            base_delay,
            max_delay,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CleanupPolicy {
    pub preserve_artifacts: bool,
}

impl CleanupPolicy {
    #[must_use]
    pub const fn new(preserve_artifacts: bool) -> Self {
        Self { preserve_artifacts }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DeploymentPolicy {
    pub readiness_enabled: bool,
    pub readiness_requirement: HttpReadinessRequirement,
    pub retry_policy: Option<RetryPolicy>,
    pub cleanup_policy: CleanupPolicy,
}

impl Default for DeploymentPolicy {
    fn default() -> Self {
        Self {
            readiness_enabled: true,
            readiness_requirement: HttpReadinessRequirement::AllNodesReady,
            retry_policy: None,
            cleanup_policy: CleanupPolicy::new(false),
        }
    }
}

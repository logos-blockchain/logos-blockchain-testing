use std::{collections::HashMap, time::Duration};

use async_trait::async_trait;
use rand::{Rng as _, seq::SliceRandom as _, thread_rng};
use testing_framework_core::scenario::{DynError, RunContext, Workload};
use tokio::time::{Instant, sleep};
use tracing::info;

const MIN_DELAY_SPREAD_FALLBACK: Duration = Duration::from_millis(1);

/// Randomly restarts validators during a run to introduce chaos.
#[derive(Debug)]
pub struct RandomRestartWorkload {
    min_delay: Duration,
    max_delay: Duration,
    target_cooldown: Duration,
    include_validators: bool,
}

impl RandomRestartWorkload {
    /// Creates a restart workload with delay bounds and per-target cooldown.
    ///
    /// `min_delay`/`max_delay` bound the sleep between restart attempts, while
    /// `target_cooldown` prevents repeatedly restarting the same node too
    /// quickly. Validators can be selectively included.
    #[must_use]
    pub const fn new(
        min_delay: Duration,
        max_delay: Duration,
        target_cooldown: Duration,
        include_validators: bool,
    ) -> Self {
        Self {
            min_delay,
            max_delay,
            target_cooldown,
            include_validators,
        }
    }

    fn targets(&self, ctx: &RunContext) -> Vec<Target> {
        let mut targets = Vec::new();
        let validator_count = ctx.descriptors().validators().len();
        if self.include_validators {
            if validator_count > 1 {
                for index in 0..validator_count {
                    targets.push(Target::Validator(index));
                }
            } else if validator_count == 1 {
                info!("chaos restart skipping validators: only one validator configured");
            }
        }
        targets
    }

    fn random_delay(&self) -> Duration {
        if self.max_delay <= self.min_delay {
            return self.min_delay;
        }
        let spread = self
            .max_delay
            .checked_sub(self.min_delay)
            .unwrap_or(MIN_DELAY_SPREAD_FALLBACK)
            .as_secs_f64();
        let offset = thread_rng().gen_range(0.0..=spread);
        let delay = self
            .min_delay
            .checked_add(Duration::from_secs_f64(offset))
            .unwrap_or(self.max_delay);
        tracing::debug!(delay_ms = delay.as_millis(), "chaos restart selected delay");
        delay
    }

    fn initialize_cooldowns(&self, targets: &[Target]) -> HashMap<Target, Instant> {
        let now = Instant::now();
        let ready = now.checked_sub(self.target_cooldown).unwrap_or(now);
        targets
            .iter()
            .copied()
            .map(|target| (target, ready))
            .collect()
    }

    async fn pick_target(
        &self,
        targets: &[Target],
        cooldowns: &HashMap<Target, Instant>,
    ) -> Result<Target, DynError> {
        if targets.is_empty() {
            return Err("chaos restart workload has no eligible targets".into());
        }

        loop {
            let now = Instant::now();
            if let Some(next_ready) = cooldowns
                .values()
                .copied()
                .filter(|ready| *ready > now)
                .min()
            {
                let wait = next_ready.saturating_duration_since(now);
                if !wait.is_zero() {
                    tracing::debug!(
                        wait_ms = wait.as_millis(),
                        "chaos restart waiting for cooldown"
                    );
                    sleep(wait).await;
                    continue;
                }
            }

            let available: Vec<Target> = targets
                .iter()
                .copied()
                .filter(|target| cooldowns.get(target).is_none_or(|ready| *ready <= now))
                .collect();

            if let Some(choice) = available.choose(&mut thread_rng()).copied() {
                tracing::debug!(?choice, "chaos restart picked target");
                return Ok(choice);
            }

            if let Some(choice) = targets.choose(&mut thread_rng()).copied() {
                return Ok(choice);
            }
            return Err("chaos restart workload has no eligible targets".into());
        }
    }
}

#[async_trait]
impl Workload for RandomRestartWorkload {
    fn name(&self) -> &'static str {
        "chaos_restart"
    }

    async fn start(&self, ctx: &RunContext) -> Result<(), DynError> {
        let handle = ctx
            .node_control()
            .ok_or_else(|| "chaos restart workload requires node control".to_owned())?;

        let targets = self.targets(ctx);
        if targets.is_empty() {
            return Err("chaos restart workload has no eligible targets".into());
        }

        tracing::info!(
            config = ?self,
            validators = ctx.descriptors().validators().len(),
            target_count = targets.len(),
            "starting chaos restart workload"
        );

        let mut cooldowns = self.initialize_cooldowns(&targets);

        loop {
            sleep(self.random_delay()).await;
            let target = self.pick_target(&targets, &cooldowns).await?;

            match target {
                Target::Validator(index) => {
                    tracing::info!(index, "chaos restarting validator");
                    handle
                        .restart_validator(index)
                        .await
                        .map_err(|err| format!("validator restart failed: {err}"))?
                }
            }

            cooldowns.insert(target, Instant::now() + self.target_cooldown);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
enum Target {
    Validator(usize),
}

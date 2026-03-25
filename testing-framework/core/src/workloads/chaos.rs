use std::{collections::HashMap, mem::swap, time::Duration};

use async_trait::async_trait;
use rand::{Rng as _, seq::SliceRandom as _, thread_rng};
use tokio::time::{Instant, sleep};

use crate::{
    scenario::{
        Application, DynError, NodeControlCapability, RunContext, Workload, internal::CoreBuilder,
    },
    topology::DeploymentDescriptor,
};

const MIN_DELAY_SPREAD_FALLBACK: Duration = Duration::from_millis(1);
const DEFAULT_CHAOS_MIN_DELAY: Duration = Duration::from_secs(10);
const DEFAULT_CHAOS_MAX_DELAY: Duration = Duration::from_secs(30);
const DEFAULT_CHAOS_TARGET_COOLDOWN: Duration = Duration::from_secs(60);
const NO_ELIGIBLE_TARGETS: &str = "chaos restart workload has no eligible targets";

/// Chaos helpers for scenarios that can control nodes.
pub trait ChaosBuilderExt<E: Application>: Sized {
    fn chaos(self) -> ChaosBuilder<E>;

    fn chaos_with(
        self,
        f: impl FnOnce(ChaosBuilder<E>) -> CoreBuilder<E, NodeControlCapability>,
    ) -> CoreBuilder<E, NodeControlCapability>;
}

impl<E: Application> ChaosBuilderExt<E> for CoreBuilder<E, NodeControlCapability> {
    fn chaos(self) -> ChaosBuilder<E> {
        ChaosBuilder { builder: self }
    }

    fn chaos_with(
        self,
        f: impl FnOnce(ChaosBuilder<E>) -> CoreBuilder<E, NodeControlCapability>,
    ) -> CoreBuilder<E, NodeControlCapability> {
        f(self.chaos())
    }
}

pub struct ChaosBuilder<E: Application> {
    builder: CoreBuilder<E, NodeControlCapability>,
}

impl<E: Application> ChaosBuilder<E> {
    #[must_use]
    pub fn apply(self) -> CoreBuilder<E, NodeControlCapability> {
        self.builder
    }

    #[must_use]
    pub fn restart(self) -> ChaosRestartBuilder<E> {
        ChaosRestartBuilder {
            builder: self.builder,
            min_delay: DEFAULT_CHAOS_MIN_DELAY,
            max_delay: DEFAULT_CHAOS_MAX_DELAY,
            target_cooldown: DEFAULT_CHAOS_TARGET_COOLDOWN,
        }
    }
}

pub struct ChaosRestartBuilder<E: Application> {
    builder: CoreBuilder<E, NodeControlCapability>,
    min_delay: Duration,
    max_delay: Duration,
    target_cooldown: Duration,
}

impl<E: Application> ChaosRestartBuilder<E> {
    #[must_use]
    pub fn min_delay(mut self, delay: Duration) -> Self {
        if !delay.is_zero() {
            self.min_delay = delay;
        }
        self
    }

    #[must_use]
    pub fn max_delay(mut self, delay: Duration) -> Self {
        if !delay.is_zero() {
            self.max_delay = delay;
        }
        self
    }

    #[must_use]
    pub fn target_cooldown(mut self, cooldown: Duration) -> Self {
        if !cooldown.is_zero() {
            self.target_cooldown = cooldown;
        }
        self
    }

    #[must_use]
    pub fn apply(mut self) -> CoreBuilder<E, NodeControlCapability> {
        if self.min_delay > self.max_delay {
            swap(&mut self.min_delay, &mut self.max_delay);
        }

        if self.target_cooldown < self.min_delay {
            self.target_cooldown = self.min_delay;
        }

        self.builder.with_workload(RandomRestartWorkload::new(
            self.min_delay,
            self.max_delay,
            self.target_cooldown,
        ))
    }
}

#[derive(Debug)]
pub struct RandomRestartWorkload {
    min_delay: Duration,
    max_delay: Duration,
    target_cooldown: Duration,
}

impl RandomRestartWorkload {
    #[must_use]
    pub const fn new(min_delay: Duration, max_delay: Duration, target_cooldown: Duration) -> Self {
        Self {
            min_delay,
            max_delay,
            target_cooldown,
        }
    }

    fn random_delay(&self) -> Duration {
        if self.max_delay <= self.min_delay {
            return self.min_delay;
        }

        let spread = self.max_delay.saturating_sub(self.min_delay);
        let spread = if spread.is_zero() {
            MIN_DELAY_SPREAD_FALLBACK
        } else {
            spread
        };

        let spread_secs = spread.as_secs_f64();
        let offset = thread_rng().gen_range(0.0..=spread_secs);

        self.min_delay
            .checked_add(Duration::from_secs_f64(offset))
            .unwrap_or(self.max_delay)
    }

    fn initialize_cooldowns(&self, targets: &[Target]) -> HashMap<Target, Instant> {
        let now = Instant::now();
        let ready = now.checked_sub(self.target_cooldown).unwrap_or(now);

        targets
            .iter()
            .cloned()
            .map(|target| (target, ready))
            .collect()
    }

    fn targets<E: Application>(&self, ctx: &RunContext<E>) -> Vec<Target> {
        let node_count = ctx.descriptors().node_count();
        if node_count <= 1 {
            return Vec::new();
        }

        (0..node_count).map(node_target).collect()
    }

    async fn pick_target(
        &self,
        targets: &[Target],
        cooldowns: &HashMap<Target, Instant>,
    ) -> Result<Target, DynError> {
        ensure_targets_exist(targets)?;

        loop {
            let now = Instant::now();
            if let Some(wait) = next_target_wait(now, cooldowns) {
                sleep(wait).await;
                continue;
            }

            return select_target(targets, cooldowns, now);
        }
    }
}

fn ensure_targets_exist(targets: &[Target]) -> Result<(), DynError> {
    if targets.is_empty() {
        return Err(NO_ELIGIBLE_TARGETS.into());
    }

    Ok(())
}

fn next_target_wait(now: Instant, cooldowns: &HashMap<Target, Instant>) -> Option<Duration> {
    let next_ready = cooldowns
        .values()
        .copied()
        .filter(|ready| *ready > now)
        .min()?;
    let wait = next_ready.saturating_duration_since(now);
    if wait.is_zero() { None } else { Some(wait) }
}

fn pick_available_target(
    targets: &[Target],
    cooldowns: &HashMap<Target, Instant>,
    now: Instant,
) -> Option<Target> {
    let available: Vec<Target> = targets
        .iter()
        .cloned()
        .filter(|target| cooldowns.get(target).is_none_or(|ready| *ready <= now))
        .collect();
    available.choose(&mut thread_rng()).cloned()
}

fn select_target(
    targets: &[Target],
    cooldowns: &HashMap<Target, Instant>,
    now: Instant,
) -> Result<Target, DynError> {
    if let Some(target) = pick_available_target(targets, cooldowns, now) {
        return Ok(target);
    }

    targets
        .choose(&mut thread_rng())
        .cloned()
        .ok_or_else(|| NO_ELIGIBLE_TARGETS.into())
}

fn node_target(index: usize) -> Target {
    Target::Node(format!("node-{index}"))
}

#[async_trait]
impl<E: Application> Workload<E> for RandomRestartWorkload {
    fn name(&self) -> &'static str {
        "chaos_restart"
    }

    async fn start(&self, ctx: &RunContext<E>) -> Result<(), DynError> {
        let Some(handle) = ctx.node_control() else {
            return Err("chaos restart workload requires node control".into());
        };

        let targets = self.targets(ctx);
        ensure_targets_exist(&targets)?;

        let mut cooldowns = self.initialize_cooldowns(&targets);

        loop {
            sleep(self.random_delay()).await;
            let target = self.pick_target(&targets, &cooldowns).await?;

            match target {
                Target::Node(ref name) => handle
                    .restart_node(name)
                    .await
                    .map_err(|err| format!("node restart failed: {err}"))?,
            }

            cooldowns.insert(target, Instant::now() + self.target_cooldown);
        }
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Target {
    Node(String),
}

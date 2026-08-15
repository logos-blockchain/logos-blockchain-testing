use std::{
    collections::{HashMap, HashSet},
    mem::swap,
    time::Duration,
};

use async_trait::async_trait;
use rand::{Rng as _, seq::SliceRandom as _, thread_rng};
use tokio::time::{Instant, sleep, sleep_until, timeout_at};

use crate::{
    scenario::{
        Application, DynError, NodeControlCapability, RunContext, ScenarioBuilder, Workload,
        internal::{CoreBuilder, CoreBuilderAccess, NodeControlScenarioBuilder},
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

/// Direct random-restart verb that requests node control when necessary.
pub trait RestartChaosBuilderExt: Sized {
    type Target: CoreBuilderAccess;

    #[must_use]
    fn restart_nodes_randomly(self) -> RestartBuilder<Self::Target>;
}

impl<E: Application> RestartChaosBuilderExt for ScenarioBuilder<E> {
    type Target = NodeControlScenarioBuilder<E>;

    fn restart_nodes_randomly(self) -> RestartBuilder<Self::Target> {
        RestartBuilder::new(self.with_node_control())
    }
}

impl<E: Application> RestartChaosBuilderExt for NodeControlScenarioBuilder<E> {
    type Target = Self;

    fn restart_nodes_randomly(self) -> RestartBuilder<Self::Target> {
        RestartBuilder::new(self)
    }
}

impl<E: Application> RestartChaosBuilderExt for CoreBuilder<E, ()> {
    type Target = CoreBuilder<E, NodeControlCapability>;

    fn restart_nodes_randomly(self) -> RestartBuilder<Self::Target> {
        RestartBuilder::new(self.with_node_control())
    }
}

impl<E: Application> RestartChaosBuilderExt for CoreBuilder<E, NodeControlCapability> {
    type Target = Self;

    fn restart_nodes_randomly(self) -> RestartBuilder<Self::Target> {
        RestartBuilder::new(self)
    }
}

pub struct RestartBuilder<B: CoreBuilderAccess> {
    builder: B,
    min_delay: Duration,
    max_delay: Duration,
    target_cooldown: Duration,
    excluded_nodes: HashSet<String>,
}

impl<B: CoreBuilderAccess> RestartBuilder<B> {
    fn new(builder: B) -> Self {
        Self {
            builder,
            min_delay: DEFAULT_CHAOS_MIN_DELAY,
            max_delay: DEFAULT_CHAOS_MAX_DELAY,
            target_cooldown: DEFAULT_CHAOS_TARGET_COOLDOWN,
            excluded_nodes: HashSet::new(),
        }
    }

    #[must_use]
    pub fn every_secs(self, min: u64, max: u64) -> Self {
        self.every(Duration::from_secs(min), Duration::from_secs(max))
    }

    #[must_use]
    pub const fn every(mut self, min: Duration, max: Duration) -> Self {
        self.min_delay = min;
        self.max_delay = max;
        self
    }

    #[must_use]
    pub fn cooldown_secs(self, secs: u64) -> Self {
        self.cooldown(Duration::from_secs(secs))
    }

    #[must_use]
    pub const fn cooldown(mut self, cooldown: Duration) -> Self {
        self.target_cooldown = cooldown;
        self
    }

    #[must_use]
    pub fn excluding_nodes(mut self, nodes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.excluded_nodes
            .extend(nodes.into_iter().map(Into::into));
        self
    }

    #[must_use]
    pub fn done(self) -> B {
        let Self {
            builder,
            mut min_delay,
            mut max_delay,
            mut target_cooldown,
            excluded_nodes,
        } = self;

        if min_delay > max_delay {
            swap(&mut min_delay, &mut max_delay);
        }

        if target_cooldown < min_delay {
            target_cooldown = min_delay;
        }

        builder.map_core_builder(|inner| {
            inner.with_workload(
                RandomRestartWorkload::new(min_delay, max_delay, target_cooldown)
                    .excluding_nodes(excluded_nodes),
            )
        })
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
    excluded_nodes: HashSet<String>,
}

impl RandomRestartWorkload {
    #[must_use]
    pub fn new(min_delay: Duration, max_delay: Duration, target_cooldown: Duration) -> Self {
        Self {
            min_delay,
            max_delay,
            target_cooldown,
            excluded_nodes: HashSet::new(),
        }
    }

    #[must_use]
    pub fn excluding_nodes(mut self, nodes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.excluded_nodes
            .extend(nodes.into_iter().map(Into::into));
        self
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

        (0..node_count)
            .map(node_target)
            .filter(|target| match target {
                Target::Node(name) => !self.excluded_nodes.contains(name),
            })
            .collect()
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

        let deadline = Instant::now() + ctx.run_duration();
        while Instant::now() < deadline {
            sleep_until((Instant::now() + self.random_delay()).min(deadline)).await;
            if Instant::now() >= deadline {
                break;
            }
            let target = match timeout_at(deadline, self.pick_target(&targets, &cooldowns)).await {
                Ok(target) => target?,
                Err(_) => break,
            };

            match target {
                Target::Node(ref name) => handle
                    .restart_node(name)
                    .await
                    .map_err(|err| format!("node restart failed: {err}"))?,
            }

            cooldowns.insert(target, Instant::now() + self.target_cooldown);
        }

        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
enum Target {
    Node(String),
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, time::Duration};

    use tokio::time::Instant;

    use super::{
        NO_ELIGIBLE_TARGETS, RandomRestartWorkload, Target, ensure_targets_exist, next_target_wait,
        node_target, select_target,
    };

    #[test]
    fn fixed_restart_delay_is_deterministic() {
        let workload = RandomRestartWorkload::new(
            Duration::from_secs(3),
            Duration::from_secs(3),
            Duration::from_secs(5),
        );

        assert_eq!(workload.random_delay(), Duration::from_secs(3));
    }

    #[test]
    fn random_restart_delay_stays_inside_configured_bounds() {
        let min = Duration::from_millis(10);
        let max = Duration::from_millis(20);
        let workload = RandomRestartWorkload::new(min, max, Duration::from_secs(1));

        for _ in 0..100 {
            let delay = workload.random_delay();
            assert!(delay >= min);
            assert!(delay <= max);
        }
    }

    #[test]
    fn empty_target_set_is_rejected() {
        let error = ensure_targets_exist(&[]).expect_err("empty target set must fail");

        assert_eq!(error.to_string(), NO_ELIGIBLE_TARGETS);
    }

    #[test]
    fn target_selection_ignores_nodes_still_in_cooldown() {
        let now = Instant::now();
        let ready = node_target(0);
        let cooling_down = node_target(1);
        let targets = vec![ready.clone(), cooling_down.clone()];
        let cooldowns = HashMap::from([
            (
                ready.clone(),
                now.checked_sub(Duration::from_secs(1)).unwrap_or(now),
            ),
            (cooling_down, now + Duration::from_secs(5)),
        ]);

        assert_eq!(
            select_target(&targets, &cooldowns, now).expect("one target is ready"),
            ready
        );
        assert_eq!(
            next_target_wait(now, &HashMap::from([(Target::Node("node-0".into()), now)])),
            None
        );
    }
}

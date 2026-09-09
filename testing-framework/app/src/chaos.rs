use std::{
    collections::{HashMap, HashSet},
    marker::PhantomData,
    mem::swap,
    time::Duration,
};

use async_trait::async_trait;
use rand::{Rng as _, seq::SliceRandom as _, thread_rng};
use testing_framework_core::scenario::{
    Application, ClusterHandle, DynError, RunContext, Workload,
};
use tokio::time::{Instant, sleep, sleep_until, timeout_at};

use crate::{AppHostEnv, extension::AppRunContextExt as _};

const MIN_DELAY_SPREAD_FALLBACK: Duration = Duration::from_millis(1);
const DEFAULT_CHAOS_MIN_DELAY: Duration = Duration::from_secs(10);
const DEFAULT_CHAOS_MAX_DELAY: Duration = Duration::from_secs(30);
const DEFAULT_CHAOS_TARGET_COOLDOWN: Duration = Duration::from_secs(60);
const NO_ELIGIBLE_TARGETS: &str = "chaos restart workload has no eligible targets";

/// Randomly restarts nodes of one deployed cluster application.
///
/// The workload resolves the cluster through its exposed [`ClusterHandle`]
/// and drives restarts through that handle, so it works on every backend
/// whose provisioner grants node control.
pub struct ClusterRestartChaos<E: Application> {
    min_delay: Duration,
    max_delay: Duration,
    target_cooldown: Duration,
    excluded_nodes: HashSet<String>,
    handle_name: Option<String>,
    _env: PhantomData<fn() -> E>,
}

impl<E: Application> Default for ClusterRestartChaos<E> {
    fn default() -> Self {
        Self::new()
    }
}

impl<E: Application> ClusterRestartChaos<E> {
    /// Creates the workload with default delay and cooldown settings.
    #[must_use]
    pub fn new() -> Self {
        Self {
            min_delay: DEFAULT_CHAOS_MIN_DELAY,
            max_delay: DEFAULT_CHAOS_MAX_DELAY,
            target_cooldown: DEFAULT_CHAOS_TARGET_COOLDOWN,
            excluded_nodes: HashSet::new(),
            handle_name: None,
            _env: PhantomData,
        }
    }

    /// Sets the random delay window between restarts, in seconds.
    #[must_use]
    pub fn every_secs(self, min: u64, max: u64) -> Self {
        self.every(Duration::from_secs(min), Duration::from_secs(max))
    }

    /// Sets the random delay window between restarts.
    #[must_use]
    pub fn every(mut self, min: Duration, max: Duration) -> Self {
        self.min_delay = min;
        self.max_delay = max;
        self.normalize()
    }

    /// Sets the minimum time before the same node is restarted again.
    #[must_use]
    pub fn cooldown_secs(self, secs: u64) -> Self {
        self.cooldown(Duration::from_secs(secs))
    }

    /// Sets the minimum time before the same node is restarted again.
    #[must_use]
    pub fn cooldown(mut self, cooldown: Duration) -> Self {
        self.target_cooldown = cooldown;
        self.normalize()
    }

    /// Excludes nodes from restart selection by name.
    #[must_use]
    pub fn excluding_nodes(mut self, nodes: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.excluded_nodes
            .extend(nodes.into_iter().map(Into::into));
        self
    }

    /// Targets a named cluster handle instead of the default one.
    #[must_use]
    pub fn named(mut self, name: impl Into<String>) -> Self {
        self.handle_name = Some(name.into());
        self
    }

    fn normalize(mut self) -> Self {
        if self.min_delay > self.max_delay {
            swap(&mut self.min_delay, &mut self.max_delay);
        }
        if self.target_cooldown < self.min_delay {
            self.target_cooldown = self.min_delay;
        }
        self
    }

    fn resolve_handle(&self, ctx: &RunContext<AppHostEnv>) -> Result<ClusterHandle<E>, DynError> {
        match &self.handle_name {
            Some(name) => ctx.require_app_named::<ClusterHandle<E>>(name),
            None => ctx.require_app::<ClusterHandle<E>>(),
        }
        .map_err(Into::into)
    }

    fn targets(&self, handle: &ClusterHandle<E>) -> Vec<Target> {
        let names = handle.node_names();
        if names.len() <= 1 {
            return Vec::new();
        }

        names
            .into_iter()
            .filter(|name| !self.excluded_nodes.contains(name))
            .map(Target::Node)
            .collect()
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

#[async_trait]
impl<E: Application> Workload<AppHostEnv> for ClusterRestartChaos<E> {
    fn name(&self) -> &'static str {
        "chaos_restart"
    }

    async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let handle = self.resolve_handle(ctx)?;

        let targets = self.targets(&handle);
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
    use std::{collections::HashMap, sync::Arc, time::Duration};

    use testing_framework_core::{
        scenario::{ClusterControlProfile, ClusterUnit, NodeClients, NodeControlHandle},
        topology::NodeCountTopology,
    };
    use tokio::time::Instant;

    use super::{
        ClusterRestartChaos, NO_ELIGIBLE_TARGETS, Target, ensure_targets_exist, next_target_wait,
        select_target,
    };

    struct TestEnv;

    #[async_trait::async_trait]
    impl testing_framework_core::scenario::Application for TestEnv {
        type Deployment = NodeCountTopology;
        type NodeClient = u8;
        type NodeConfig = ();
    }

    fn node_target(index: usize) -> Target {
        Target::Node(format!("node-{index}"))
    }

    struct NamedControl {
        names: Vec<String>,
    }

    #[async_trait::async_trait]
    impl NodeControlHandle<TestEnv> for NamedControl {
        fn node_names(&self) -> Vec<String> {
            self.names.clone()
        }
    }

    fn handle_with_names(
        names: &[&str],
    ) -> testing_framework_core::scenario::ClusterHandle<TestEnv> {
        ClusterUnit::new(
            None,
            NodeClients::default(),
            ClusterControlProfile::ExistingClusterAttached,
        )
        .with_node_control(Arc::new(NamedControl {
            names: names.iter().map(ToString::to_string).collect(),
        }))
        .handle()
    }

    #[test]
    fn targets_come_from_the_handle_node_inventory() {
        let workload = ClusterRestartChaos::<TestEnv>::new().excluding_nodes(["svc-b"]);
        let handle = handle_with_names(&["svc-a", "svc-b", "svc-c"]);

        let targets = workload.targets(&handle);

        assert_eq!(
            targets,
            vec![
                Target::Node("svc-a".to_owned()),
                Target::Node("svc-c".to_owned())
            ]
        );
    }

    #[test]
    fn single_node_inventories_yield_no_targets() {
        let workload = ClusterRestartChaos::<TestEnv>::new();
        let handle = handle_with_names(&["svc-a"]);

        assert!(workload.targets(&handle).is_empty());
    }

    #[test]
    fn fixed_restart_delay_is_deterministic() {
        let workload = ClusterRestartChaos::<TestEnv>::new()
            .every(Duration::from_secs(3), Duration::from_secs(3))
            .cooldown(Duration::from_secs(5));

        assert_eq!(workload.random_delay(), Duration::from_secs(3));
    }

    #[test]
    fn random_restart_delay_stays_inside_configured_bounds() {
        let min = Duration::from_millis(10);
        let max = Duration::from_millis(20);
        let workload = ClusterRestartChaos::<TestEnv>::new()
            .every(min, max)
            .cooldown(Duration::from_secs(1));

        for _ in 0..100 {
            let delay = workload.random_delay();
            assert!(delay >= min);
            assert!(delay <= max);
        }
    }

    #[test]
    fn inverted_delay_window_is_normalized() {
        let workload = ClusterRestartChaos::<TestEnv>::new()
            .every(Duration::from_secs(30), Duration::from_secs(5));

        assert!(workload.min_delay <= workload.max_delay);
        assert!(workload.target_cooldown >= workload.min_delay);
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

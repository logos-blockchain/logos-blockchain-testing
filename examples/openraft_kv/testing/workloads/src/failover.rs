use std::time::Duration;

use async_trait::async_trait;
use openraft_kv_node::OpenRaftKvClient;
use openraft_kv_runtime_ext::{OpenRaftClusterObserver, OpenRaftKvEnv};
use testing_framework_core::{
    observation::ObservationHandle,
    scenario::{DynError, RunContext, Workload},
};
use tracing::info;

use crate::support::{
    OpenRaftMembership, ensure_cluster_size, resolve_client_for_node, wait_for_observed_leader,
    wait_for_observed_membership, write_batch,
};

/// Workload that bootstraps the cluster, expands it to three voters, writes one
/// batch, restarts the leader, then writes a second batch through the new
/// leader.
#[derive(Clone)]
pub struct OpenRaftKvFailoverWorkload {
    first_batch: usize,
    second_batch: usize,
    timeout: Duration,
    key_prefix: String,
}

impl OpenRaftKvFailoverWorkload {
    /// Creates the default failover workload configuration.
    #[must_use]
    pub fn new() -> Self {
        Self {
            first_batch: 8,
            second_batch: 8,
            timeout: Duration::from_secs(30),
            key_prefix: "raft-key".to_owned(),
        }
    }

    /// Sets the number of writes issued before the leader restart.
    #[must_use]
    pub const fn first_batch(mut self, value: usize) -> Self {
        self.first_batch = value;
        self
    }

    /// Sets the number of writes issued after the leader restart.
    #[must_use]
    pub const fn second_batch(mut self, value: usize) -> Self {
        self.second_batch = value;
        self
    }

    /// Overrides the key prefix used for generated writes.
    #[must_use]
    pub fn key_prefix(mut self, value: &str) -> Self {
        self.key_prefix = value.to_owned();
        self
    }

    /// Overrides the timeout used for leader and membership transitions.
    #[must_use]
    pub const fn timeout(mut self, value: Duration) -> Self {
        self.timeout = value;
        self
    }
}

impl Default for OpenRaftKvFailoverWorkload {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Workload<OpenRaftKvEnv> for OpenRaftKvFailoverWorkload {
    fn name(&self) -> &str {
        "openraft_kv_failover_workload"
    }

    async fn start(&self, ctx: &RunContext<OpenRaftKvEnv>) -> Result<(), DynError> {
        let clients = ctx.node_clients().snapshot();
        let observer = ctx.require_extension::<ObservationHandle<OpenRaftClusterObserver>>()?;

        ensure_cluster_size(&clients, 3)?;

        self.bootstrap_cluster(&clients).await?;

        let initial_leader = wait_for_observed_leader(&observer, self.timeout, None).await?;
        let membership = OpenRaftMembership::discover(&clients).await?;

        self.promote_cluster(&observer, &clients, initial_leader, &membership)
            .await?;
        self.write_initial_batch(&clients, initial_leader).await?;

        let new_leader = self
            .restart_leader_and_wait_for_failover(ctx, &observer, initial_leader)
            .await?;
        self.write_second_batch(&clients, new_leader).await?;

        Ok(())
    }
}

impl OpenRaftKvFailoverWorkload {
    async fn bootstrap_cluster(&self, clients: &[OpenRaftKvClient]) -> Result<(), DynError> {
        info!("initializing openraft cluster");

        clients[0].init_self().await?;

        Ok(())
    }

    async fn promote_cluster(
        &self,
        observer: &ObservationHandle<OpenRaftClusterObserver>,
        clients: &[OpenRaftKvClient],
        leader_id: u64,
        membership: &OpenRaftMembership,
    ) -> Result<(), DynError> {
        let leader = resolve_client_for_node(clients, leader_id, self.timeout).await?;

        for learner in membership.learner_targets(leader_id) {
            info!(
                target = learner.node_id,
                addr = %learner.public_addr,
                "adding learner"
            );

            leader
                .add_learner(learner.node_id, &learner.public_addr)
                .await?;
        }

        let voter_ids = membership.voter_ids();
        leader.change_membership(voter_ids.iter().copied()).await?;

        wait_for_observed_membership(observer, &voter_ids, self.timeout).await?;

        Ok(())
    }

    async fn write_initial_batch(
        &self,
        clients: &[OpenRaftKvClient],
        leader_id: u64,
    ) -> Result<(), DynError> {
        info!(
            leader = leader_id,
            writes = self.first_batch,
            "writing initial batch"
        );

        let leader = resolve_client_for_node(clients, leader_id, self.timeout).await?;
        write_batch(&leader, &self.key_prefix, 0, self.first_batch).await?;

        Ok(())
    }

    async fn restart_leader_and_wait_for_failover(
        &self,
        ctx: &RunContext<OpenRaftKvEnv>,
        observer: &ObservationHandle<OpenRaftClusterObserver>,
        leader_id: u64,
    ) -> Result<u64, DynError> {
        let Some(control) = ctx.node_control() else {
            return Err("openraft failover workload requires node control".into());
        };

        let leader_name = format!("node-{leader_id}");
        info!(%leader_name, "restarting current leader");

        control.restart_node(&leader_name).await?;

        let new_leader = wait_for_observed_leader(observer, self.timeout, Some(leader_id)).await?;

        info!(
            old_leader = leader_id,
            new_leader, "leader changed after restart"
        );

        Ok(new_leader)
    }

    async fn write_second_batch(
        &self,
        clients: &[OpenRaftKvClient],
        leader_id: u64,
    ) -> Result<(), DynError> {
        info!(
            leader = leader_id,
            writes = self.second_batch,
            "writing second batch"
        );

        let leader = resolve_client_for_node(clients, leader_id, self.timeout).await?;
        write_batch(
            &leader,
            &self.key_prefix,
            self.first_batch,
            self.second_batch,
        )
        .await?;

        Ok(())
    }
}

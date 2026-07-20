# Readiness, Retry, and Artifact Preservation

`DeploymentPolicy` is the single policy struct that controls readiness gating, deploy retries, and artifact retention across all deployers.

---

## The Policy

From `testing-framework-core` (`core/src/scenario/deployment_policy.rs`):

```rust,ignore
pub struct DeploymentPolicy {
    pub readiness_enabled: bool,
    pub readiness_requirement: HttpReadinessRequirement,
    pub retry_policy: Option<RetryPolicy>,
    pub cleanup_policy: CleanupPolicy,
}

pub struct RetryPolicy {
    pub max_attempts: usize,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

pub struct CleanupPolicy {
    pub preserve_artifacts: bool,
}
```

Defaults: `readiness_enabled: true`, `readiness_requirement: HttpReadinessRequirement::AllNodesReady`, `retry_policy: None`, `preserve_artifacts: false`. `HttpReadinessRequirement` is `AllNodesReady`, `AnyNodeReady`, or `AtLeast(usize)`.

Set it on the builder:

```rust,ignore
use std::time::Duration;
use testing_framework_core::scenario::{
    CleanupPolicy, DeploymentPolicy, HttpReadinessRequirement, RetryPolicy,
};

let scenario = KvScenarioBuilder::deployment_with(|_| KvTopology::new(3))
    .with_deployment_policy(DeploymentPolicy {
        readiness_enabled: true,
        readiness_requirement: HttpReadinessRequirement::AtLeast(2),
        retry_policy: Some(RetryPolicy::new(
            5,
            Duration::from_millis(500),
            Duration::from_secs(5),
        )),
        cleanup_policy: CleanupPolicy::new(true),
    })
    .build()?;
```

To adjust only the requirement, `with_http_readiness_requirement(...)` is the shortcut.

---

## Readiness

`readiness_enabled` and `readiness_requirement` gate the post-spawn probe pass in every deployer. Each backend also has its own deployer-level switch that must agree (`ProcessDeployer::with_membership_check(bool)`, `ComposeDeployer::with_readiness(bool)`, `K8sDeployer::with_readiness(bool)`), so effective readiness is `deployer switch && policy.readiness_enabled`. The probe shape (HTTP path vs TCP) comes from the application environment; see the per-deployer chapters ([Local](deployer-local.md), [Compose](deployer-compose.md), [K8s](deployer-k8s.md)).

---

## Retry

`retry_policy` drives the local deployer's spawn-and-readiness loop: on failure, all nodes from the attempt are dropped and the cluster is respawned with exponential backoff (from `base_delay`, capped at `max_delay`, with jitter) up to `max_attempts`. When `retry_policy` is `None`, the local deployer falls back to its built-in default of 3 attempts, 250 ms base delay, 2 s max delay.

The Compose and Kubernetes deployers currently honor the readiness fields of the policy but do not repeat deployment on failure; `retry_policy` has no effect on those backends today.

---

## Artifact Preservation

`cleanup_policy.preserve_artifacts` controls **artifact and tempdir retention, not teardown ordering**. Teardown itself always follows the runner's cleanup-guard chain (see [Handle Ownership and Teardown](handles-teardown.md)); this flag only decides whether per-node working directories survive it.

The local orchestrator computes retention as:

```rust,ignore
policy.cleanup_policy.preserve_artifacts || keep_tempdir_from_env() // TF_KEEP_LOGS
```

so either the policy flag or `TF_KEEP_LOGS=1` (also `true`/`yes`) keeps every node's working directory (configs, on-disk state, anything the process wrote) after the run. Panicking tests preserve working directories regardless.

The container deployers preserve through env vars rather than the policy: `COMPOSE_RUNNER_PRESERVE` / `TESTNET_RUNNER_PRESERVE` keep the compose stack and workspace, `K8S_RUNNER_PRESERVE` keeps the Helm release and namespace. See [Diagnostics and Retained Artifacts](diagnostics.md).

| Backend | Policy `preserve_artifacts` | Env var |
|---|---|---|
| Local | Yes — keeps node tempdirs | `TF_KEEP_LOGS` |
| Compose | No effect | `COMPOSE_RUNNER_PRESERVE` / `TESTNET_RUNNER_PRESERVE` |
| K8s | No effect | `K8S_RUNNER_PRESERVE` |

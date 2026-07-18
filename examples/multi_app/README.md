# Multi-App Acceptance Tests

This directory provides a reusable fixture and end-to-end coverage for composed
systems. The fixture deploys a two-node queue, a two-node result store, and a
worker process that consumes queued jobs and records their completion:

```text
workload -> queue -> worker -> result store -> expectation
```

The local happy-path test drives the fixture through a scenario:

```rust
let mut scenario = AppHost::scenario()
    .with_app(JobStackApp::new())
    .with_workload(EnqueueJobs::new(10))
    .with_expectation(AllJobsCompleted::new(10))
    .build()?;
```

`multi-app-fixture` owns the stack definition. It deploys child apps, exposes
their typed handles, and returns a composed stack handle:

```rust
let queue = ctx.deploy_and_expose(self.queue).await?;
let results = ctx.deploy_and_expose(self.results).await?;
let worker = ctx
    .deploy_and_expose(JobWorkerApp::new(queue_url, results_url))
    .await?;

let stack = JobStackHandle { queue, results, worker };
ctx.expose(stack.clone())?;
```

Workloads, expectations, and later lifecycle tests request the composed handle:

```rust
let stack = ctx.require_app::<JobStackHandle>()?;
```

The worker is part of the deployed system, rather than test code moving data
between otherwise unrelated applications. It lives in the
`multi-app-job-worker` crate, receives the queue and result-store endpoints from
the parent deployment, and is started through `LocalProcessApp`. Reverse
cleanup stops the worker before either dependency.

Resource lifecycle comes from the TF adapter used by a child deployment:

- `LocalProcessApp` manages one local binary process.
- `LocalAppCluster` manages a uniform local cluster.

Attached and external sources are not registered again at the app layer. The
outer scenario resolves them through its existing source providers, and the app
deployment receives the resulting node clients and control profile through
`DeployContext`. When the active deployer provides node control or cluster
readiness, the same handles are available through `DeployContext::node_control`
and `DeployContext::cluster_wait`; the app layer does not create replacements.

Managed adapters register cleanup during deployment. Cleanup runs in reverse
acquisition order and does not depend on the last handle clone being dropped.
An app-specific deployment only describes composition; it does not implement a
second lifecycle interface.

For a single uniform cluster, the core `ScenarioBuilder<AppEnv>` flow remains
valid. For composed systems, prefer this app-layer shape instead of building a
fake outer cluster or adding app-specific code to TF.

Run the local end-to-end test from the workspace root:

```shell
cargo test -p multi-app-e2e --test local_happy_path
```

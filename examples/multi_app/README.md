# Multi-App Acceptance Tests

This directory provides a reusable fixture and end-to-end coverage for composed
systems. The fixture deploys a two-node queue, a two-node result store, and a
worker process that consumes queued jobs and records their completion:

```text
workload -> queue -> worker -> result store -> expectation
```

The Local happy-path test drives the fixture through local child apps:

```rust
let mut scenario = AppHost::scenario()
    .with_app(JobStackApp::new())
    .with_workload(EnqueueJobs::new(10))
    .with_expectation(AllJobsCompleted::new(10))
    .build()?;
```

The Compose test uses the same typed stack handle, workloads, and expectation,
but selects the containerized app declaration and Compose provisioner:

```rust
let mut scenario = AppHost::scenario()
    .with_app_using(
        JobStackContainerApp::new(),
        ComposeContainerProvisioner::default(),
    )
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

For containers, `JobStackContainerApp` submits the queue, result store, and
worker as three requests in dependency order. Clones of
`ComposeContainerProvisioner` share one project, so the services remain mutually
reachable without collapsing the app deployment into one monolithic request.
Dependencies use backend-internal endpoints; workloads use stable published
endpoints.

Resource lifecycle comes from the TF adapter used by a child deployment:

- `LocalProcessApp` manages one local binary process.
- `LocalAppCluster` manages a uniform local cluster.
- `ComposeContainerProvisioner` manages container services in one Compose project.

Container service handles provide per-service `start`, `stop`, `restart`,
`wait_ready`, and `is_running` operations. The API is portable across container
backends, so a Kubernetes provisioner can implement it with Services and
workload resources without changing `JobStackContainerApp`.

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

Run the Local end-to-end test from the workspace root:

```shell
cargo test -p multi-app-e2e --test local_happy_path
```

Build the example images and run the Compose end-to-end test:

```shell
docker build -t queue-node:local -f examples/queue/Dockerfile .
docker build -t kvstore-node:local -f examples/kvstore/Dockerfile .
docker build -t multi-app-job-worker:local -f examples/multi_app/job-worker/Dockerfile .
cargo test -p multi-app-e2e --test compose_happy_path
```

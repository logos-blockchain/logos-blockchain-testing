# Multi-App Examples

This directory shows the canonical app-layer pattern for composed systems.

Use this shape when a scenario needs several apps or resources that should feel
like one system to the workload:

```rust
let mut scenario = AppHost::scenario()
    .with_app(ExampleStackApp::new())
    .with_workload(ExampleStackWorkload::new())
    .build()?;
```

The stack app deploys child apps, exposes their typed handles, and returns a
composed stack handle:

```rust
let store = StoreHandle::new(ctx.deploy_and_expose(self.store).await?);
let consensus = ConsensusHandle::new(ctx.deploy_and_expose(self.consensus).await?);
let wallet = WalletHandle::new(store.clone(), consensus.clone());
let stack = ExampleStackHandle::new(store.clone(), consensus.clone(), wallet.clone());

ctx.expose(store)?;
ctx.expose(consensus)?;
ctx.expose(wallet)?;
ctx.expose(stack.clone())?;
```

Workloads then request concrete handles:

```rust
let stack = ctx.require_app::<ExampleStackHandle>()?;
let wallet = ctx.require_app::<WalletHandle>()?;
```

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

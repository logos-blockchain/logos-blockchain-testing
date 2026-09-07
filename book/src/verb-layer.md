# The Verb Layer

The verb layer provides optional, domain-specific helpers for recurring test actions. It uses the same scenario builder, workloads, and expectations described in the preceding chapters — a verb chain is sugar over an `AppHost` scenario.

---

## Two Equivalent Levels

The explicit API names the objects being assembled:

```rust,ignore
let mut scenario = AppHost::scenario()
    .with_app(ClusterApp::<QueueEnv>::new(QueueTopology::new(5)))
    .with_workload(QueueProduceWorkload::new().operations(400).rate_per_sec(40))
    .with_workload(
        ClusterRestartChaos::<QueueEnv>::new()
            .every_secs(5, 15)
            .excluding_nodes(["node-0"]),
    )
    .with_expectation(QueueConverges::new(400).timeout(Duration::from_secs(60)))
    .with_run_duration(Duration::from_secs(120))
    .build()?;

let runner = AppHostDeployer.deploy(&scenario).await?;
runner.run(&mut scenario).await?;
```

The verb API lowers to those same operations:

```rust,ignore
QueueScenario::nodes(5)
    .produce(400).rate_per_sec(40).done()
    .restart_nodes_randomly().every_secs(5, 15).excluding_nodes(["node-0"]).done()
    .expect_converged(400).within_secs(60)
    .run_secs(120)
    .await?;
```

Runnable as `cargo run -p queue-examples --bin queue_dsl_demo`. The explicit API remains available at every point. Use it for one-off workloads, unusual policies, or operations that do not have a domain verb.

---

## How Verbs Map to the Builder

The queue DSL (`examples/queue/testing/workloads/src/dsl.rs`) shows the pattern. `QueueScenario::nodes(n)` wraps a fresh `AppHost::scenario()` builder and remembers the requested node count:

```rust,ignore
pub struct QueueScenarioBuilder {
    inner: AppHostScenarioBuilder,
    nodes: usize,
}
```

A verb does not introduce a second runtime. Its sub-builder stores an ordinary workload or expectation and adds it when `done()` or a terminal method is called:

```rust,ignore
pub trait QueueDslExt: CoreBuilderAccess<Env = AppHostEnv> + Sized {
    fn produce(self, operations: usize) -> QueueProduceBuilder<Self> {
        QueueProduceBuilder {
            builder: self,
            workload: QueueProduceWorkload::new().operations(operations),
        }
    }
}

impl<B: CoreBuilderAccess<Env = AppHostEnv>> QueueProduceBuilder<B> {
    pub fn done(self) -> B {
        let Self { builder, workload } = self;
        builder.map_core_builder(|inner| inner.with_workload(workload))
    }
}
```

`restart_nodes_randomly()` wraps `ClusterRestartChaos<QueueEnv>` the same way, and `expect_converged(n).within_secs(s)` lowers to `with_expectation(QueueConverges::new(n).timeout(...))`.

The finisher does the rest: `run_secs(secs)` attaches the queue cluster as a `ClusterApp`, sets the run duration, builds the scenario, and runs it through the backend-neutral deployer:

```rust,ignore
impl QueueRunExt for QueueScenarioBuilder {
    async fn run_secs(self, secs: u64) -> Result<(), DynError> {
        let Self { inner, nodes } = self;
        let mut scenario = inner
            .with_app(ClusterApp::<QueueEnv>::new(QueueTopology::new(nodes)))
            .with_run_duration(Duration::from_secs(secs))
            .build()
            .map_err(DynError::from)?;

        let runner = AppHostDeployer.deploy(&scenario).await?;
        runner.run(&mut scenario).await?;
        Ok(())
    }
}
```

Both forms therefore use the same execution, failure aggregation, and teardown. Because the verbs are defined over any `CoreBuilderAccess<Env = AppHostEnv>` builder, generic and application-specific verbs can extend the same chain.

---

## Verbs and Cluster Control

Some verbs imply a runtime requirement. `restart_nodes_randomly()` needs node control on the queue cluster's handle — which `ClusterApp` requests by default (`ClusterControlRequest::Full`), so the verb simply adds the chaos workload and relies on the handle. A data-plane verb such as `produce(...)` needs nothing beyond a client.

Do not hide an unrelated policy choice inside a verb. A verb may configure what its action necessarily needs; retry policy, cleanup policy, backend selection, and other test-wide decisions remain explicit on the cluster app or request.

---

## Designing Application Verbs

Put vocabulary shared by applications in the framework and vocabulary specific to one protocol in that application's testing crate. A verb should:

1. name an operation in the application's domain;
2. configure one workload or expectation, or a small fixed combination;
3. expose meaningful options through a short sub-builder;
4. return the underlying builder through `done()` or a clear terminal method;
5. preserve access to `with_workload` and `with_expectation` for uncommon cases.

The queue example keeps random restarts generic — its verb wraps the framework's `ClusterRestartChaos` — while `produce` and `expect_converged` live with the queue integration. Other applications can then reuse chaos behavior without depending on queue terminology.

---

## See Also

- [Workloads and Concurrency](workloads.md) and [Expectations and Evaluation](expectations.md): the objects verbs add.
- [Cluster Control and Observability](capabilities.md): the control surface restart verbs rely on.
- [Chaos and Controlled Failure](chaos.md): the generic restart workload.

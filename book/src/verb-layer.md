# The Verb Layer

The verb layer provides optional, domain-specific helpers for recurring test actions. It uses the same scenario builder, workloads, expectations, and capabilities described in the preceding chapters.

---

## Two Equivalent Levels

The explicit API names the objects being assembled:

```rust,ignore
let scenario = QueueScenarioBuilder::with_deployment(QueueTopology::new(5))
    .with_node_control()
    .with_network_control()
    .with_workload(QueueProduceWorkload::new().operations(400).rate_per_sec(40))
    .with_workload(RandomRestartWorkload::new(
        Duration::from_secs(5),
        Duration::from_secs(15),
        Duration::from_secs(15),
    ))
    .with_workload(NetworkPartitionWorkload::new(
        NetworkPartitionSpec::new([
            vec!["node-0", "node-1"],
            vec!["node-2", "node-3", "node-4"],
        ]),
        Duration::from_secs(20),
        Duration::from_secs(60),
    ))
    .with_expectation(QueueConverges::new(400).timeout(Duration::from_secs(60)))
    .with_run_duration(Duration::from_secs(120))
    .build()?;
```

The verb API lowers to those same operations:

```rust,ignore
QueueScenario::nodes(5)
    .produce(400).rate_per_sec(40).done()
    .restart_nodes_randomly().every_secs(5, 15).done()
    .partition(["node-0", "node-1"], ["node-2", "node-3", "node-4"])
        .hold_secs(20).done()
    .expect_converged(400).within_secs(60)
    .run_secs(120)
    .await?;
```

The explicit API remains available at every point. Use it for one-off workloads, unusual policies, or operations that do not have a domain verb.

---

## How Verbs Map to the Builder

A verb does not introduce a second runtime. Its sub-builder stores an ordinary workload or expectation and adds it when `done()` or a terminal method is called:

```rust,ignore
pub trait QueueDslExt: CoreBuilderAccess<Env = QueueEnv> + Sized {
    fn produce(self, operations: usize) -> QueueProduceBuilder<Self> {
        QueueProduceBuilder {
            builder: self,
            workload: QueueProduceWorkload::new().operations(operations),
        }
    }
}

impl<B: CoreBuilderAccess<Env = QueueEnv>> QueueProduceBuilder<B> {
    pub fn done(self) -> B {
        self.builder.map_core_builder(|builder| {
            builder.with_workload(self.workload)
        })
    }
}
```

Both forms therefore use the same execution, failure aggregation, and teardown. Generic and application-specific verbs can extend the same builder chain.

---

## Capability-Aware Verbs

Some actions require a runtime capability. The verb should request it when the requirement follows directly from the action:

- `restart_nodes_randomly()` transitions a plain builder to a node-control builder.
- `partition(...).done()` requests network control before adding the partition workload.
- A data-plane verb such as `produce(...)` needs no control capability.

The resulting Rust type records the capability transition. If a deployer cannot supply the requested capability, deployment fails before the workload starts.

Do not hide an unrelated policy choice inside a verb. A verb may request what its action necessarily needs; retry policy, cleanup policy, backend selection, and other test-wide decisions remain explicit.

---

## Designing Application Verbs

Put vocabulary shared by applications in the framework and vocabulary specific to one protocol in that application's testing crate. A verb should:

1. names an operation in the application's domain;
2. configures one workload or expectation, or a small fixed combination;
3. exposes meaningful options through a short sub-builder;
4. returns the underlying builder through `done()` or a clear terminal method;
5. preserves access to `with_workload` and `with_expectation` for uncommon cases.

The queue example keeps `partition` and random restarts generic, while `produce` and `expect_converged` live with the queue integration. Other applications can then reuse chaos behavior without depending on queue terminology.

---

## See Also

- [Workloads and Concurrency](workloads.md) and [Expectations and Evaluation](expectations.md): the objects verbs add.
- [Scenario Capabilities](capabilities.md): the requirements capability-aware verbs request.
- [Chaos and Controlled Failure](chaos.md): the generic restart and partition workloads.

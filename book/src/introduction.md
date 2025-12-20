# Introduction

The Nomos Testing Framework is a purpose-built toolkit for exercising Logos in
realistic, multi-node environments. It solves the gap between small, isolated
tests and full-system validation by letting teams describe a cluster layout,
drive meaningful traffic, and assert the outcomes in one coherent plan.

It is for protocol engineers, infrastructure operators, and QA teams who need
repeatable confidence that validators, executors, and data-availability
components work together under network and timing constraints.

Multi-node integration testing is required because many Logos behaviors—block
progress, data availability, liveness under churn—only emerge when several
roles interact over real networking and time. This framework makes those checks
declarative, observable, and portable across environments.

## A Scenario in 20 Lines

Here's the conceptual shape of every test you'll write:

```rust,ignore
// 1. Define the cluster
let scenario = ScenarioBuilder::topology_with(|t| {
    t.network_star()
        .validators(3)
        .executors(2)
})
// 2. Add workloads (traffic)
.transactions_with(|tx| tx.rate(10).users(5))
.da_with(|da| da.channel_rate(2).blob_rate(2))

// 3. Define success criteria
.expect_consensus_liveness()

// 4. Set experiment duration
.with_run_duration(Duration::from_secs(60))
.build();

// 5. Deploy and run
let runner = deployer.deploy(&scenario).await?;
runner.run(&mut scenario).await?;
```

This pattern—topology, workloads, expectations, duration—repeats across all scenarios in this book.

**Learn more:** For protocol-level documentation and node internals, see the [Nomos Project Documentation](https://nomos-tech.notion.site/project).

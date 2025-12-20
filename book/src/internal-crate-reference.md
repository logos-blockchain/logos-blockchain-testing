# Internal Crate Reference

High-level roles of the crates that make up the framework:

- **Configs** (`testing-framework/configs/`): Prepares reusable configuration primitives for nodes, networking, tracing, data availability, and wallets, shared by all scenarios and runners. Includes topology generation and circuit asset resolution.

- **Core scenario orchestration** (`testing-framework/core/`): Houses the topology and scenario model, runtime coordination, node clients, and readiness/health probes. Defines `Deployer` and `Runner` traits, `ScenarioBuilder`, and `RunContext`.

- **Workflows** (`testing-framework/workflows/`): Packages workloads (transaction, DA, chaos) and expectations (consensus liveness) into reusable building blocks. Offers fluent DSL extensions (`ScenarioBuilderExt`, `ChaosBuilderExt`).

- **Runners** (`testing-framework/runners/{local,compose,k8s}/`): Implements deployment backends (local host, Docker Compose, Kubernetes) that all consume the same scenario plan. Each provides a `Deployer` implementation (`LocalDeployer`, `ComposeDeployer`, `K8sDeployer`).

- **Runner Examples** (crate name: `runner-examples`, path: `examples/`): Runnable binaries demonstrating framework usage and serving as living documentation. These are the **primary entry point** for running scenarios (`examples/src/bin/local_runner.rs`, `examples/src/bin/compose_runner.rs`, `examples/src/bin/k8s_runner.rs`).

## Where to Add New Capabilities

| What You're Adding | Where It Goes | Examples |
|-------------------|---------------|----------|
| **Node config parameter** | `testing-framework/configs/src/topology/configs/` | Slot duration, log levels, DA params |
| **Topology feature** | `testing-framework/core/src/topology/` | New network layouts, node roles |
| **Scenario capability** | `testing-framework/core/src/scenario/` | New capabilities, context methods |
| **Workload** | `testing-framework/workflows/src/workloads/` | New traffic generators |
| **Expectation** | `testing-framework/workflows/src/expectations/` | New success criteria |
| **Builder API** | `testing-framework/workflows/src/builder/` | DSL extensions, fluent methods |
| **Deployer** | `testing-framework/runners/` | New deployment backends |
| **Example scenario** | `examples/src/bin/` | Demonstration binaries |

## Extension Workflow

### Adding a New Workload

1. **Define the workload** in `testing-framework/workflows/src/workloads/your_workload.rs`:
```rust,ignore
use async_trait::async_trait;
use testing_framework_core::scenario::{DynError, RunContext, Workload};

pub struct YourWorkload;

#[async_trait]
impl Workload for YourWorkload {
    fn name(&self) -> &'static str {
        "your_workload"
    }

    async fn start(&self, _ctx: &RunContext) -> Result<(), DynError> {
        // implementation
        Ok(())
    }
}
```

2. **Add builder extension** in `testing-framework/workflows/src/builder/mod.rs`:
```rust,ignore
pub struct YourWorkloadBuilder;

impl YourWorkloadBuilder {
    pub fn some_config(self) -> Self {
        self
    }
}

pub trait ScenarioBuilderExt: Sized {
    fn your_workload(self) -> YourWorkloadBuilder;
}
```

3. **Use in examples** in `examples/src/bin/your_scenario.rs`:
```rust,ignore
use testing_framework_core::scenario::ScenarioBuilder;

pub struct YourWorkloadBuilder;

impl YourWorkloadBuilder {
    pub fn some_config(self) -> Self {
        self
    }
}

pub trait YourWorkloadDslExt: Sized {
    fn your_workload_with<F>(self, configurator: F) -> Self
    where
        F: FnOnce(YourWorkloadBuilder) -> YourWorkloadBuilder;
}

impl<Caps> YourWorkloadDslExt for testing_framework_core::scenario::Builder<Caps> {
    fn your_workload_with<F>(self, configurator: F) -> Self
    where
        F: FnOnce(YourWorkloadBuilder) -> YourWorkloadBuilder,
    {
        let _ = configurator(YourWorkloadBuilder);
        self
    }
}

pub fn use_in_examples() {
    let _plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(3).executors(0))
        .your_workload_with(|w| w.some_config())
        .build();
}
```

### Adding a New Expectation

1. **Define the expectation** in `testing-framework/workflows/src/expectations/your_expectation.rs`:
```rust,ignore
use async_trait::async_trait;
use testing_framework_core::scenario::{DynError, Expectation, RunContext};

pub struct YourExpectation;

#[async_trait]
impl Expectation for YourExpectation {
    fn name(&self) -> &'static str {
        "your_expectation"
    }

    async fn evaluate(&mut self, _ctx: &RunContext) -> Result<(), DynError> {
        // implementation
        Ok(())
    }
}
```

2. **Add builder extension** in `testing-framework/workflows/src/builder/mod.rs`:
```rust,ignore
use testing_framework_core::scenario::ScenarioBuilder;

pub trait YourExpectationDslExt: Sized {
    fn expect_your_condition(self) -> Self;
}

impl<Caps> YourExpectationDslExt for testing_framework_core::scenario::Builder<Caps> {
    fn expect_your_condition(self) -> Self {
        self
    }
}

pub fn use_in_examples() {
    let _plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(3).executors(0))
        .expect_your_condition()
        .build();
}
```

### Adding a New Deployer

1. **Implement `Deployer` trait** in `testing-framework/runners/your_runner/src/deployer.rs`:
```rust,ignore
use async_trait::async_trait;
use testing_framework_core::scenario::{Deployer, Runner, Scenario};

#[derive(Debug)]
pub struct YourError;

pub struct YourDeployer;

#[async_trait]
impl Deployer for YourDeployer {
    type Error = YourError;

    async fn deploy(&self, _scenario: &Scenario<()>) -> Result<Runner, Self::Error> {
        // Provision infrastructure
        // Wait for readiness
        // Return Runner
        todo!()
    }
}
```

2. **Provide cleanup** and handle node control if supported.

3. **Add example** in `examples/src/bin/your_runner.rs`.

For detailed examples, see [Extending the Framework](extending.md) and [Custom Workload Example](custom-workload-example.md).

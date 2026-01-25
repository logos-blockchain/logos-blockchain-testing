use testing_framework_core::scenario::ScenarioBuilder;

use crate::SnippetResult;

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

pub fn use_in_examples() -> SnippetResult<()> {
    let _plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(3))
        .your_workload_with(|w| w.some_config())
        .build()?;
    Ok(())
}

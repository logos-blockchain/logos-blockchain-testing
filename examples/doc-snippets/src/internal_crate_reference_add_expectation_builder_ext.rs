use testing_framework_core::scenario::ScenarioBuilder;

use crate::SnippetResult;

pub trait YourExpectationDslExt: Sized {
    fn expect_your_condition(self) -> Self;
}

impl<Caps> YourExpectationDslExt for testing_framework_core::scenario::Builder<Caps> {
    fn expect_your_condition(self) -> Self {
        self
    }
}

pub fn use_in_examples() -> SnippetResult<()> {
    let _plan = ScenarioBuilder::topology_with(|t| t.network_star().validators(3))
        .expect_your_condition()
        .build()?;
    Ok(())
}

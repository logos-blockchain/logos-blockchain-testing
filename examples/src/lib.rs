pub mod defaults;
pub mod demo;
pub mod env;

pub use env::read_env_any;
pub use testing_framework_workflows::{ChaosBuilderExt, ScenarioBuilderExt};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DeployerKind {
    #[default]
    Local,
    Compose,
}

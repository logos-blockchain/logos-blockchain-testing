pub mod defaults;
pub mod demo;
pub mod env;

pub use env::{read_env_any, read_topology_seed, read_topology_seed_or_default};
pub use lb_framework::ScenarioBuilderExt as NodeScenarioBuilderExt;
pub use lb_workloads::{ChaosBuilderExt, ScenarioBuilderExt};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum DeployerKind {
    #[default]
    Local,
    Compose,
}

pub struct YourWorkloadBuilder;

impl YourWorkloadBuilder {
    pub fn some_config(self) -> Self {
        self
    }
}

pub trait ScenarioBuilderExt: Sized {
    fn your_workload(self) -> YourWorkloadBuilder;
}

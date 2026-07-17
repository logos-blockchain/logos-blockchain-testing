use testing_framework_app::AppScenarioBuilderExt;
use testing_framework_core::scenario::ScenarioBuilder;

use crate::{KvEnv, KvExistingClusterApp, KvTopology};

pub type KvScenarioBuilder = ScenarioBuilder<KvEnv>;

pub trait KvBuilderExt: Sized {
    fn deployment_with(f: impl FnOnce(KvTopology) -> KvTopology) -> Self;

    fn with_existing_kvstore_app(app: KvExistingClusterApp) -> Self;
}

impl KvBuilderExt for KvScenarioBuilder {
    fn deployment_with(f: impl FnOnce(KvTopology) -> KvTopology) -> Self {
        KvScenarioBuilder::with_deployment(f(KvTopology::new(3)))
    }

    fn with_existing_kvstore_app(app: KvExistingClusterApp) -> Self {
        KvScenarioBuilder::with_deployment(app.topology()).with_app(app)
    }
}

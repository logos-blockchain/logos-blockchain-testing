use testing_framework_core::scenario::ScenarioBuilder;

use crate::{PubSubEnv, PubSubTopology, feed::PubSubTopicFeedFactory};

pub type PubSubScenarioBuilder = ScenarioBuilder<PubSubEnv>;

pub trait PubSubBuilderExt: Sized {
    fn deployment_with(f: impl FnOnce(PubSubTopology) -> PubSubTopology) -> Self;
    fn with_topic_feed(self, topic: impl Into<String>) -> Self;
}

impl PubSubBuilderExt for PubSubScenarioBuilder {
    fn deployment_with(f: impl FnOnce(PubSubTopology) -> PubSubTopology) -> Self {
        PubSubScenarioBuilder::with_deployment(f(PubSubTopology::new(3)))
    }

    fn with_topic_feed(self, topic: impl Into<String>) -> Self {
        self.with_runtime_extension_factory(Box::new(PubSubTopicFeedFactory::new(topic)))
    }
}

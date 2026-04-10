use async_trait::async_trait;

use super::{DynError, Feed, FeedRuntime};

#[derive(Clone, Default)]
pub struct DefaultFeed;

impl Feed for DefaultFeed {
    type Subscription = ();

    fn subscribe(&self) -> Self::Subscription {}
}

#[derive(Default)]
pub struct DefaultFeedRuntime;

#[async_trait]
impl FeedRuntime for DefaultFeedRuntime {
    type Feed = DefaultFeed;

    async fn run(self: Box<Self>) {}
}

pub fn default_feed_result() -> Result<(DefaultFeed, DefaultFeedRuntime), DynError> {
    Ok((DefaultFeed, DefaultFeedRuntime))
}

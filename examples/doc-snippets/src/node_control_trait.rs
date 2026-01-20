use async_trait::async_trait;
use testing_framework_core::scenario::DynError;

#[async_trait]
pub trait NodeControlHandle: Send + Sync {
    async fn restart_node(&self, index: usize) -> Result<(), DynError>;
}

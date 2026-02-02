use std::error::Error;

use async_trait::async_trait;

use crate::env::Application;

#[async_trait]
pub trait RuntimeNode<E: Application + ?Sized>: Send {
    type SpawnError: Error + Send + Sync + 'static;

    fn client(&self) -> E::NodeClient;

    fn is_running(&mut self) -> bool;

    fn pid(&self) -> u32;

    async fn stop(&mut self);

    async fn restart(&mut self) -> Result<(), Self::SpawnError>;
}

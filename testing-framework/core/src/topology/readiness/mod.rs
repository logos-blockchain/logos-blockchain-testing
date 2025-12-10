pub mod balancer;
pub mod membership;
pub mod network;

use std::time::Duration;

pub use balancer::DaBalancerReadiness;
pub use membership::{HttpMembershipReadiness, MembershipReadiness};
pub use network::{HttpNetworkReadiness, NetworkReadiness};
use thiserror::Error;
use tokio::time::{sleep, timeout};

use crate::adjust_timeout;

#[derive(Debug, Error)]
pub enum ReadinessError {
    #[error("{message}")]
    Timeout { message: String },
}

#[async_trait::async_trait]
pub trait ReadinessCheck<'a> {
    type Data: Send;

    async fn collect(&'a self) -> Self::Data;

    fn is_ready(&self, data: &Self::Data) -> bool;

    fn timeout_message(&self, data: Self::Data) -> String;

    fn poll_interval(&self) -> Duration {
        Duration::from_millis(200)
    }

    async fn wait(&'a self) -> Result<(), ReadinessError> {
        let timeout_duration = adjust_timeout(Duration::from_secs(60));
        let poll_interval = self.poll_interval();
        let mut data = self.collect().await;

        let wait_result = timeout(timeout_duration, async {
            loop {
                if self.is_ready(&data) {
                    return;
                }

                sleep(poll_interval).await;

                data = self.collect().await;
            }
        })
        .await;

        if wait_result.is_err() {
            let message = self.timeout_message(data);
            return Err(ReadinessError::Timeout { message });
        }

        Ok(())
    }
}

use std::{future::Future, time::Duration};

use tokio::time::sleep;

#[derive(Clone, Copy, Debug)]
pub struct RetryConfig {
    pub max_attempts: usize,
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub backoff_factor: u32,
}

impl RetryConfig {
    #[must_use]
    pub const fn bounded(
        max_attempts: usize,
        initial_delay: Duration,
        max_delay: Duration,
    ) -> Self {
        Self {
            max_attempts,
            initial_delay,
            max_delay,
            backoff_factor: 2,
        }
    }

    #[must_use]
    pub fn delay_for_attempt(self, attempt: usize) -> Duration {
        let mut delay = self.initial_delay;
        for _ in 1..attempt {
            delay = delay.saturating_mul(self.backoff_factor);
            if delay >= self.max_delay {
                return self.max_delay;
            }
        }
        delay.min(self.max_delay)
    }
}

pub async fn retry_async<T, E, Op, Fut>(config: RetryConfig, mut op: Op) -> Result<T, E>
where
    Op: FnMut(usize) -> Fut,
    Fut: Future<Output = Result<T, E>>,
{
    let mut attempt = 1usize;
    loop {
        match op(attempt).await {
            Ok(value) => return Ok(value),
            Err(_) if attempt < config.max_attempts => {
                sleep(config.delay_for_attempt(attempt)).await;
                attempt += 1;
            }

            Err(err) => return Err(err),
        }
    }
}

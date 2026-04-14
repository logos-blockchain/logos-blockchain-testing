pub mod cfgsync;
pub mod env;
pub mod observation;
pub mod runtime;
pub mod scenario;
pub mod topology;
pub mod workloads;

use std::{env as std_env, ops::Mul as _, sync::LazyLock, time::Duration};

pub use runtime::{manual, process, retry};

static IS_SLOW_TEST_ENV: LazyLock<bool> =
    LazyLock::new(|| std_env::var("SLOW_TEST_ENV").is_ok_and(|s| s == "true"));

/// In slow test environments like Codecov, use 2x timeout.
#[must_use]
pub fn adjust_timeout(d: Duration) -> Duration {
    if *IS_SLOW_TEST_ENV { d.mul(2) } else { d }
}

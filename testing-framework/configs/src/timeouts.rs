use std::{env, time::Duration};

pub const DISPERSAL_TIMEOUT_SECS: u64 = 20;
pub const RETRY_COOLDOWN_SECS: u64 = 3;
pub const GRACE_PERIOD_SECS: u64 = 20 * 60;
pub const PRUNE_DURATION_SECS: u64 = 30;
pub const PRUNE_INTERVAL_SECS: u64 = 5;
pub const SHARE_DURATION_SECS: u64 = 5;
pub const COMMITMENTS_WAIT_SECS: u64 = 1;
pub const SDP_TRIGGER_DELAY_SECS: u64 = 5;

fn env_duration(key: &str, default: u64) -> Duration {
    env::var(key)
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or_else(|| Duration::from_secs(default))
}

pub fn dispersal_timeout() -> Duration {
    env_duration("NOMOS_DISPERSAL_TIMEOUT_SECS", DISPERSAL_TIMEOUT_SECS)
}

pub fn retry_cooldown() -> Duration {
    env_duration("NOMOS_RETRY_COOLDOWN_SECS", RETRY_COOLDOWN_SECS)
}

pub fn grace_period() -> Duration {
    env_duration("NOMOS_GRACE_PERIOD_SECS", GRACE_PERIOD_SECS)
}

pub fn prune_duration() -> Duration {
    env_duration("NOMOS_PRUNE_DURATION_SECS", PRUNE_DURATION_SECS)
}

pub fn prune_interval() -> Duration {
    env_duration("NOMOS_PRUNE_INTERVAL_SECS", PRUNE_INTERVAL_SECS)
}

pub fn share_duration() -> Duration {
    env_duration("NOMOS_SHARE_DURATION_SECS", SHARE_DURATION_SECS)
}

pub fn commitments_wait() -> Duration {
    env_duration("NOMOS_COMMITMENTS_WAIT_SECS", COMMITMENTS_WAIT_SECS)
}

pub fn sdp_trigger_delay() -> Duration {
    env_duration("NOMOS_SDP_TRIGGER_DELAY_SECS", SDP_TRIGGER_DELAY_SECS)
}

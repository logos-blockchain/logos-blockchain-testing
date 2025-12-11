#![allow(dead_code)]

use std::process::Child;

use tracing::debug;

/// Check if a child process is still running.
pub fn is_running(child: &mut Child) -> bool {
    match child.try_wait() {
        Ok(None) => {
            debug!("process still running");
            true
        }
        Ok(Some(status)) => {
            debug!(?status, "process exited");
            false
        }
        Err(err) => {
            debug!(error = ?err, "process state check failed");
            false
        }
    }
}

#![allow(dead_code)]

use std::process::Child;
use tracing::debug;

/// Shared lifecycle hooks (placeholder).
pub fn kill_child(child: &mut Child) {
    debug!("killing child process");
    let _ = child.kill();
}

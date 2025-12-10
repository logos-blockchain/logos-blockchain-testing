#![allow(dead_code)]

use std::process::Child;

/// Shared cleanup helpers for child processes.
pub fn kill_child(child: &mut Child) {
    let _ = child.kill();
}

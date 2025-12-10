#![allow(dead_code)]

use std::process::Child;

/// Shared lifecycle hooks (placeholder).
pub fn kill_child(child: &mut Child) {
    let _ = child.kill();
}

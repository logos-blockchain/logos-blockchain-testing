#![allow(dead_code)]

use std::process::Child;

/// Check if a child process is still running.
pub fn is_running(child: &mut Child) -> bool {
    match child.try_wait() {
        Ok(None) => true,
        Ok(Some(_)) | Err(_) => false,
    }
}

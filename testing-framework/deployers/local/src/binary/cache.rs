//! Per-process cache for resolved binary paths.
//!
//! Provider resolution can involve filesystem scans, downloads, or Cargo
//! builds. The local runner may render launch specs repeatedly during one test
//! process, so successful resolutions are cached by the expanded request key.

use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
};

static RESOLVED_BINARIES: OnceLock<Mutex<HashMap<String, PathBuf>>> = OnceLock::new();

/// Shared in-process cache for provider results.
pub(super) struct BinaryCache;

impl BinaryCache {
    pub(super) fn get(key: &str) -> Option<PathBuf> {
        let cache = RESOLVED_BINARIES.get_or_init(|| Mutex::new(HashMap::new()));
        let guard = cache.lock().expect("binary cache mutex poisoned");

        guard.get(key).cloned()
    }

    pub(super) fn insert(key: String, path: PathBuf) {
        let cache = RESOLVED_BINARIES.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = cache.lock().expect("binary cache mutex poisoned");

        guard.insert(key, path);
    }
}

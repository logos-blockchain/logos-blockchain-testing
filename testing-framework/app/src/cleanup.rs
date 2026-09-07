use testing_framework_core::scenario::CleanupGuard;

/// LIFO cleanup owned by one application deployment preparation.
///
/// Managed adapters register guards here as soon as they acquire a resource.
/// If deployment fails, dropping the stack releases everything already
/// acquired. On success, the stack is transferred to the scenario runner.
#[derive(Default)]
pub(crate) struct AppCleanupStack {
    guards: Vec<Box<dyn CleanupGuard>>,
}

impl AppCleanupStack {
    pub(crate) fn push(&mut self, guard: Box<dyn CleanupGuard>) {
        self.guards.push(guard);
    }

    pub(crate) fn into_guard(self) -> Option<Box<dyn CleanupGuard>> {
        if self.guards.is_empty() {
            None
        } else {
            Some(Box::new(self))
        }
    }

    fn cleanup_all(&mut self) {
        while let Some(guard) = self.guards.pop() {
            guard.cleanup();
        }
    }
}

impl CleanupGuard for AppCleanupStack {
    fn cleanup(mut self: Box<Self>) {
        self.cleanup_all();
    }
}

impl Drop for AppCleanupStack {
    fn drop(&mut self) {
        self.cleanup_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use testing_framework_core::scenario::CleanupGuard;

    use super::AppCleanupStack;

    struct Probe {
        label: &'static str,
        order: Arc<Mutex<Vec<&'static str>>>,
    }

    impl CleanupGuard for Probe {
        fn cleanup(self: Box<Self>) {
            self.order.lock().unwrap().push(self.label);
        }
    }

    #[test]
    fn managed_resources_clean_up_in_reverse_acquisition_order() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut cleanup = AppCleanupStack::default();
        cleanup.push(Box::new(Probe {
            label: "dependency",
            order: Arc::clone(&order),
        }));
        cleanup.push(Box::new(Probe {
            label: "dependent",
            order: Arc::clone(&order),
        }));

        drop(cleanup);

        assert_eq!(*order.lock().unwrap(), ["dependent", "dependency"]);
    }
}

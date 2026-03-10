use cfgsync_core::NodeRegistration;

/// Immutable view of registrations currently known to cfgsync.
#[derive(Debug, Clone, Default)]
pub struct RegistrationSnapshot {
    registrations: Vec<NodeRegistration>,
}

impl RegistrationSnapshot {
    #[must_use]
    pub fn new(registrations: Vec<NodeRegistration>) -> Self {
        Self { registrations }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.registrations.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.registrations.is_empty()
    }

    #[must_use]
    pub fn iter(&self) -> impl Iterator<Item = &NodeRegistration> {
        self.registrations.iter()
    }

    #[must_use]
    pub fn get(&self, identifier: &str) -> Option<&NodeRegistration> {
        self.registrations
            .iter()
            .find(|registration| registration.identifier == identifier)
    }
}

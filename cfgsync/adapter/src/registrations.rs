use cfgsync_core::NodeRegistration;
use serde::Serialize;

/// Immutable view of registrations currently known to cfgsync.
#[derive(Debug, Clone, Default, Serialize)]
pub struct RegistrationSnapshot {
    registrations: Vec<NodeRegistration>,
}

impl RegistrationSnapshot {
    #[must_use]
    pub fn new(mut registrations: Vec<NodeRegistration>) -> Self {
        registrations.sort_by(|left, right| left.identifier.cmp(&right.identifier));

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

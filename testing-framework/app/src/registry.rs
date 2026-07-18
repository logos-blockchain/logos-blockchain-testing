use std::{
    any::{Any, TypeId, type_name},
    sync::Arc,
};

use crate::{AppDeployError, AppHandle};

const DEFAULT_HANDLE_NAME: &str = "";

#[derive(Default)]
/// Type-indexed storage for application runtime handles.
///
/// One unnamed handle may be stored per concrete type. Named handles allow
/// multiple instances of the same type. Entries retain exposure order and are
/// released in reverse order. Managed resource teardown is registered
/// separately by TF adapters and does not depend on handle clone counts.
pub struct HandleRegistry {
    handles: Vec<(HandleKey, Box<StoredHandle>)>,
}

impl HandleRegistry {
    /// Creates an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Stores the default unnamed handle for `T`.
    ///
    /// Returns [`AppDeployError::DuplicateHandle`] if one is already stored.
    pub fn expose<T>(&mut self, handle: T) -> Result<(), AppDeployError>
    where
        T: AppHandle,
    {
        self.expose_named(DEFAULT_HANDLE_NAME, handle)
    }

    /// Stores `handle` under its concrete type and `name`.
    ///
    /// Returns [`AppDeployError::DuplicateHandle`] for an existing type/name
    /// pair.
    pub fn expose_named<T>(
        &mut self,
        name: impl Into<String>,
        handle: T,
    ) -> Result<(), AppDeployError>
    where
        T: AppHandle,
    {
        let key = HandleKey::new::<T>(name);
        if self.handles.iter().any(|(existing, _)| existing == &key) {
            return Err(AppDeployError::DuplicateHandle {
                type_name: type_name::<T>(),
                name: key.display_name().map(ToOwned::to_owned),
            });
        }

        self.handles.push((key, Box::new(handle)));
        Ok(())
    }

    /// Returns a clone of the default handle for `T`, if present.
    #[must_use]
    pub fn get<T>(&self) -> Option<T>
    where
        T: AppHandle,
    {
        self.get_named(DEFAULT_HANDLE_NAME)
    }

    /// Returns a clone of the named handle for `T`, if present.
    #[must_use]
    pub fn get_named<T>(&self, name: &str) -> Option<T>
    where
        T: AppHandle,
    {
        let key = HandleKey::new::<T>(name);
        self.handles
            .iter()
            .find(|(existing, _)| existing == &key)
            .and_then(|(_, handle)| handle.as_ref().downcast_ref::<T>())
            .cloned()
    }

    /// Returns the default handle for `T` or [`AppDeployError::HandleMissing`].
    pub fn require<T>(&self) -> Result<T, AppDeployError>
    where
        T: AppHandle,
    {
        self.require_named(DEFAULT_HANDLE_NAME)
    }

    /// Returns the named handle for `T` or [`AppDeployError::HandleMissing`].
    pub fn require_named<T>(&self, name: &str) -> Result<T, AppDeployError>
    where
        T: AppHandle,
    {
        self.get_named(name)
            .ok_or_else(|| AppDeployError::HandleMissing {
                type_name: type_name::<T>(),
                name: (!name.is_empty()).then(|| name.to_owned()),
            })
    }

    /// Returns whether the default handle for `T` is present.
    #[must_use]
    pub fn contains<T>(&self) -> bool
    where
        T: AppHandle,
    {
        self.contains_named::<T>(DEFAULT_HANDLE_NAME)
    }

    /// Returns whether the named handle for `T` is present.
    #[must_use]
    pub fn contains_named<T>(&self, name: &str) -> bool
    where
        T: AppHandle,
    {
        let key = HandleKey::new::<T>(name);
        self.handles.iter().any(|(existing, _)| existing == &key)
    }

    /// Returns whether the registry contains no handles.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }
}

impl Drop for HandleRegistry {
    fn drop(&mut self) {
        // Release arbitrary handle state deterministically. Managed resources
        // use the deployment cleanup stack, whose order follows acquisition.
        while self.handles.pop().is_some() {}
    }
}

#[derive(Clone, Default)]
/// Scenario runtime extension containing exposed application handles.
///
/// Runtime clones share one registry. Managed resources have a separate
/// scenario-owned cleanup stack.
pub struct AppRuntime {
    handles: Arc<HandleRegistry>,
}

impl AppRuntime {
    /// Wraps a prepared handle registry as a scenario runtime extension.
    #[must_use]
    pub fn new(handles: HandleRegistry) -> Self {
        Self {
            handles: Arc::new(handles),
        }
    }

    /// Returns a clone of the default handle for `T`, if present.
    #[must_use]
    pub fn get<T>(&self) -> Option<T>
    where
        T: AppHandle,
    {
        self.handles.get()
    }

    /// Returns a clone of the named handle for `T`, if present.
    #[must_use]
    pub fn get_named<T>(&self, name: &str) -> Option<T>
    where
        T: AppHandle,
    {
        self.handles.get_named(name)
    }

    /// Returns the default handle for `T` or [`AppDeployError::HandleMissing`].
    pub fn require<T>(&self) -> Result<T, AppDeployError>
    where
        T: AppHandle,
    {
        self.handles.require()
    }

    /// Returns the named handle for `T` or [`AppDeployError::HandleMissing`].
    pub fn require_named<T>(&self, name: &str) -> Result<T, AppDeployError>
    where
        T: AppHandle,
    {
        self.handles.require_named(name)
    }
}

#[derive(Eq, Hash, PartialEq)]
struct HandleKey {
    type_id: TypeId,
    name: String,
}

impl HandleKey {
    fn new<T: 'static>(name: impl Into<String>) -> Self {
        Self {
            type_id: TypeId::of::<T>(),
            name: name.into(),
        }
    }

    fn display_name(&self) -> Option<&str> {
        (!self.name.is_empty()).then_some(self.name.as_str())
    }
}

type StoredHandle = dyn Any + Send + Sync;

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::HandleRegistry;

    #[derive(Clone, Debug, Eq, PartialEq)]
    struct Handle(u8);

    #[test]
    fn named_handles_allow_multiple_instances_of_one_type() {
        let mut handles = HandleRegistry::new();
        handles.expose_named("left", Handle(1)).unwrap();
        handles.expose_named("right", Handle(2)).unwrap();

        assert_eq!(handles.get_named("left"), Some(Handle(1)));
        assert_eq!(handles.get_named("right"), Some(Handle(2)));
    }

    #[test]
    fn duplicate_handle_is_rejected() {
        let mut handles = HandleRegistry::new();
        handles.expose(Handle(1)).unwrap();

        let error = handles.expose(Handle(2)).unwrap_err();
        assert!(error.to_string().contains("already exposed"));
        assert_eq!(handles.get(), Some(Handle(1)));
    }

    #[test]
    fn handles_drop_in_reverse_exposure_order() {
        let order = Arc::new(Mutex::new(Vec::new()));
        let mut handles = HandleRegistry::new();
        handles
            .expose_named("first", OwnedHandle::new("first", Arc::clone(&order)))
            .unwrap();
        handles
            .expose_named("second", OwnedHandle::new("second", Arc::clone(&order)))
            .unwrap();

        drop(handles);

        assert_eq!(*order.lock().unwrap(), ["second", "first"]);
    }

    #[derive(Clone)]
    struct OwnedHandle {
        _resource: Arc<OwnedResource>,
    }

    impl OwnedHandle {
        fn new(label: &'static str, order: Arc<Mutex<Vec<&'static str>>>) -> Self {
            Self {
                _resource: Arc::new(OwnedResource { label, order }),
            }
        }
    }

    struct OwnedResource {
        label: &'static str,
        order: Arc<Mutex<Vec<&'static str>>>,
    }

    impl Drop for OwnedResource {
        fn drop(&mut self) {
            self.order.lock().unwrap().push(self.label);
        }
    }
}

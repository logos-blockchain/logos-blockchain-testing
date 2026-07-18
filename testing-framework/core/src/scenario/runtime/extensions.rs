use std::{
    any::{Any, TypeId, type_name},
    collections::HashMap,
};

use async_trait::async_trait;
use tokio::task::JoinHandle;

use super::context::CleanupGuard;
use crate::scenario::{Application, DynError, NodeClients};

/// Prepared runtime extension value plus optional cleanup.
pub struct PreparedRuntimeExtension {
    type_id: TypeId,
    type_name: &'static str,
    value: Box<dyn Any + Send + Sync>,
    cleanup: Option<Box<dyn CleanupGuard>>,
}

impl PreparedRuntimeExtension {
    /// Builds a runtime extension value with no extra cleanup.
    #[must_use]
    pub fn new<T>(value: T) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        Self {
            type_id: TypeId::of::<T>(),
            type_name: type_name::<T>(),
            value: Box::new(value),
            cleanup: None,
        }
    }

    /// Builds a runtime extension value with a custom cleanup guard.
    #[must_use]
    pub fn with_cleanup<T>(value: T, cleanup: Box<dyn CleanupGuard>) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        Self {
            cleanup: Some(cleanup),
            ..Self::new(value)
        }
    }

    /// Builds a runtime extension value backed by a background task.
    #[must_use]
    pub fn from_task<T>(value: T, task: JoinHandle<()>) -> Self
    where
        T: Clone + Send + Sync + 'static,
    {
        Self::with_cleanup(value, Box::new(TaskCleanupGuard::new(task)))
    }
}

/// Factory that prepares a scenario runtime extension once node clients are
/// available.
#[async_trait]
pub trait RuntimeExtensionFactory<E: Application>: Send + Sync {
    /// Prepares one extension value for this scenario run.
    async fn prepare(
        &self,
        deployment: &E::Deployment,
        node_clients: NodeClients<E>,
    ) -> Result<PreparedRuntimeExtension, DynError>;
}

/// Type-indexed runtime extension store exposed through `RunContext`.
#[derive(Default)]
pub struct RuntimeExtensions {
    values: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl RuntimeExtensions {
    /// Returns a cloned extension value by type.
    #[must_use]
    pub fn get<T>(&self) -> Option<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        self.values
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref::<T>())
            .cloned()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

#[derive(Default)]
pub(crate) struct CleanupChain {
    guards: Vec<Box<dyn CleanupGuard>>,
}

impl CleanupChain {
    pub(crate) fn push(&mut self, guard: Box<dyn CleanupGuard>) {
        self.guards.push(guard);
    }

    pub(crate) fn push_optional(&mut self, guard: Option<Box<dyn CleanupGuard>>) {
        if let Some(guard) = guard {
            self.guards.push(guard);
        }
    }

    pub(crate) fn into_guard(self) -> Option<Box<dyn CleanupGuard>> {
        if self.guards.is_empty() {
            None
        } else {
            Some(Box::new(self))
        }
    }
}

impl CleanupGuard for CleanupChain {
    fn cleanup(mut self: Box<Self>) {
        while let Some(guard) = self.guards.pop() {
            guard.cleanup();
        }
    }
}

#[derive(Default)]
pub(crate) struct PreparedRuntimeExtensions {
    values: RuntimeExtensions,
    cleanup: CleanupChain,
}

impl PreparedRuntimeExtensions {
    pub(crate) fn into_parts(self) -> (RuntimeExtensions, Option<Box<dyn CleanupGuard>>) {
        (self.values, self.cleanup.into_guard())
    }

    fn insert(&mut self, extension: PreparedRuntimeExtension) -> Result<(), DynError> {
        let PreparedRuntimeExtension {
            type_id,
            type_name,
            value,
            cleanup,
        } = extension;

        if self.values.values.contains_key(&type_id) {
            return Err(format!("duplicate runtime extension type registered: {type_name}").into());
        }

        self.values.values.insert(type_id, value);
        self.cleanup.push_optional(cleanup);
        Ok(())
    }
}

pub(crate) async fn prepare_runtime_extensions<E: Application>(
    factories: &[Box<dyn RuntimeExtensionFactory<E>>],
    deployment: &E::Deployment,
    node_clients: NodeClients<E>,
) -> Result<PreparedRuntimeExtensions, DynError> {
    let mut prepared = PreparedRuntimeExtensions::default();

    for factory in factories {
        prepared.insert(factory.prepare(deployment, node_clients.clone()).await?)?;
    }

    Ok(prepared)
}

struct TaskCleanupGuard {
    handle: JoinHandle<()>,
}

impl TaskCleanupGuard {
    const fn new(handle: JoinHandle<()>) -> Self {
        Self { handle }
    }
}

impl CleanupGuard for TaskCleanupGuard {
    fn cleanup(self: Box<Self>) {
        self.handle.abort();
    }
}

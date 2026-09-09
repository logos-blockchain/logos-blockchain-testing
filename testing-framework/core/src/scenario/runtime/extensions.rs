use std::{
    any::{Any, TypeId, type_name},
    collections::HashMap,
};

use async_trait::async_trait;
use tokio::task::JoinHandle;

use super::context::CleanupGuard;
use crate::scenario::{Application, ClusterControlProfile, DynError, NodeClients};

type ErasedExtension = Box<dyn Any + Send + Sync>;
type ExtensionMergeFn = Box<
    dyn Fn(ErasedExtension, ErasedExtension) -> Result<ErasedExtension, DynError> + Send + Sync,
>;

/// Aggregated cluster control facts across every prepared extension.
///
/// The profile follows [`ClusterControlProfile::strongest`]; node control is
/// granted when any contributing cluster actually received a control handle,
/// so runtime settling applies only to scenarios the framework can perturb.
#[derive(Clone, Copy, Debug, Default)]
pub struct ClusterControlSummary {
    profile: Option<ClusterControlProfile>,
    node_control_granted: bool,
}

impl ClusterControlSummary {
    /// Returns the strongest control profile reported by any extension.
    #[must_use]
    pub const fn profile(&self) -> Option<ClusterControlProfile> {
        self.profile
    }

    /// Returns whether any contributing cluster granted a node control handle.
    #[must_use]
    pub const fn node_control_granted(&self) -> bool {
        self.node_control_granted
    }
}

/// Prepared runtime extension value plus optional cleanup.
pub struct PreparedRuntimeExtension {
    type_id: TypeId,
    type_name: &'static str,
    value: ErasedExtension,
    cleanup: Option<Box<dyn CleanupGuard>>,
    merge: Option<ExtensionMergeFn>,
    control_profile: Option<ClusterControlProfile>,
    node_control_granted: bool,
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
            merge: None,
            control_profile: None,
            node_control_granted: false,
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

    /// Builds a runtime extension value that can merge with another of the
    /// same type.
    ///
    /// When two prepared extensions share one value type and both provide a
    /// merge hook, registration combines them with `merge` instead of failing
    /// with a duplicate-type error. Cleanup guards from both extensions are
    /// chained in registration order.
    #[must_use]
    pub fn mergeable<T, F>(value: T, cleanup: Option<Box<dyn CleanupGuard>>, merge: F) -> Self
    where
        T: Clone + Send + Sync + 'static,
        F: Fn(T, T) -> Result<T, DynError> + Send + Sync + 'static,
    {
        let merge: ExtensionMergeFn = Box::new(move |left, right| {
            let left = downcast_for_merge::<T>(left)?;
            let right = downcast_for_merge::<T>(right)?;
            Ok(Box::new(merge(*left, *right)?))
        });

        Self {
            cleanup,
            merge: Some(merge),
            ..Self::new(value)
        }
    }

    /// Records the control profile of the clusters backing this extension.
    ///
    /// Deployers aggregate the strongest recorded profile (see
    /// [`ClusterControlProfile::strongest`]) instead of assuming an
    /// uncontrolled external cluster.
    #[must_use]
    pub fn with_control_profile(mut self, control_profile: ClusterControlProfile) -> Self {
        self.control_profile = Some(control_profile);
        self
    }

    /// Records whether a cluster backing this extension granted node control.
    ///
    /// The runner uses the aggregated grant to enable post-workload settle
    /// waits and the minimum stabilization cooldown only for scenarios whose
    /// nodes can actually be perturbed at runtime.
    #[must_use]
    pub const fn with_node_control_granted(mut self, granted: bool) -> Self {
        self.node_control_granted = granted;
        self
    }
}

fn downcast_for_merge<T: Any>(value: ErasedExtension) -> Result<Box<T>, DynError> {
    value.downcast::<T>().map_err(|_| {
        format!(
            "runtime extension merge received a mismatched value type, expected {}",
            type_name::<T>()
        )
        .into()
    })
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
    merges: HashMap<TypeId, ExtensionMergeFn>,
    control: ClusterControlSummary,
    cleanup: CleanupChain,
}

impl PreparedRuntimeExtensions {
    pub(crate) fn into_parts(
        self,
    ) -> (
        RuntimeExtensions,
        Option<Box<dyn CleanupGuard>>,
        ClusterControlSummary,
    ) {
        (self.values, self.cleanup.into_guard(), self.control)
    }

    fn insert(&mut self, extension: PreparedRuntimeExtension) -> Result<(), DynError> {
        let PreparedRuntimeExtension {
            type_id,
            type_name,
            value,
            cleanup,
            merge,
            control_profile,
            node_control_granted,
        } = extension;

        self.cleanup.push_optional(cleanup);
        self.record_control_profile(control_profile);
        self.control.node_control_granted |= node_control_granted;

        let Some(existing) = self.values.values.remove(&type_id) else {
            self.values.values.insert(type_id, value);
            if let Some(merge) = merge {
                self.merges.insert(type_id, merge);
            }
            return Ok(());
        };

        let merged = match (self.merges.get(&type_id), merge) {
            (Some(hook), Some(_)) => hook(existing, value)?,
            _ => {
                return Err(
                    format!("duplicate runtime extension type registered: {type_name}").into(),
                );
            }
        };
        self.values.values.insert(type_id, merged);
        Ok(())
    }

    fn record_control_profile(&mut self, control_profile: Option<ClusterControlProfile>) {
        self.control.profile = match (self.control.profile, control_profile) {
            (Some(current), Some(incoming)) => Some(current.strongest(incoming)),
            (current, incoming) => current.or(incoming),
        };
    }

    fn run_cleanup(self) {
        if let Some(guard) = self.cleanup.into_guard() {
            guard.cleanup();
        }
    }
}

pub(crate) async fn prepare_runtime_extensions<E: Application>(
    factories: &[Box<dyn RuntimeExtensionFactory<E>>],
    deployment: &E::Deployment,
    node_clients: NodeClients<E>,
) -> Result<PreparedRuntimeExtensions, DynError> {
    let mut prepared = PreparedRuntimeExtensions::default();

    for factory in factories {
        let extension = match factory.prepare(deployment, node_clients.clone()).await {
            Ok(extension) => extension,
            Err(error) => {
                prepared.run_cleanup();
                return Err(error);
            }
        };

        if let Err(error) = prepared.insert(extension) {
            prepared.run_cleanup();
            return Err(error);
        }
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use async_trait::async_trait;

    use super::{PreparedRuntimeExtension, RuntimeExtensionFactory, prepare_runtime_extensions};
    use crate::{
        scenario::{
            Application, ClusterControlProfile, DynError, NodeClients, internal::CleanupGuard,
        },
        topology::NodeCountTopology,
    };

    struct TestApp;

    #[async_trait]
    impl Application for TestApp {
        type Deployment = NodeCountTopology;
        type NodeClient = ();
        type NodeConfig = ();
    }

    struct RecordingCleanup {
        label: &'static str,
        events: Arc<Mutex<Vec<&'static str>>>,
    }

    impl CleanupGuard for RecordingCleanup {
        fn cleanup(self: Box<Self>) {
            self.events
                .lock()
                .expect("cleanup events lock")
                .push(self.label);
        }
    }

    struct TestFactory<T> {
        value: T,
        cleanup_label: &'static str,
        cleanup_events: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl<T> RuntimeExtensionFactory<TestApp> for TestFactory<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        async fn prepare(
            &self,
            _deployment: &NodeCountTopology,
            _node_clients: NodeClients<TestApp>,
        ) -> Result<PreparedRuntimeExtension, DynError> {
            Ok(PreparedRuntimeExtension::with_cleanup(
                self.value.clone(),
                Box::new(RecordingCleanup {
                    label: self.cleanup_label,
                    events: Arc::clone(&self.cleanup_events),
                }),
            ))
        }
    }

    fn factory<T>(
        value: T,
        cleanup_label: &'static str,
        cleanup_events: &Arc<Mutex<Vec<&'static str>>>,
    ) -> Box<dyn RuntimeExtensionFactory<TestApp>>
    where
        T: Clone + Send + Sync + 'static,
    {
        Box::new(TestFactory {
            value,
            cleanup_label,
            cleanup_events: Arc::clone(cleanup_events),
        })
    }

    #[tokio::test]
    async fn prepared_extensions_are_typed_and_clean_up_in_reverse_order() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let factories = vec![
            factory(7_u8, "first", &events),
            factory(9_u16, "second", &events),
        ];

        let prepared = prepare_runtime_extensions(
            &factories,
            &NodeCountTopology::new(0),
            NodeClients::default(),
        )
        .await
        .expect("different extension types should prepare");
        let (extensions, cleanup, _) = prepared.into_parts();

        assert_eq!(extensions.get::<u8>(), Some(7));
        assert_eq!(extensions.get::<u16>(), Some(9));

        cleanup.expect("extension cleanup chain").cleanup();
        assert_eq!(
            *events.lock().expect("cleanup events lock"),
            vec!["second", "first"]
        );
    }

    #[tokio::test]
    async fn duplicate_extension_types_are_rejected() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let factories = vec![
            factory(1_u8, "first", &events),
            factory(2_u8, "second", &events),
        ];

        let error = prepare_runtime_extensions(
            &factories,
            &NodeCountTopology::new(0),
            NodeClients::default(),
        )
        .await
        .err()
        .expect("duplicate extension types must fail");

        assert!(
            error
                .to_string()
                .contains("duplicate runtime extension type")
        );
    }

    #[tokio::test]
    async fn duplicate_extension_failure_runs_accumulated_cleanup() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let factories = vec![
            factory(1_u8, "first", &events),
            factory(2_u8, "second", &events),
        ];

        prepare_runtime_extensions(
            &factories,
            &NodeCountTopology::new(0),
            NodeClients::default(),
        )
        .await
        .err()
        .expect("duplicate extension types must fail");

        assert_eq!(
            *events.lock().expect("cleanup events lock"),
            vec!["second", "first"]
        );
    }

    #[tokio::test]
    async fn factory_failure_runs_cleanup_of_prepared_extensions() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let factories = vec![factory(7_u8, "prepared", &events), failing_factory()];

        let error = prepare_runtime_extensions(
            &factories,
            &NodeCountTopology::new(0),
            NodeClients::default(),
        )
        .await
        .err()
        .expect("failing factory must fail preparation");

        assert_eq!(error.to_string(), "factory exploded");
        assert_eq!(
            *events.lock().expect("cleanup events lock"),
            vec!["prepared"]
        );
    }

    #[tokio::test]
    async fn mergeable_extensions_of_one_type_are_combined() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let factories = vec![
            merge_factory(vec!["left"], "first", &events),
            merge_factory(vec!["right"], "second", &events),
        ];

        let prepared = prepare_runtime_extensions(
            &factories,
            &NodeCountTopology::new(0),
            NodeClients::default(),
        )
        .await
        .expect("mergeable extensions should prepare");
        let (extensions, cleanup, _) = prepared.into_parts();

        assert_eq!(
            extensions.get::<Vec<&'static str>>(),
            Some(vec!["left", "right"])
        );

        cleanup.expect("extension cleanup chain").cleanup();
        assert_eq!(
            *events.lock().expect("cleanup events lock"),
            vec!["second", "first"]
        );
    }

    #[tokio::test]
    async fn merge_requires_hooks_on_both_extensions() {
        let events = Arc::new(Mutex::new(Vec::new()));
        let factories = vec![
            merge_factory(vec!["left"], "first", &events),
            factory(vec!["right"], "second", &events),
        ];

        let error = prepare_runtime_extensions(
            &factories,
            &NodeCountTopology::new(0),
            NodeClients::default(),
        )
        .await
        .err()
        .expect("merge without both hooks must fail");

        assert!(
            error
                .to_string()
                .contains("duplicate runtime extension type")
        );
        assert_eq!(
            *events.lock().expect("cleanup events lock"),
            vec!["second", "first"]
        );
    }

    #[tokio::test]
    async fn strongest_control_profile_is_aggregated() {
        let factories = vec![
            profile_factory(1_u8, ClusterControlProfile::ExistingClusterAttached),
            profile_factory(2_u16, ClusterControlProfile::FrameworkManaged),
            profile_factory(3_u32, ClusterControlProfile::ExternalUncontrolled),
        ];

        let prepared = prepare_runtime_extensions(
            &factories,
            &NodeCountTopology::new(0),
            NodeClients::default(),
        )
        .await
        .expect("profile extensions should prepare");
        let (_, _, control) = prepared.into_parts();

        assert_eq!(
            control.profile(),
            Some(ClusterControlProfile::FrameworkManaged)
        );
        assert!(!control.node_control_granted());
    }

    #[tokio::test]
    async fn node_control_grants_aggregate_across_extensions() {
        let factories: Vec<Box<dyn RuntimeExtensionFactory<TestApp>>> = vec![
            profile_factory(1_u8, ClusterControlProfile::ExternalUncontrolled),
            Box::new(GrantFactory),
        ];

        let prepared = prepare_runtime_extensions(
            &factories,
            &NodeCountTopology::new(0),
            NodeClients::default(),
        )
        .await
        .expect("grant extensions should prepare");
        let (_, _, control) = prepared.into_parts();

        assert!(control.node_control_granted());
    }

    struct FailingFactory;

    #[async_trait]
    impl RuntimeExtensionFactory<TestApp> for FailingFactory {
        async fn prepare(
            &self,
            _deployment: &NodeCountTopology,
            _node_clients: NodeClients<TestApp>,
        ) -> Result<PreparedRuntimeExtension, DynError> {
            Err("factory exploded".into())
        }
    }

    fn failing_factory() -> Box<dyn RuntimeExtensionFactory<TestApp>> {
        Box::new(FailingFactory)
    }

    struct MergeFactory {
        value: Vec<&'static str>,
        cleanup_label: &'static str,
        cleanup_events: Arc<Mutex<Vec<&'static str>>>,
    }

    #[async_trait]
    impl RuntimeExtensionFactory<TestApp> for MergeFactory {
        async fn prepare(
            &self,
            _deployment: &NodeCountTopology,
            _node_clients: NodeClients<TestApp>,
        ) -> Result<PreparedRuntimeExtension, DynError> {
            let cleanup = Box::new(RecordingCleanup {
                label: self.cleanup_label,
                events: Arc::clone(&self.cleanup_events),
            });
            Ok(PreparedRuntimeExtension::mergeable(
                self.value.clone(),
                Some(cleanup),
                |mut left: Vec<&'static str>, right| {
                    left.extend(right);
                    Ok(left)
                },
            ))
        }
    }

    fn merge_factory(
        value: Vec<&'static str>,
        cleanup_label: &'static str,
        cleanup_events: &Arc<Mutex<Vec<&'static str>>>,
    ) -> Box<dyn RuntimeExtensionFactory<TestApp>> {
        Box::new(MergeFactory {
            value,
            cleanup_label,
            cleanup_events: Arc::clone(cleanup_events),
        })
    }

    struct ProfileFactory<T> {
        value: T,
        profile: ClusterControlProfile,
    }

    #[async_trait]
    impl<T> RuntimeExtensionFactory<TestApp> for ProfileFactory<T>
    where
        T: Clone + Send + Sync + 'static,
    {
        async fn prepare(
            &self,
            _deployment: &NodeCountTopology,
            _node_clients: NodeClients<TestApp>,
        ) -> Result<PreparedRuntimeExtension, DynError> {
            Ok(
                PreparedRuntimeExtension::new(self.value.clone())
                    .with_control_profile(self.profile),
            )
        }
    }

    fn profile_factory<T>(
        value: T,
        profile: ClusterControlProfile,
    ) -> Box<dyn RuntimeExtensionFactory<TestApp>>
    where
        T: Clone + Send + Sync + 'static,
    {
        Box::new(ProfileFactory { value, profile })
    }

    struct GrantFactory;

    #[async_trait]
    impl RuntimeExtensionFactory<TestApp> for GrantFactory {
        async fn prepare(
            &self,
            _deployment: &NodeCountTopology,
            _node_clients: NodeClients<TestApp>,
        ) -> Result<PreparedRuntimeExtension, DynError> {
            Ok(PreparedRuntimeExtension::new(2_u16).with_node_control_granted(true))
        }
    }
}

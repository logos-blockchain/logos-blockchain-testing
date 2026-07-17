use testing_framework_core::scenario::ClusterHandle;

/// Typed handle for a local cluster provisioned as part of an application
/// stack.
///
/// This is the same cluster runtime used by local scenarios and manual tests;
/// the app layer only gives it a role-specific name.
pub type LocalAppCluster<E> = ClusterHandle<E>;

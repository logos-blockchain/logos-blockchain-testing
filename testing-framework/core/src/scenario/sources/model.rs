/// Typed descriptor for an existing cluster.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExistingCluster {
    kind: ExistingClusterKind,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ExistingClusterKind {
    K8s {
        namespace: Option<String>,
        label_selector: String,
    },
    Compose {
        project: Option<String>,
        services: Vec<String>,
    },
}

impl ExistingCluster {
    #[must_use]
    pub fn for_k8s_selector(label_selector: String) -> Self {
        Self {
            kind: ExistingClusterKind::K8s {
                namespace: None,
                label_selector,
            },
        }
    }

    #[must_use]
    pub fn for_k8s_selector_in_namespace(namespace: String, label_selector: String) -> Self {
        Self {
            kind: ExistingClusterKind::K8s {
                namespace: Some(namespace),
                label_selector,
            },
        }
    }

    #[must_use]
    pub fn for_compose_project(project: String) -> Self {
        Self {
            kind: ExistingClusterKind::Compose {
                project: Some(project),
                services: Vec::new(),
            },
        }
    }

    #[must_use]
    pub fn for_compose_services(project: String, services: Vec<String>) -> Self {
        Self {
            kind: ExistingClusterKind::Compose {
                project: Some(project),
                services,
            },
        }
    }

    #[must_use]
    #[doc(hidden)]
    pub fn compose_project(&self) -> Option<&str> {
        match &self.kind {
            ExistingClusterKind::Compose { project, .. } => project.as_deref(),
            ExistingClusterKind::K8s { .. } => None,
        }
    }

    #[must_use]
    #[doc(hidden)]
    pub fn compose_services(&self) -> Option<&[String]> {
        match &self.kind {
            ExistingClusterKind::Compose { services, .. } => Some(services),
            ExistingClusterKind::K8s { .. } => None,
        }
    }

    #[must_use]
    #[doc(hidden)]
    pub fn k8s_namespace(&self) -> Option<&str> {
        match &self.kind {
            ExistingClusterKind::K8s { namespace, .. } => namespace.as_deref(),
            ExistingClusterKind::Compose { .. } => None,
        }
    }

    #[must_use]
    #[doc(hidden)]
    pub fn k8s_label_selector(&self) -> Option<&str> {
        match &self.kind {
            ExistingClusterKind::K8s { label_selector, .. } => Some(label_selector),
            ExistingClusterKind::Compose { .. } => None,
        }
    }
}

/// Static external node endpoint that should be included in the runtime
/// inventory.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExternalNodeSource {
    label: String,
    endpoint: String,
}

impl ExternalNodeSource {
    #[must_use]
    pub fn new(label: String, endpoint: String) -> Self {
        Self { label, endpoint }
    }

    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    #[must_use]
    pub fn endpoint(&self) -> &str {
        &self.endpoint
    }
}

/// High-level control/lifecycle expectation for a cluster surface.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterControlProfile {
    FrameworkManaged,
    ExistingClusterAttached,
    ExternalUncontrolled,
    ManualControlled,
}

impl ClusterControlProfile {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::FrameworkManaged => "framework-managed",
            Self::ExistingClusterAttached => "existing-cluster-attached",
            Self::ExternalUncontrolled => "external-uncontrolled",
            Self::ManualControlled => "manual-controlled",
        }
    }

    #[must_use]
    pub const fn framework_owns_lifecycle(self) -> bool {
        matches!(self, Self::FrameworkManaged)
    }

    /// Returns the profile granting the framework more runtime control.
    ///
    /// Profiles are totally ordered from strongest to weakest:
    /// `FrameworkManaged` > `ManualControlled` > `ExistingClusterAttached` >
    /// `ExternalUncontrolled`. A scenario combining several clusters reports
    /// the strongest profile so lifecycle-dependent behavior (such as the
    /// post-workload stabilization cooldown) is preserved.
    #[must_use]
    pub const fn strongest(self, other: Self) -> Self {
        if other.rank() > self.rank() {
            other
        } else {
            self
        }
    }

    const fn rank(self) -> u8 {
        match self {
            Self::FrameworkManaged => 3,
            Self::ManualControlled => 2,
            Self::ExistingClusterAttached => 1,
            Self::ExternalUncontrolled => 0,
        }
    }
}

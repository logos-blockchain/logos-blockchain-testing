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

/// High-level source mode of a scenario cluster.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClusterMode {
    Managed,
    ExistingCluster,
    ExternalOnly,
}

impl ClusterMode {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Managed => "managed",
            Self::ExistingCluster => "existing-cluster",
            Self::ExternalOnly => "external-only",
        }
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
}

/// Source model that makes invalid managed+attached combinations
/// unrepresentable by type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ScenarioSources {
    Managed {
        external: Vec<ExternalNodeSource>,
    },
    Attached {
        attach: ExistingCluster,
        external: Vec<ExternalNodeSource>,
    },
    ExternalOnly {
        external: Vec<ExternalNodeSource>,
    },
}

impl Default for ScenarioSources {
    fn default() -> Self {
        Self::Managed {
            external: Vec::new(),
        }
    }
}

impl ScenarioSources {
    #[must_use]
    pub(crate) fn with_external_node(mut self, node: ExternalNodeSource) -> Self {
        match &mut self {
            Self::Managed { external }
            | Self::Attached { external, .. }
            | Self::ExternalOnly { external } => external.push(node),
        }

        self
    }

    #[must_use]
    pub(crate) fn with_attach(self, attach: ExistingCluster) -> Self {
        let external = self.external_nodes().to_vec();

        Self::Attached { attach, external }
    }

    #[must_use]
    pub(crate) fn into_external_only(self) -> Self {
        let external = self.external_nodes().to_vec();

        Self::ExternalOnly { external }
    }

    #[must_use]
    pub(crate) fn existing_cluster(&self) -> Option<&ExistingCluster> {
        match self {
            Self::Attached { attach, .. } => Some(attach),
            Self::Managed { .. } | Self::ExternalOnly { .. } => None,
        }
    }

    #[must_use]
    pub(crate) fn external_nodes(&self) -> &[ExternalNodeSource] {
        match self {
            Self::Managed { external }
            | Self::Attached { external, .. }
            | Self::ExternalOnly { external } => external,
        }
    }

    #[must_use]
    pub(crate) const fn cluster_mode(&self) -> ClusterMode {
        match self {
            Self::Managed { .. } => ClusterMode::Managed,
            Self::Attached { .. } => ClusterMode::ExistingCluster,
            Self::ExternalOnly { .. } => ClusterMode::ExternalOnly,
        }
    }

    #[must_use]
    pub(crate) const fn control_profile(&self) -> ClusterControlProfile {
        match self.cluster_mode() {
            ClusterMode::Managed => ClusterControlProfile::FrameworkManaged,
            ClusterMode::ExistingCluster => ClusterControlProfile::ExistingClusterAttached,
            ClusterMode::ExternalOnly => ClusterControlProfile::ExternalUncontrolled,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ClusterControlProfile, ExistingCluster, ExternalNodeSource, ScenarioSources};

    #[test]
    fn managed_sources_map_to_framework_managed_control() {
        assert_eq!(
            ScenarioSources::default().control_profile(),
            ClusterControlProfile::FrameworkManaged,
        );
    }

    #[test]
    fn attached_sources_map_to_existing_cluster_control() {
        let sources = ScenarioSources::default()
            .with_attach(ExistingCluster::for_compose_project("project".to_owned()));

        assert_eq!(
            sources.control_profile(),
            ClusterControlProfile::ExistingClusterAttached,
        );
    }

    #[test]
    fn external_only_sources_map_to_uncontrolled_profile() {
        let sources = ScenarioSources::default()
            .with_external_node(ExternalNodeSource::new(
                "node".to_owned(),
                "http://node".to_owned(),
            ))
            .into_external_only();

        assert_eq!(
            sources.control_profile(),
            ClusterControlProfile::ExternalUncontrolled,
        );
    }
}

/// Typed attach source for existing clusters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExistingCluster {
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
    pub fn k8s(label_selector: String) -> Self {
        Self::K8s {
            namespace: None,
            label_selector,
        }
    }

    #[must_use]
    pub fn k8s_in_namespace(label_selector: String, namespace: String) -> Self {
        Self::K8s {
            namespace: Some(namespace),
            label_selector,
        }
    }

    #[must_use]
    pub fn compose(services: Vec<String>) -> Self {
        Self::Compose {
            project: None,
            services,
        }
    }

    #[must_use]
    pub fn compose_in_project(services: Vec<String>, project: String) -> Self {
        Self::Compose {
            project: Some(project),
            services,
        }
    }

    #[must_use]
    pub fn compose_project(&self) -> Option<&str> {
        match self {
            Self::Compose { project, .. } => project.as_deref(),
            Self::K8s { .. } => None,
        }
    }

    #[must_use]
    pub fn compose_services(&self) -> Option<&[String]> {
        match self {
            Self::Compose { services, .. } => Some(services),
            Self::K8s { .. } => None,
        }
    }

    #[must_use]
    pub fn k8s_namespace(&self) -> Option<&str> {
        match self {
            Self::K8s { namespace, .. } => namespace.as_deref(),
            Self::Compose { .. } => None,
        }
    }

    #[must_use]
    pub fn k8s_label_selector(&self) -> Option<&str> {
        match self {
            Self::K8s { label_selector, .. } => Some(label_selector),
            Self::Compose { .. } => None,
        }
    }
}

#[doc(hidden)]
pub type AttachSource = ExistingCluster;

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

/// Source model that makes invalid managed+attached combinations
/// unrepresentable by type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScenarioSources {
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
    pub const fn managed() -> Self {
        Self::Managed {
            external: Vec::new(),
        }
    }

    #[must_use]
    pub fn attached(attach: ExistingCluster) -> Self {
        Self::Attached {
            attach,
            external: Vec::new(),
        }
    }

    #[must_use]
    pub fn external_only(external: Vec<ExternalNodeSource>) -> Self {
        Self::ExternalOnly { external }
    }

    #[must_use]
    pub fn with_external_node(mut self, node: ExternalNodeSource) -> Self {
        match &mut self {
            Self::Managed { external }
            | Self::Attached { external, .. }
            | Self::ExternalOnly { external } => external.push(node),
        }

        self
    }

    #[must_use]
    pub fn with_attach(self, attach: ExistingCluster) -> Self {
        let external = self.external_nodes().to_vec();

        Self::Attached { attach, external }
    }

    #[must_use]
    pub fn into_external_only(self) -> Self {
        let external = self.external_nodes().to_vec();

        Self::ExternalOnly { external }
    }

    #[must_use]
    pub fn existing_cluster(&self) -> Option<&AttachSource> {
        match self {
            Self::Attached { attach, .. } => Some(attach),
            Self::Managed { .. } | Self::ExternalOnly { .. } => None,
        }
    }

    #[must_use]
    pub fn external_nodes(&self) -> &[ExternalNodeSource] {
        match self {
            Self::Managed { external }
            | Self::Attached { external, .. }
            | Self::ExternalOnly { external } => external,
        }
    }

    #[must_use]
    pub const fn is_managed(&self) -> bool {
        matches!(self, Self::Managed { .. })
    }

    #[must_use]
    pub const fn is_attached(&self) -> bool {
        matches!(self, Self::Attached { .. })
    }

    #[must_use]
    pub const fn uses_existing_cluster(&self) -> bool {
        self.is_attached()
    }

    #[must_use]
    pub const fn is_external_only(&self) -> bool {
        matches!(self, Self::ExternalOnly { .. })
    }
}

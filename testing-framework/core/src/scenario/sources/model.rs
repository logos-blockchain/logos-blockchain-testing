/// Typed attach source for existing clusters.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttachSource {
    K8s {
        namespace: Option<String>,
        label_selector: String,
    },
    Compose {
        project: Option<String>,
        services: Vec<String>,
    },
}

impl AttachSource {
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

/// Planned readiness strategy for mixed managed/attached/external sources.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Default)]
pub enum SourceReadinessPolicy {
    /// Phase 1 default: require every known node to pass readiness checks.
    #[default]
    AllReady,
    /// Optional relaxed policy for large/partial environments.
    Quorum,
    /// Future policy for per-source constraints (for example managed minimum
    /// plus overall quorum).
    SourceAware,
}

/// Source model that makes invalid managed+attached combinations
/// unrepresentable by type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScenarioSources {
    Managed {
        external: Vec<ExternalNodeSource>,
    },
    Attached {
        attach: AttachSource,
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
    pub fn attached(attach: AttachSource) -> Self {
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
    pub fn with_attach(self, attach: AttachSource) -> Self {
        let external = self.external_nodes().to_vec();

        Self::Attached { attach, external }
    }

    #[must_use]
    pub fn into_external_only(self) -> Self {
        let external = self.external_nodes().to_vec();

        Self::ExternalOnly { external }
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
    pub const fn is_external_only(&self) -> bool {
        matches!(self, Self::ExternalOnly { .. })
    }
}

use std::sync::Arc;

use super::{SourceOrchestrationMode, SourceOrchestrationPlan, SourceOrchestrationPlanError};
use crate::scenario::{
    Application, DynError, NodeClients, Scenario,
    runtime::{
        ApplicationExternalProvider, AttachProviderError, AttachedNode, SourceProviders,
        StaticManagedProvider,
        internal::{
            ExternalNode, ExternalProviderError, ManagedProviderError, ManagedProvisionedNode,
        },
    },
};

/// Resolved source nodes grouped by source class.
#[derive(Clone, Default)]
pub struct ResolvedSources<E: Application> {
    pub managed: Vec<ManagedProvisionedNode<E>>,
    pub attached: Vec<AttachedNode<E>>,
    pub external: Vec<ExternalNode<E>>,
}

/// Errors while resolving sources through unified providers.
#[derive(Debug, thiserror::Error)]
pub enum SourceResolveError {
    #[error(transparent)]
    Plan(#[from] SourceOrchestrationPlanError),
    #[error("managed source selected but deployer produced 0 managed nodes")]
    ManagedNodesMissing,
    #[error(transparent)]
    Managed(#[from] ManagedProviderError),
    #[error(transparent)]
    Attach(#[from] AttachProviderError),
    #[error(transparent)]
    External(#[from] ExternalProviderError),
}

/// Builds a source orchestration plan from scenario source configuration.
pub fn build_source_orchestration_plan<E: Application, Caps>(
    scenario: &Scenario<E, Caps>,
) -> Result<SourceOrchestrationPlan, SourceOrchestrationPlanError> {
    Ok(scenario.source_orchestration_plan().clone())
}

/// Resolves runtime source nodes via unified providers from orchestration plan.
pub async fn resolve_sources<E: Application>(
    plan: &SourceOrchestrationPlan,
    providers: &SourceProviders<E>,
) -> Result<ResolvedSources<E>, SourceResolveError> {
    match plan.mode() {
        SourceOrchestrationMode::Managed { managed, .. } => {
            let managed_nodes = providers.managed.provide(managed).await?;
            let external_nodes = providers.external.provide(plan.external_sources()).await?;

            Ok(ResolvedSources {
                managed: managed_nodes,
                attached: Vec::new(),
                external: external_nodes,
            })
        }
        SourceOrchestrationMode::Attached { attach, external } => {
            let attached_nodes = providers.attach.discover(attach).await?;
            let external_nodes = providers.external.provide(external).await?;

            Ok(ResolvedSources {
                managed: Vec::new(),
                attached: attached_nodes,
                external: external_nodes,
            })
        }
        SourceOrchestrationMode::ExternalOnly { external } => {
            let external_nodes = providers.external.provide(external).await?;

            Ok(ResolvedSources {
                managed: Vec::new(),
                attached: Vec::new(),
                external: external_nodes,
            })
        }
    }
}

/// Orchestrates scenario sources through the unified source orchestration path.
///
/// Current wiring is transitional:
/// - Managed mode is backed by prebuilt deployer-managed clients via
///   `StaticManagedProvider`.
/// - External nodes are resolved via `Application::external_node_client`.
/// - Attached nodes are discovered through the selected attach provider.
pub async fn orchestrate_sources<E: Application>(
    plan: &SourceOrchestrationPlan,
    node_clients: NodeClients<E>,
) -> Result<NodeClients<E>, DynError> {
    let providers: SourceProviders<E> = SourceProviders::default()
        .with_managed(Arc::new(StaticManagedProvider::new(
            node_clients.snapshot(),
        )))
        .with_external(Arc::new(ApplicationExternalProvider));

    orchestrate_sources_with_providers(plan, providers).await
}

/// Orchestrates scenario sources with caller-supplied provider set.
///
/// Deployer runtimes can use this to inject attach/external providers with
/// backend-specific discovery and control semantics.
pub async fn orchestrate_sources_with_providers<E: Application>(
    plan: &SourceOrchestrationPlan,
    providers: SourceProviders<E>,
) -> Result<NodeClients<E>, DynError> {
    let resolved = resolve_sources(plan, &providers).await?;

    if matches!(plan.mode(), SourceOrchestrationMode::Managed { .. }) && resolved.managed.is_empty()
    {
        return Err(SourceResolveError::ManagedNodesMissing.into());
    }

    Ok(NodeClients::new(
        resolved
            .managed
            .into_iter()
            .map(|node| node.client)
            .chain(resolved.attached.into_iter().map(|node| node.client))
            .chain(resolved.external.into_iter().map(|node| node.client))
            .collect(),
    ))
}

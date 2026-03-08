//! Scenario orchestration primitives shared by integration tests and runners.

use std::error::Error;

mod builder_ext;
mod builder_ops;
mod capabilities;
mod common_builder_ext;
mod control;
mod definition;
mod deployment_policy;
mod expectation;
mod observability;
mod runtime;
mod sources;
mod workload;

pub type DynError = Box<dyn Error + Send + Sync + 'static>;

pub use builder_ext::{BuilderInputError, ObservabilityBuilderExt};
#[doc(hidden)]
pub use builder_ops::CoreBuilderAccess;
pub use capabilities::{
    NodeControlCapability, ObservabilityCapability, PeerSelection, RequiresNodeControl,
    StartNodeOptions, StartedNode,
};
pub use common_builder_ext::CoreBuilderExt;
pub use control::{ClusterWaitHandle, NodeControlHandle};
#[doc(hidden)]
pub use definition::{
    Builder as CoreBuilder, // internal adapter-facing core builder
    NodeControlScenarioBuilder,
    ObservabilityScenarioBuilder,
};
pub use definition::{Scenario, ScenarioBuildError, ScenarioBuilder};
pub use deployment_policy::{CleanupPolicy, DeploymentPolicy, RetryPolicy};
pub use expectation::Expectation;
pub use observability::{ObservabilityCapabilityProvider, ObservabilityInputs};
#[doc(hidden)]
pub use runtime::{
    ApplicationExternalProvider, AttachProvider, AttachProviderError, AttachedNode, CleanupGuard,
    FeedHandle, ManagedSource, RuntimeAssembly, SourceOrchestrationPlan, SourceProviders,
    StaticManagedProvider, build_source_orchestration_plan, orchestrate_sources,
    orchestrate_sources_with_providers, resolve_sources,
};
pub use runtime::{
    Deployer, Feed, FeedRuntime, HttpReadinessRequirement, NodeClients, ReadinessError, RunContext,
    RunHandle, RunMetrics, Runner, ScenarioError, StabilizationConfig,
    metrics::{
        CONSENSUS_PROCESSED_BLOCKS, CONSENSUS_TRANSACTIONS_TOTAL, Metrics, MetricsError,
        PrometheusEndpoint, PrometheusInstantSample,
    },
    spawn_feed, wait_for_http_ports, wait_for_http_ports_with_host,
    wait_for_http_ports_with_host_and_requirement, wait_for_http_ports_with_requirement,
    wait_http_readiness, wait_until_stable,
};
pub use sources::{ExistingCluster, ExternalNodeSource};
pub use workload::Workload;

pub use crate::env::Application;

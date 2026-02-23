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
pub use control::NodeControlHandle;
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
pub use runtime::{
    BorrowedNode, BorrowedOrigin, CleanupGuard, Deployer, Feed, FeedHandle, FeedRuntime,
    HttpReadinessRequirement, ManagedNode, ManagedSource, NodeClients, NodeHandle, NodeInventory,
    ReadinessError, RunContext, RunHandle, RunMetrics, Runner, ScenarioError,
    SourceOrchestrationPlan, SourceProviders, StabilizationConfig, StaticManagedProvider,
    build_source_orchestration_plan,
    metrics::{
        CONSENSUS_PROCESSED_BLOCKS, CONSENSUS_TRANSACTIONS_TOTAL, Metrics, MetricsError,
        PrometheusEndpoint, PrometheusInstantSample,
    },
    orchestrate_sources, resolve_sources, spawn_feed, wait_for_http_ports,
    wait_for_http_ports_with_host, wait_for_http_ports_with_host_and_requirement,
    wait_for_http_ports_with_requirement, wait_http_readiness, wait_until_stable,
};
pub use sources::{AttachSource, ExternalNodeSource, ScenarioSources, SourceReadinessPolicy};
pub use workload::Workload;

pub use crate::env::Application;

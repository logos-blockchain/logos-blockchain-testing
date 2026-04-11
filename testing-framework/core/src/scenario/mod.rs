//! Scenario orchestration primitives shared by integration tests and runners.

use std::error::Error;

mod builder_ext;
mod builder_ops;
mod capabilities;
mod client;
mod common_builder_ext;
mod config;
mod control;
mod definition;
mod deployment_policy;
mod expectation;
pub mod internal;
mod observability;
mod runtime;
mod sources;
mod workload;

pub type DynError = Box<dyn Error + Send + Sync + 'static>;

pub use builder_ext::{BuilderInputError, ObservabilityBuilderExt};
pub use capabilities::{
    NodeControlCapability, ObservabilityCapability, PeerSelection, RequiresNodeControl,
    StartNodeOptions, StartedNode,
};
pub use client::NodeAccess;
pub use common_builder_ext::CoreBuilderExt;
pub use config::{
    ClusterNodeConfigApplication, ClusterNodeView, ClusterPeerView, serialize_cluster_yaml_config,
};
pub use control::{ClusterWaitHandle, NodeControlHandle};
pub use definition::{Scenario, ScenarioBuildError, ScenarioBuilder};
pub use deployment_policy::{CleanupPolicy, DeploymentPolicy, RetryPolicy};
pub use expectation::Expectation;
pub use observability::{ObservabilityCapabilityProvider, ObservabilityInputs};
pub use runtime::{
    Deployer, HttpReadinessRequirement, NodeClients, ReadinessError, RunContext, RunHandle,
    RunMetrics, Runner, ScenarioError, StabilizationConfig,
    metrics::{
        CONSENSUS_PROCESSED_BLOCKS, CONSENSUS_TRANSACTIONS_TOTAL, Metrics, MetricsError,
        PrometheusEndpoint, PrometheusInstantSample,
    },
    wait_for_http_ports, wait_for_http_ports_with_host,
    wait_for_http_ports_with_host_and_requirement, wait_for_http_ports_with_requirement,
    wait_http_readiness, wait_until_stable,
};
pub use sources::{
    ClusterControlProfile, ClusterMode, ExistingCluster, ExternalNodeSource, IntoExistingCluster,
};
pub use workload::Workload;

pub use crate::env::Application;

//! Composable application deployment for heterogeneous test systems.
//!
//! The core testing framework models one
//! [`Application`](testing_framework_core::scenario::Application)
//! and its uniform node topology. This crate adds an application layer for
//! scenarios that also need singleton processes, additional clusters,
//! attached services, or a stack of several different applications.
//!
//! Implement [`AppDeployment`] in the application repository, compose child
//! deployments through [`DeployContext`], and expose typed handles to workloads
//! with [`AppRunContextExt`]. TF adapters register managed resources with the
//! scenario cleanup path, while attached and external apps remain explicit.

#![warn(missing_docs)]

mod cleanup;
mod context;
mod deployment;
mod error;
mod extension;
mod host;
mod local;
mod process;
mod registry;

pub use context::DeployContext;
pub use deployment::{AppDeployment, AppHandle};
pub use error::AppDeployError;
pub use extension::{AppDeploymentFactory, AppRunContextExt, AppScenarioBuilderExt};
pub use host::{
    AppHost, AppHostEnv, AppHostLocalDeployer, AppHostScenarioBuilder, AppHostTopology,
};
pub use local::LocalAppCluster;
pub use process::{LocalProcessApp, LocalProcessHandle};
pub use registry::{AppRuntime, HandleRegistry};

use std::time::Duration;

use testing_framework_core::scenario::{Deployer, ScenarioBuilder};
use testing_framework_runner_compose::ComposeDeployer;
use testing_framework_runner_k8s::K8sDeployer;
use testing_framework_runner_local::LocalDeployer;
use testing_framework_workflows::{ChaosBuilderExt, ScenarioBuilderExt};

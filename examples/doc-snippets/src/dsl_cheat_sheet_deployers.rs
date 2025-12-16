use testing_framework_runner_compose::ComposeDeployer;
use testing_framework_runner_k8s::K8sDeployer;
use testing_framework_runner_local::LocalDeployer;

pub fn deployers() {
    // Local processes
    let _deployer = LocalDeployer::default();

    // Docker Compose
    let _deployer = ComposeDeployer::default();

    // Kubernetes
    let _deployer = K8sDeployer::default();
}

use testing_framework_runner_k8s::{BinaryConfigK8sSpec, K8sBinaryApp};

use crate::PubSubEnv;

const CONTAINER_CONFIG_PATH: &str = "/etc/pubsub/config.yaml";
const CONTAINER_HTTP_PORT: u16 = 8080;
const SERVICE_TESTING_PORT: u16 = 8081;
const NODE_NAME_PREFIX: &str = "pubsub-node";

impl K8sBinaryApp for PubSubEnv {
    fn k8s_binary_spec() -> BinaryConfigK8sSpec {
        BinaryConfigK8sSpec::conventional(
            "pubsub",
            NODE_NAME_PREFIX,
            "/usr/local/bin/pubsub-node",
            CONTAINER_CONFIG_PATH,
            CONTAINER_HTTP_PORT,
            SERVICE_TESTING_PORT,
        )
    }
}

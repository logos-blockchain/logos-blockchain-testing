use testing_framework_runner_k8s::{BinaryConfigK8sSpec, K8sBinaryApp};

use crate::KvEnv;

const CONTAINER_CONFIG_PATH: &str = "/etc/kvstore/config.yaml";
const CONTAINER_HTTP_PORT: u16 = 8080;
const SERVICE_TESTING_PORT: u16 = 8081;
const NODE_NAME_PREFIX: &str = "kvstore-node";

impl K8sBinaryApp for KvEnv {
    fn k8s_binary_spec() -> BinaryConfigK8sSpec {
        BinaryConfigK8sSpec::conventional(
            "kvstore",
            NODE_NAME_PREFIX,
            "/usr/local/bin/kvstore-node",
            CONTAINER_CONFIG_PATH,
            CONTAINER_HTTP_PORT,
            SERVICE_TESTING_PORT,
        )
    }
}

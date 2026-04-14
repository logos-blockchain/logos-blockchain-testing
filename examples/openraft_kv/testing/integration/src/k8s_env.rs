use testing_framework_runner_k8s::{BinaryConfigK8sSpec, K8sBinaryApp};

use crate::OpenRaftKvEnv;

const CONTAINER_CONFIG_PATH: &str = "/etc/openraft-kv/config.yaml";
const CONTAINER_HTTP_PORT: u16 = 8080;
const SERVICE_TESTING_PORT: u16 = 8081;
const NODE_NAME_PREFIX: &str = "openraft-kv-node";

impl K8sBinaryApp for OpenRaftKvEnv {
    fn k8s_binary_spec() -> BinaryConfigK8sSpec {
        BinaryConfigK8sSpec::conventional(
            "openraft-kv",
            NODE_NAME_PREFIX,
            "/usr/local/bin/openraft-kv-node",
            CONTAINER_CONFIG_PATH,
            CONTAINER_HTTP_PORT,
            SERVICE_TESTING_PORT,
        )
    }
}

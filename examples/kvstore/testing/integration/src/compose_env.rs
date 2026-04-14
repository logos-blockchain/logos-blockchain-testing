use testing_framework_runner_compose::{BinaryConfigNodeSpec, ComposeBinaryApp};

use crate::KvEnv;

const NODE_CONFIG_PATH: &str = "/etc/kvstore/config.yaml";

impl ComposeBinaryApp for KvEnv {
    fn compose_node_spec() -> BinaryConfigNodeSpec {
        BinaryConfigNodeSpec::conventional(
            "/usr/local/bin/kvstore-node",
            NODE_CONFIG_PATH,
            vec![8080, 8081],
        )
    }
}

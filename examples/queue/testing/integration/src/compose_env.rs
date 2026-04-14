use testing_framework_runner_compose::{BinaryConfigNodeSpec, ComposeBinaryApp};

use crate::QueueEnv;

const NODE_CONFIG_PATH: &str = "/etc/queue/config.yaml";

impl ComposeBinaryApp for QueueEnv {
    fn compose_node_spec() -> BinaryConfigNodeSpec {
        BinaryConfigNodeSpec::conventional(
            "/usr/local/bin/queue-node",
            NODE_CONFIG_PATH,
            vec![8080, 8081],
        )
    }
}

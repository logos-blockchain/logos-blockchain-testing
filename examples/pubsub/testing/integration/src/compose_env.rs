use testing_framework_runner_compose::{BinaryConfigNodeSpec, ComposeBinaryApp};

use crate::PubSubEnv;

const NODE_CONFIG_PATH: &str = "/etc/pubsub/config.yaml";

impl ComposeBinaryApp for PubSubEnv {
    fn compose_node_spec() -> BinaryConfigNodeSpec {
        BinaryConfigNodeSpec::conventional(
            "/usr/local/bin/pubsub-node",
            NODE_CONFIG_PATH,
            vec![8080, 8081],
        )
    }
}

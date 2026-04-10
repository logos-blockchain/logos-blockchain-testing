use testing_framework_runner_compose::{BinaryConfigNodeSpec, ComposeBinaryApp};

use crate::SchedulerEnv;

const NODE_CONFIG_PATH: &str = "/etc/scheduler/config.yaml";

impl ComposeBinaryApp for SchedulerEnv {
    fn compose_node_spec() -> BinaryConfigNodeSpec {
        BinaryConfigNodeSpec::conventional(
            "/usr/local/bin/scheduler-node",
            NODE_CONFIG_PATH,
            vec![8080, 8081],
        )
    }
}

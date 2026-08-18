use std::{
    net::{SocketAddr, TcpStream},
    process::Command,
    time::Duration,
};

use kvstore_runtime_ext::{KvExistingClusterApp, KvLocalDeployer};
use kvstore_runtime_workloads::{
    KvBuilderExt, KvClusterAccessible, KvConverges, KvScenarioBuilder, KvWriteWorkload,
};
use testing_framework_core::scenario::Deployer;

const NODE_COUNT: usize = 2;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_kvstore_runs_a_complete_scenario() {
    let mut scenario =
        KvScenarioBuilder::with_existing_kvstore_app(KvExistingClusterApp::nodes(NODE_COUNT))
            .with_node_control()
            .with_run_duration(Duration::from_secs(10))
            .with_workload(KvClusterAccessible::new(NODE_COUNT))
            .with_workload(
                KvWriteWorkload::new()
                    .operations(10)
                    .key_count(2)
                    .rate_per_sec(20)
                    .key_prefix("local-smoke"),
            )
            .with_expectation(KvConverges::new("local-smoke", 2).timeout(Duration::from_secs(10)))
            .build()
            .expect("local kvstore smoke scenario must build");

    let runner = KvLocalDeployer::default()
        .deploy(&scenario)
        .await
        .expect("local deployer must start the kvstore cluster");
    assert_eq!(runner.context().node_clients().len(), NODE_COUNT);

    let handle = runner
        .run(&mut scenario)
        .await
        .expect("local kvstore workloads and expectations must pass");
    assert_eq!(handle.context().node_clients().len(), NODE_COUNT);

    let node_control = handle
        .context()
        .node_control()
        .expect("local runner must expose node control");
    let node_pids = (0..NODE_COUNT)
        .map(|index| {
            node_control
                .node_pid(&format!("node-{index}"))
                .expect("running local node must expose its process id")
        })
        .collect::<Vec<_>>();

    let node_addresses = handle
        .context()
        .node_clients()
        .snapshot()
        .iter()
        .map(|client| {
            let url = client.base_url();
            let host = url.host_str().expect("kvstore client url must have a host");
            let port = url
                .port_or_known_default()
                .expect("kvstore client url must have a port");
            format!("{host}:{port}")
                .parse::<SocketAddr>()
                .expect("kvstore client url must resolve to a socket address")
        })
        .collect::<Vec<_>>();

    drop(handle);

    for (pid, address) in node_pids.into_iter().zip(node_addresses) {
        assert!(
            !process_is_running(pid),
            "local node process survived cleanup: {pid}"
        );
        assert!(
            TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err(),
            "local node still accepts connections after run cleanup: {address}"
        );
    }
}

fn process_is_running(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "pid="])
        .output()
        .expect("ps must be available for the local process cleanup assertion");

    output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
}

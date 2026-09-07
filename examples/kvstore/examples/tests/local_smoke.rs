use std::{
    net::{SocketAddr, TcpStream},
    process::Command,
    time::Duration,
};

use async_trait::async_trait;
use kvstore_runtime_workloads::{
    KvClusterAccessible, KvConverges, KvEnv, KvTopology, KvWriteWorkload,
};
use serde::{Deserialize, Serialize};
use testing_framework_app::{
    AppHost, AppHostDeployer, AppHostEnv, AppRunContextExt as _, AppScenarioBuilderExt as _,
    ClusterApp,
};
use testing_framework_core::scenario::{ClusterHandle, DynError, RunContext, Workload};

const NODE_COUNT: usize = 2;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_kvstore_runs_a_complete_scenario() {
    let mut scenario = AppHost::scenario()
        .with_app(ClusterApp::<KvEnv>::new(KvTopology::new(NODE_COUNT)))
        .with_run_duration(Duration::from_secs(10))
        .with_workload(KvClusterAccessible::new(NODE_COUNT))
        .with_workload(
            KvWriteWorkload::new()
                .operations(10)
                .key_count(2)
                .rate_per_sec(20)
                .key_prefix("local-smoke"),
        )
        .with_workload(KvRestartExercise::new(NODE_COUNT))
        .with_expectation(KvConverges::new("local-smoke", 2).timeout(Duration::from_secs(10)))
        .build()
        .expect("local kvstore smoke scenario must build");

    let runner = AppHostDeployer
        .deploy(&scenario)
        .await
        .expect("app host deployer must start the kvstore cluster");
    let cluster = runner
        .context()
        .require_app::<ClusterHandle<KvEnv>>()
        .expect("kvstore cluster handle must be exposed");
    assert_eq!(cluster.clients().len(), NODE_COUNT);

    let handle = runner
        .run(&mut scenario)
        .await
        .expect("local kvstore workloads and expectations must pass");
    assert_eq!(cluster.clients().len(), NODE_COUNT);

    let node_pids = (0..NODE_COUNT)
        .map(|index| {
            cluster
                .node_pid(&format!("node-{index}"))
                .expect("running local node must expose its process id")
        })
        .collect::<Vec<_>>();

    let node_addresses = cluster
        .clients()
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

#[derive(Clone)]
struct KvRestartExercise {
    expected_nodes: usize,
}

impl KvRestartExercise {
    const fn new(expected_nodes: usize) -> Self {
        Self { expected_nodes }
    }
}

#[async_trait]
impl Workload<AppHostEnv> for KvRestartExercise {
    fn name(&self) -> &str {
        "kv_restart_exercise"
    }

    async fn start(&self, ctx: &RunContext<AppHostEnv>) -> Result<(), DynError> {
        let cluster = ctx.require_app::<ClusterHandle<KvEnv>>()?;

        ensure_cluster_shape(&cluster, self.expected_nodes)?;
        put_value(&cluster, "before-restart").await?;
        cluster.restart_node("node-1").await?;
        cluster.wait_node_ready("node-1").await?;
        put_value(&cluster, "after-restart").await?;

        Ok(())
    }
}

fn ensure_cluster_shape(
    cluster: &ClusterHandle<KvEnv>,
    expected_nodes: usize,
) -> Result<(), DynError> {
    if cluster.node_count() != expected_nodes || cluster.clients().len() != expected_nodes {
        return Err(format!("kv smoke cluster expected {expected_nodes} nodes").into());
    }

    if cluster.node_client("node-1").is_none() {
        return Err("kv smoke cluster cannot access node-1 client".into());
    }

    if cluster.node_pid("node-1").is_none() {
        return Err("kv smoke cluster cannot access node-1 process id".into());
    }

    Ok(())
}

async fn put_value(cluster: &ClusterHandle<KvEnv>, value: &str) -> Result<(), DynError> {
    let Some(client) = cluster.first_client() else {
        return Err("kv smoke cluster has no clients".into());
    };

    let response: KvPutResponse = client
        .put(
            "/kv/local-smoke-restart",
            &KvPutRequest {
                value: value.to_owned(),
                expected_version: None,
            },
        )
        .await?;

    if !response.applied {
        return Err(format!("kv smoke write was rejected for value {value}").into());
    }

    Ok(())
}

#[derive(Serialize)]
struct KvPutRequest {
    value: String,
    expected_version: Option<u64>,
}

#[derive(Deserialize)]
struct KvPutResponse {
    applied: bool,
}

fn process_is_running(pid: u32) -> bool {
    let output = Command::new("ps")
        .args(["-p", &pid.to_string(), "-o", "pid="])
        .output()
        .expect("ps must be available for the local process cleanup assertion");

    output.status.success() && !String::from_utf8_lossy(&output.stdout).trim().is_empty()
}

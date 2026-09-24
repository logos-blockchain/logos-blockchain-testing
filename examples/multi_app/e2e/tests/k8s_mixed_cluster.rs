//! Mixed application lifecycle coverage using the existing Kind example images.
//! Set K8S_RUNNER_REQUIRE_CLUSTER=1 to require a real Kubernetes run.

use std::time::Duration;

use anyhow::{Context as _, Result, anyhow, ensure};
use kvstore_runtime_ext::KvEnv;
use pubsub_runtime_ext::PubSubEnv;
use serde_json::{Value, json};
use testing_framework_app::{AppHostEnv, AppHostTopology, ClusterApp, DeployContext};
use testing_framework_core::{
    scenario::{ClusterHandle, NodeClients},
    topology::ClusterTopology,
};
use testing_framework_runner_k8s::K8sClusterProvisioner;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn different_apps_share_a_k8s_context_across_restarts() -> Result<()> {
    if std::env::var("K8S_RUNNER_REQUIRE_CLUSTER").as_deref() != Ok("1") {
        eprintln!("skipping mixed K8s test; set K8S_RUNNER_REQUIRE_CLUSTER=1 to run it");
        return Ok(());
    }

    let mut ctx = DeployContext::<AppHostEnv, _>::new_with_provisioner(
        AppHostTopology,
        NodeClients::default(),
        K8sClusterProvisioner,
    );

    ctx.deploy_and_expose(ClusterApp::<KvEnv>::new(ClusterTopology::new(1)).with_name("kv"))
        .await
        .map_err(|error| anyhow!(error))
        .context("deploying kvstore")?;

    ctx.deploy_and_expose(
        ClusterApp::<PubSubEnv>::new(ClusterTopology::new(1)).with_name("pubsub"),
    )
    .await
    .map_err(|error| anyhow!(error))
    .context("deploying pubsub in the same context")?;

    // Resolve both typed handles from the shared registry, as a workload does.
    let kv = ctx.require::<ClusterHandle<KvEnv>>()?;
    let pubsub = ctx.require::<ClusterHandle<PubSubEnv>>()?;

    let kv_namespace = kv
        .attachment()
        .and_then(|attachment| attachment.k8s_namespace())
        .context("kvstore namespace missing")?
        .to_owned();

    let pubsub_namespace = pubsub
        .attachment()
        .and_then(|attachment| attachment.k8s_namespace())
        .context("pubsub namespace missing")?
        .to_owned();

    ensure!(
        kv_namespace != pubsub_namespace,
        "different apps must have separate namespaces"
    );

    kv.wait_network_ready()
        .await
        .map_err(|error| anyhow!(error))?;
    pubsub
        .wait_network_ready()
        .await
        .map_err(|error| anyhow!(error))?;

    assert_kv_roundtrip(&kv, "before-restart").await?;
    assert_pubsub_roundtrip(&pubsub, "before-restart").await?;

    kv.restart_node("node-0")
        .await
        .map_err(|error| anyhow!(error))
        .context("restarting kvstore while pubsub is running")?;
    kv.wait_node_ready("node-0")
        .await
        .map_err(|error| anyhow!(error))?;

    assert_kv_roundtrip(&kv, "after-kv-restart").await?;
    assert_pubsub_roundtrip(&pubsub, "after-kv-restart").await?;

    pubsub
        .restart_node("node-0")
        .await
        .map_err(|error| anyhow!(error))
        .context("restarting pubsub while kvstore is running")?;
    pubsub
        .wait_node_ready("node-0")
        .await
        .map_err(|error| anyhow!(error))?;

    assert_pubsub_roundtrip(&pubsub, "after-pubsub-restart").await?;

    // The pubsub restart must not erase the other application's state.
    let client = kv.node_client("node-0").context("kvstore client missing")?;
    let stored: Value =
        tokio::time::timeout(Duration::from_secs(30), client.get("/kv/after-kv-restart"))
            .await
            .context("reading preserved kvstore state timed out")??;

    ensure!(
        stored["record"]["value"] == "after-kv-restart",
        "restarting pubsub changed kvstore data"
    );

    assert_kv_roundtrip(&kv, "after-pubsub-restart").await?;

    drop(kv);
    drop(pubsub);
    drop(ctx);

    // Cleanup belongs to the deployment context, not individual handle clones.
    for namespace in [kv_namespace, pubsub_namespace] {
        let output = tokio::process::Command::new("kubectl")
            .args([
                "wait",
                "--for=delete",
                &format!("namespace/{namespace}"),
                "--timeout=90s",
            ])
            .output()
            .await
            .context("checking namespace cleanup")?;

        ensure!(
            output.status.success(),
            "namespace {namespace} was not cleaned up: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    Ok(())
}

async fn assert_kv_roundtrip(cluster: &ClusterHandle<KvEnv>, value: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), async {
        let client = cluster
            .node_client("node-0")
            .context("kvstore client missing")?;
        let path = format!("/kv/{value}");

        let written: Value = client.put(&path, &json!({"value": value})).await?;
        ensure!(written["applied"] == true, "kvstore rejected the write");

        let stored: Value = client.get(&path).await?;
        ensure!(
            stored["record"]["value"] == value,
            "kvstore returned the wrong value"
        );

        Ok(())
    })
    .await
    .context("kvstore roundtrip timed out")?
}

async fn assert_pubsub_roundtrip(cluster: &ClusterHandle<PubSubEnv>, payload: &str) -> Result<()> {
    tokio::time::timeout(Duration::from_secs(30), async {
        let client = cluster
            .node_client("node-0")
            .context("pubsub client missing")?;
        let mut session = client.connect().await?;

        // One connection guarantees subscribe is processed before publish.
        session.subscribe("mixed-app").await?;
        session.publish("mixed-app", payload.to_owned()).await?;

        loop {
            if let Some(event) = session.next_event_timeout(Duration::from_secs(1)).await? {
                ensure!(
                    event.topic == "mixed-app" && event.payload == payload,
                    "pubsub returned the wrong event"
                );
                break;
            }
        }

        session.close().await?;

        Ok(())
    })
    .await
    .context("pubsub roundtrip timed out")?
}

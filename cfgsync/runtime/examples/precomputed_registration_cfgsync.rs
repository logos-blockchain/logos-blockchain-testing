use cfgsync_adapter::MaterializedArtifacts;
use cfgsync_artifacts::{ArtifactFile, ArtifactSet};
use cfgsync_core::NodeRegistration;
use cfgsync_runtime::{Client, OutputMap, serve};
use tempfile::tempdir;
use tokio::time::{Duration, sleep};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = 4401;
    let artifacts = MaterializedArtifacts::from_nodes([
        (
            "node-1".to_owned(),
            ArtifactSet::new(vec![ArtifactFile::new(
                "/config.yaml".to_string(),
                "id: node-1\n".to_string(),
            )]),
        ),
        (
            "node-2".to_owned(),
            ArtifactSet::new(vec![ArtifactFile::new(
                "/config.yaml".to_string(),
                "id: node-2\n".to_string(),
            )]),
        ),
    ])
    .with_shared(ArtifactSet::new(vec![ArtifactFile::new(
        "/shared/cluster.yaml".to_string(),
        "cluster: demo\n".to_string(),
    )]));

    let server = tokio::spawn(async move { serve(port, artifacts).await });

    // Give the server a moment to bind before clients register.
    sleep(Duration::from_millis(100)).await;

    let node_1_dir = tempdir()?;
    let node_1_outputs = OutputMap::config_and_shared(
        node_1_dir.path().join("config.yaml"),
        node_1_dir.path().join("shared"),
    );
    let node_1 = NodeRegistration::new("node-1".to_string(), "127.0.0.1".parse()?);

    Client::new("http://127.0.0.1:4401")
        .fetch_and_write(&node_1, &node_1_outputs)
        .await?;

    println!(
        "node-1 config:\n{}",
        std::fs::read_to_string(node_1_dir.path().join("config.yaml"))?
    );

    // A later node still uses the same registration/fetch flow. The artifacts
    // were already known; registration only gates delivery.
    sleep(Duration::from_millis(250)).await;

    let node_2_dir = tempdir()?;
    let node_2_outputs = OutputMap::config_and_shared(
        node_2_dir.path().join("config.yaml"),
        node_2_dir.path().join("shared"),
    );
    let node_2 = NodeRegistration::new("node-2".to_string(), "127.0.0.2".parse()?);

    Client::new("http://127.0.0.1:4401")
        .fetch_and_write(&node_2, &node_2_outputs)
        .await?;

    println!(
        "node-2 config:\n{}",
        std::fs::read_to_string(node_2_dir.path().join("config.yaml"))?
    );
    println!(
        "shared artifact:\n{}",
        std::fs::read_to_string(node_2_dir.path().join("shared/shared/cluster.yaml"))?
    );

    server.abort();
    Ok(())
}

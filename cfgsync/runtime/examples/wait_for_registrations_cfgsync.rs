use cfgsync_adapter::{
    DynCfgsyncError, MaterializationResult, MaterializedArtifacts, RegistrationSnapshot,
    RegistrationSnapshotMaterializer,
};
use cfgsync_artifacts::{ArtifactFile, ArtifactSet};
use cfgsync_core::NodeRegistration;
use cfgsync_runtime::{Client, OutputMap, serve};
use tempfile::tempdir;
use tokio::time::{Duration, sleep};

struct ThresholdMaterializer;

impl RegistrationSnapshotMaterializer for ThresholdMaterializer {
    fn materialize_snapshot(
        &self,
        registrations: &RegistrationSnapshot,
    ) -> Result<MaterializationResult, DynCfgsyncError> {
        if registrations.len() < 2 {
            return Ok(MaterializationResult::NotReady);
        }

        let nodes = registrations.iter().map(|registration| {
            (
                registration.identifier.clone(),
                ArtifactSet::new(vec![ArtifactFile::new(
                    "/config.yaml",
                    format!("id: {}\ncluster_ready: true\n", registration.identifier),
                )]),
            )
        });

        Ok(MaterializationResult::ready(
            MaterializedArtifacts::from_nodes(nodes),
        ))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let port = 4402;
    let server = tokio::spawn(async move { serve(port, ThresholdMaterializer).await });

    sleep(Duration::from_millis(100)).await;

    let waiting_dir = tempdir()?;
    let waiting_outputs = OutputMap::under(waiting_dir.path());
    let waiting_node = NodeRegistration::new("node-1", "127.0.0.1".parse()?);
    let waiting_client = Client::new("http://127.0.0.1:4402");

    let waiting_task = tokio::spawn(async move {
        waiting_client
            .fetch_and_write(&waiting_node, &waiting_outputs)
            .await
    });

    // node-1 is now polling. The materializer will keep returning NotReady
    // until node-2 registers.
    sleep(Duration::from_millis(400)).await;

    let second_dir = tempdir()?;
    let second_outputs = OutputMap::under(second_dir.path());
    let second_node = NodeRegistration::new("node-2", "127.0.0.2".parse()?);

    Client::new("http://127.0.0.1:4402")
        .fetch_and_write(&second_node, &second_outputs)
        .await?;

    waiting_task.await??;

    println!(
        "node-1 config after threshold reached:\n{}",
        std::fs::read_to_string(waiting_dir.path().join("config.yaml"))?
    );
    println!(
        "node-2 config:\n{}",
        std::fs::read_to_string(second_dir.path().join("config.yaml"))?
    );

    server.abort();
    Ok(())
}

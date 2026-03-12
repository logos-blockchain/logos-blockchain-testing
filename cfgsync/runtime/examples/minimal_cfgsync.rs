use cfgsync_adapter::{
    DynCfgsyncError, MaterializationResult, MaterializedArtifacts, RegistrationSnapshot,
    RegistrationSnapshotMaterializer,
};
use cfgsync_artifacts::{ArtifactFile, ArtifactSet};
use cfgsync_core::NodeRegistration;
use cfgsync_runtime::{Client, OutputMap, serve};
use tempfile::tempdir;
use tokio::time::{Duration, sleep};

struct ExampleMaterializer;

impl RegistrationSnapshotMaterializer for ExampleMaterializer {
    fn materialize_snapshot(
        &self,
        registrations: &RegistrationSnapshot,
    ) -> Result<MaterializationResult, DynCfgsyncError> {
        if registrations.is_empty() {
            return Ok(MaterializationResult::NotReady);
        }

        let nodes = registrations.iter().map(|registration| {
            (
                registration.identifier.clone(),
                ArtifactSet::new(vec![ArtifactFile::new(
                    "/config.yaml".to_string(),
                    format!("id: {}\n", registration.identifier),
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
    let port = 4400;
    let server = tokio::spawn(async move { serve(port, ExampleMaterializer).await });

    // Give the server a moment to bind before the client registers.
    sleep(Duration::from_millis(100)).await;

    let tempdir = tempdir()?;
    let outputs = OutputMap::under(tempdir.path().to_path_buf());
    let registration = NodeRegistration::new("node-1".to_string(), "127.0.0.1".parse()?);

    Client::new("http://127.0.0.1:4400")
        .fetch_and_write(&registration, &outputs)
        .await?;

    println!(
        "{}",
        std::fs::read_to_string(tempdir.path().join("config.yaml"))?
    );

    server.abort();
    Ok(())
}

use cfgsync_adapter::{
    DynCfgsyncError, MaterializationResult, MaterializedArtifacts, RegistrationSnapshot,
    RegistrationSnapshotMaterializer,
};
use cfgsync_artifacts::{ArtifactFile, ArtifactSet};
use cfgsync_core::NodeRegistration;
use cfgsync_runtime::{ArtifactOutputMap, fetch_and_write_artifacts, serve_cfgsync};
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
                    "/config.yaml",
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
    let server = tokio::spawn(async move { serve_cfgsync(port, ExampleMaterializer).await });

    // Give the server a moment to bind before the client registers.
    sleep(Duration::from_millis(100)).await;

    let tempdir = tempdir()?;
    let config_path = tempdir.path().join("config.yaml");
    let outputs = ArtifactOutputMap::new().route("/config.yaml", &config_path);
    let registration = NodeRegistration::new("node-1", "127.0.0.1".parse()?);

    fetch_and_write_artifacts(&registration, "http://127.0.0.1:4400", &outputs).await?;

    println!("{}", std::fs::read_to_string(&config_path)?);

    server.abort();
    Ok(())
}

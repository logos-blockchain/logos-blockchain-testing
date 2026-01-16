use std::path::PathBuf;

use anyhow::Context as _;
use cfgsync_tf::server::{CfgSyncConfig, cfgsync_app};
use clap::Parser;
use tokio::net::TcpListener;

#[derive(Parser, Debug)]
#[command(about = "CfgSync")]
struct Args {
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Args::parse();

    let config = CfgSyncConfig::load_from_file(&cli.config)
        .map_err(anyhow::Error::msg)
        .with_context(|| {
            format!(
                "failed to load cfgsync config from {}",
                cli.config.display()
            )
        })?;

    let port = config.port;
    let app = cfgsync_app(config.into());

    println!("Server running on http://0.0.0.0:{port}");
    let listener = TcpListener::bind(&format!("0.0.0.0:{port}"))
        .await
        .with_context(|| format!("failed to bind cfgsync server on 0.0.0.0:{port}"))?;

    axum::serve(listener, app)
        .await
        .context("cfgsync server terminated unexpectedly")?;

    Ok(())
}

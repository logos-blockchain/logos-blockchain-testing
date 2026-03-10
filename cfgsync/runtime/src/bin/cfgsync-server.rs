use std::path::PathBuf;

use cfgsync_runtime::serve_cfgsync_from_config;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "Cfgsync server")]
struct Args {
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    serve_cfgsync_from_config(&args.config).await
}

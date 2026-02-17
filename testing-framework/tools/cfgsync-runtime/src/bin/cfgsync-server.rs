use std::path::PathBuf;

use cfgsync_runtime::run_cfgsync_server;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(about = "CfgSync")]
struct Args {
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    run_cfgsync_server(&args.config).await
}

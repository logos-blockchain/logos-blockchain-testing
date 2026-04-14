use std::path::PathBuf;

use clap::Parser;
use openraft_kv_node::{config::OpenRaftKvNodeConfig, server::run_server};
use tracing_subscriber::EnvFilter;

#[derive(Parser, Clone, Debug)]
#[command(author, version, about)]
struct Opt {
    #[arg(long)]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .with_ansi(false)
        .init();

    let options = Opt::parse();
    let config = OpenRaftKvNodeConfig::load(&options.config)?;
    run_server(config).await
}

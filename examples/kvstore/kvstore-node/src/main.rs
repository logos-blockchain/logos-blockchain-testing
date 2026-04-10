mod config;
mod server;
mod state;
mod sync;

use std::path::PathBuf;

use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{config::KvConfig, state::KvState, sync::SyncService};

#[derive(Parser, Debug)]
#[command(name = "kvstore-node")]
struct Args {
    #[arg(short, long)]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "kvstore_node=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();
    let config = KvConfig::load(&args.config)?;

    let state = KvState::new(config.node_id);
    SyncService::new(config.clone(), state.clone()).start();
    server::start_server(config, state).await
}

mod config;
mod server;
mod state;
mod sync;

use std::path::PathBuf;

use clap::Parser;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

use crate::{config::SchedulerConfig, state::SchedulerState, sync::SyncService};

#[derive(Parser, Debug)]
#[command(name = "scheduler-node")]
struct Args {
    #[arg(short, long)]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "scheduler_node=info,tower_http=debug".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    let args = Args::parse();
    let config = SchedulerConfig::load(&args.config)?;

    let state = SchedulerState::new(config.node_id, config.lease_ttl_ms);
    SyncService::new(config.clone(), state.clone()).start();
    server::start_server(config, state).await
}

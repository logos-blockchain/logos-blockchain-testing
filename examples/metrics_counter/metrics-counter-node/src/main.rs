mod config;
mod server;
mod state;

use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let cli = config::Cli::parse();
    let raw = std::fs::read_to_string(cli.config)?;
    let config: config::CounterConfig = serde_yaml::from_str(&raw)?;
    let state = state::CounterState::new();

    server::start_server(config, state).await
}

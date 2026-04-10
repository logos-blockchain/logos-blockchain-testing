use clap::Parser;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CounterConfig {
    pub node_id: u64,
    pub http_port: u16,
}

#[derive(Debug, Parser)]
pub struct Cli {
    #[arg(long)]
    pub config: std::path::PathBuf,
}

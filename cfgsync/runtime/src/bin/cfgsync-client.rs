use std::{env, process};

use cfgsync_runtime::run_client_from_env;

const CFGSYNC_PORT_ENV: &str = "LOGOS_BLOCKCHAIN_CFGSYNC_PORT";
const DEFAULT_CFGSYNC_PORT: u16 = 4400;

fn cfgsync_port() -> u16 {
    env::var(CFGSYNC_PORT_ENV)
        .ok()
        .and_then(|value| value.parse::<u16>().ok())
        .unwrap_or(DEFAULT_CFGSYNC_PORT)
}

#[tokio::main]
async fn main() {
    if let Err(err) = run_client_from_env(cfgsync_port()).await {
        eprintln!("Error: {err}");
        process::exit(1);
    }
}

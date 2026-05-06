use std::env;

use approval_gate::{register, Config};
use iii_sdk::{register_worker, InitOptions};

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let url = env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.into());
    let iii = register_worker(&url, InitOptions::default());
    let _refs = register(&iii, Config::from_env())?;
    log::info!("approval-gate registered; awaiting events");
    tokio::signal::ctrl_c().await?;
    Ok(())
}

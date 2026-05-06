use std::env;

use sandbox_host::register;

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let url = env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.into());
    let iii = iii_sdk::register_worker(&url, iii_sdk::InitOptions::default());
    let _refs = register(&iii);
    log::info!("sandbox-host registered ({} functions); awaiting calls", _refs.len());
    tokio::signal::ctrl_c().await?;
    Ok(())
}

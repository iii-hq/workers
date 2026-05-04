use anyhow::{anyhow, Result};
use iii_sdk::{register_worker, InitOptions};

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine_url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let iii = register_worker(&engine_url, InitOptions::default());
    let _sub =
        dlp_scrubber::subscribe_dlp_scrubber(&iii).map_err(|e| anyhow!("subscribe failed: {e}"))?;
    log::info!("dlp-scrubber registered (policy::dlp_scrubber on agent::after_tool_call)");

    tokio::signal::ctrl_c().await.ok();
    Ok(())
}

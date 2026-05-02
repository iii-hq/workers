use anyhow::{anyhow, Result};
use iii_sdk::{register_worker, InitOptions};

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";
const DEFAULT_DENYLIST: &str = "bash:rm -rf,sudo,curl-pipe-bash";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine_url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let denied: Vec<String> = std::env::var("POLICY_DENIED_TOOLS")
        .unwrap_or_else(|_| DEFAULT_DENYLIST.to_string())
        .split(',')
        .map(str::to_string)
        .collect();

    let iii = register_worker(&engine_url, InitOptions::default());
    let _sub = policy_denylist::subscribe_denylist(&iii, denied.clone())
        .map_err(|e| anyhow!("subscribe failed: {e}"))?;
    log::info!(
        "policy-denylist registered (policy::denylist on agent::before_tool_call); denied=[{}]",
        denied.join(", ")
    );

    tokio::signal::ctrl_c().await.ok();
    Ok(())
}

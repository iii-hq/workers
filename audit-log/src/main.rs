use std::path::PathBuf;

use anyhow::{anyhow, Result};
use iii_sdk::{register_worker, InitOptions};

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

fn default_log_path() -> PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    PathBuf::from(home).join(".harness/audit.jsonl")
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine_url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let log_path = std::env::var("AUDIT_LOG_PATH")
        .map(PathBuf::from)
        .unwrap_or_else(|_| default_log_path());

    let iii = register_worker(&engine_url, InitOptions::default());
    let _sub = audit_log::subscribe_audit_log(&iii, log_path.clone())
        .map_err(|e| anyhow!("subscribe failed: {e}"))?;
    log::info!(
        "audit-log registered (policy::audit_log on agent::after_tool_call); writing to {}",
        log_path.display()
    );

    tokio::signal::ctrl_c().await.ok();
    Ok(())
}

use std::sync::Arc;

use anyhow::{Context, Result};
use iii_sdk::{register_worker, InitOptions};

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine_url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let iii = Arc::new(register_worker(&engine_url, InitOptions::default()));

    let cfg = auth_rbac::AuthRbacConfig::from_env()
        .context("AUTH_HMAC_SECRET must be set for auth-rbac")?;
    let _refs = auth_rbac::register_with_iii(&iii, cfg)
        .await
        .context("auth-rbac register failed")?;
    log::info!("auth-rbac registered (auth::rbac::*)");

    tokio::signal::ctrl_c().await.ok();
    Ok(())
}

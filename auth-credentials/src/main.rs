use std::sync::Arc;

use anyhow::{Context, Result};
use iii_sdk::{register_worker, InitOptions};

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine_url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let iii = Arc::new(register_worker(&engine_url, InitOptions::default()));

    let store: Arc<dyn auth_credentials::CredentialStore> =
        match std::env::var("AUTH_CREDENTIALS_STORE").as_deref() {
            Ok("memory") => {
                log::info!(
                    "auth-credentials: using in-memory store (AUTH_CREDENTIALS_STORE=memory)"
                );
                Arc::new(auth_credentials::InMemoryStore::new())
            }
            Ok("iii_state") | Err(_) => {
                log::info!("auth-credentials: using iii-state store");
                let iii_for_store: Arc<dyn auth_credentials::io::IIITrigger> = iii.clone();
                Arc::new(auth_credentials::store_iii_state::IiiStateCredentialStore::new(
                    iii_for_store,
                ))
            }
            Ok(other) => {
                log::warn!(
                    "auth-credentials: unknown AUTH_CREDENTIALS_STORE={other:?}; falling back to iii-state. \
                     Valid values: memory, iii_state."
                );
                let iii_for_store: Arc<dyn auth_credentials::io::IIITrigger> = iii.clone();
                Arc::new(auth_credentials::store_iii_state::IiiStateCredentialStore::new(
                    iii_for_store,
                ))
            }
        };
    let _refs = auth_credentials::register_with_iii(&iii, store)
        .await
        .context("auth-credentials register failed")?;
    log::info!("auth-credentials registered (auth::*)");

    tokio::signal::ctrl_c().await.ok();
    Ok(())
}

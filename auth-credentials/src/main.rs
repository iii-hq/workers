use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine_url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let iii = Arc::new(register_worker(&engine_url, InitOptions::default()));

    let store: Arc<dyn auth_credentials::CredentialStore> = match std::env::var(
        "AUTH_CREDENTIALS_STORE",
    )
    .as_deref()
    {
        Ok("memory") => {
            log::info!("auth-credentials: using in-memory store (AUTH_CREDENTIALS_STORE=memory)");
            Arc::new(auth_credentials::InMemoryStore::new())
        }
        Ok("iii_state") | Err(_) => {
            log::info!("auth-credentials: using iii-state store");
            let iii_for_store: Arc<dyn auth_credentials::io::IIITrigger> = iii.clone();
            Arc::new(auth_credentials::store_iii_state::IiiStateCredentialStore::new(iii_for_store))
        }
        Ok(other) => {
            log::warn!(
                    "auth-credentials: unknown AUTH_CREDENTIALS_STORE={other:?}; falling back to iii-state. \
                     Valid values: memory, iii_state."
                );
            let iii_for_store: Arc<dyn auth_credentials::io::IIITrigger> = iii.clone();
            Arc::new(auth_credentials::store_iii_state::IiiStateCredentialStore::new(iii_for_store))
        }
    };
    let _refs = auth_credentials::register_with_iii(&iii, store)
        .await
        .context("auth-credentials register failed")?;
    log::info!("auth-credentials registered (auth::*)");

    spawn_skill_register(iii.clone());

    wait_for_shutdown().await?;

    unregister_skill(&iii).await;
    Ok(())
}

fn spawn_skill_register(iii: Arc<iii_sdk::III>) {
    tokio::spawn(async move {
        let mut backoff = Duration::from_secs(5);
        let started = Instant::now();
        loop {
            let res = iii
                .trigger(TriggerRequest {
                    function_id: "skills::register".into(),
                    payload: json!({
                        "id": auth_credentials::SKILL_ID,
                        "skill": auth_credentials::SKILL_MD,
                    }),
                    action: None,
                    timeout_ms: Some(5_000),
                })
                .await;
            match res {
                Ok(_) => {
                    log::info!("registered skill: {}", auth_credentials::SKILL_ID);
                    return;
                }
                Err(e) => {
                    if started.elapsed() > Duration::from_secs(180) {
                        log::warn!(
                            "skills handshake gave up for {}; install/start the skills worker and restart (last error: {e})",
                            auth_credentials::SKILL_ID
                        );
                        return;
                    }
                    log::debug!(
                        "skills::register failed for {}: {e}; retrying in {backoff:?}",
                        auth_credentials::SKILL_ID
                    );
                }
            }
            tokio::time::sleep(backoff).await;
            backoff = (backoff * 2).min(Duration::from_secs(60));
        }
    });
}

async fn wait_for_shutdown() -> Result<()> {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let mut sigterm =
            signal(SignalKind::terminate()).context("failed to install SIGTERM handler")?;
        tokio::select! {
            r = tokio::signal::ctrl_c() => r.context("failed to await SIGINT")?,
            _ = sigterm.recv() => {}
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c()
            .await
            .context("failed to await SIGINT")
    }
}

// Best-effort: a missed unregister is self-healing on next boot's re-register.
async fn unregister_skill(iii: &Arc<iii_sdk::III>) {
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "skills::unregister".into(),
            payload: json!({ "id": auth_credentials::SKILL_ID }),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await;
}

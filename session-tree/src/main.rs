use std::sync::Arc;

use anyhow::Result;
use iii_sdk::{register_worker, InitOptions};

const DEFAULT_ENGINE_URL: &str = "ws://127.0.0.1:49134";

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let engine_url = std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_ENGINE_URL.to_string());
    let iii = Arc::new(register_worker(&engine_url, InitOptions::default()));

    let store: Arc<dyn session_tree::SessionStore> =
        match std::env::var("SESSION_TREE_STORE").as_deref() {
            Ok("memory") => {
                log::info!("session-tree: using in-memory store (SESSION_TREE_STORE=memory)");
                Arc::new(session_tree::InMemoryStore::default())
            }
            Ok("iii_state") | Err(_) => {
                log::info!("session-tree: using iii-state store");
                let iii_for_store: Arc<dyn session_tree::io::IIITrigger> = iii.clone();
                Arc::new(session_tree::store_iii_state::IiiStateSessionStore::new(
                    iii_for_store,
                ))
            }
            Ok(other) => {
                log::warn!(
                "session-tree: unknown SESSION_TREE_STORE={other:?}; falling back to iii-state. \
                     Valid values: memory, iii_state."
            );
                let iii_for_store: Arc<dyn session_tree::io::IIITrigger> = iii.clone();
                Arc::new(session_tree::store_iii_state::IiiStateSessionStore::new(
                    iii_for_store,
                ))
            }
        };
    let _refs = session_tree::register_with_iii(&iii, store);
    log::info!("session-tree registered (session::*)");

    tokio::signal::ctrl_c().await.ok();
    Ok(())
}

//! Lazily resolve a single shared `iii_sdk::III` handle so `@engine`
//! scenarios can run in-process against a live engine. When the engine
//! isn't reachable (typical contributor laptop), every consumer gets
//! `None` and `@engine` steps short-circuit without failing the run.
//!
//! Override the engine URL via `III_ENGINE_WS_URL`. `@pure` scenarios
//! never call this and run with no engine at all.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::{register_worker, InitOptions, III};
use tokio::sync::OnceCell;

static ENGINE: OnceCell<Option<Arc<III>>> = OnceCell::const_new();

pub async fn get_or_init() -> Option<Arc<III>> {
    ENGINE
        .get_or_init(|| async {
            let url = std::env::var("III_ENGINE_WS_URL")
                .unwrap_or_else(|_| "ws://127.0.0.1:49134".into());
            // Soft-probe: try to register; if the engine is unreachable
            // the SDK's reconnect machinery will keep trying forever, so
            // we cap the attempt with a timeout and treat the failure
            // as a skip.
            let connect = tokio::time::timeout(
                Duration::from_millis(500),
                tokio::task::spawn_blocking(move || {
                    let iii = register_worker(&url, InitOptions::default());
                    Arc::new(iii)
                }),
            )
            .await;
            match connect {
                Ok(Ok(arc)) => Some(arc),
                _ => None,
            }
        })
        .await
        .clone()
}

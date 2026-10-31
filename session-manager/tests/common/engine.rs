//! Connect-or-skip wrapper around the iii SDK.
//!
//! One engine connection per test binary process via OnceCell — the
//! cost of each WebSocket handshake dwarfs cucumber's per-scenario
//! overhead. On success the production surface (trigger types +
//! functions) is registered in-process so @engine scenarios drive the
//! exact code the binary would run. An engine that already runs
//! session-manager is refused like an unreachable one (see
//! `engine_guard`): the scenarios would write into its real store.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
use tokio::sync::OnceCell;

use super::engine_guard::session_manager_on_engine;

/// iii's default port: `cargo test` finds a developer's bare `iii`
/// without configuration. That is also where an everyday engine runs
/// session-manager, which `engine_guard` refuses.
const DEFAULT_WS_URL: &str = "ws://127.0.0.1:49134";

static ENGINE: OnceCell<Option<Arc<IIIClient>>> = OnceCell::const_new();

/// Payloads received by engine-side recorder functions, keyed by the
/// recorder function id.
static RECEIVED: Mutex<Option<HashMap<String, Vec<Value>>>> = Mutex::new(None);

pub fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| DEFAULT_WS_URL.to_string())
}

/// How to run the @engine scenarios when they skip.
const RUN_THEM: &str = "To run them, set III_ENGINE_WS_URL to a dedicated engine with no \
                        session-manager (e.g. a bare `iii --config <file>` on a free port).";

async fn try_connect_raw() -> Option<Arc<IIIClient>> {
    let url = ws_url();
    let iii = Arc::new(register_worker(&url, InitOptions::default()));

    let mut workers = None;
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Ok(listing) = list(&iii, "engine::workers::list", json!({})).await {
            workers = Some(listing);
            break;
        }
    }
    let Some(workers) = workers else {
        return skip(
            iii,
            format!(
                "iii engine not reachable at {url}; \
                 set III_ENGINE_WS_URL or start `iii` to enable engine-bound BDD scenarios"
            ),
        )
        .await;
    };

    // Nothing is registered yet: decide before `register_all` puts a
    // second session::* surface next to a real one. Fail closed: an
    // engine whose functions cannot be listed may be a live one.
    let functions = match list(
        &iii,
        "engine::functions::list",
        json!({ "include_internal": true }),
    )
    .await
    {
        Ok(functions) => functions,
        Err(e) => {
            return skip(
                iii,
                format!(
                    "could not list the functions of the iii engine at {url} ({e}), so it may \
                     run session-manager: every @engine scenario is skipped. {RUN_THEM}"
                ),
            )
            .await
        }
    };
    if let Some(evidence) = session_manager_on_engine(&workers, &functions) {
        return skip(
            iii,
            format!(
                "iii engine at {url} already runs session-manager ({evidence}). The \
                 engine-bound BDD scenarios register their own session::* surface, so their \
                 calls would reach that worker and write into its real store: every @engine \
                 scenario is skipped. {RUN_THEM}"
            ),
        )
        .await;
    }
    Some(iii)
}

/// Report why the @engine scenarios soft-skip and drop the connection.
async fn skip(iii: Arc<IIIClient>, why: String) -> Option<Arc<IIIClient>> {
    eprintln!("[skip] {why}");
    iii.shutdown_async().await;
    None
}

/// One `engine::*::list` call; the reachability probe retries it.
async fn list(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(800),
    })
    .await
}

/// Get-or-init the shared engine handle, registering the production
/// session-manager surface in-process on first success.
pub async fn get_or_init() -> Option<Arc<IIIClient>> {
    ENGINE
        .get_or_init(|| async {
            let iii = try_connect_raw().await?;
            super::workers::register_all(&iii).await;
            Some(iii)
        })
        .await
        .clone()
}

/// Register an engine-side recorder function that stashes every payload
/// it receives. Returns the generated function id.
pub fn register_recorder_function(iii: &Arc<IIIClient>) -> String {
    let function_id = format!("test::session_recv::{}", uuid::Uuid::new_v4().simple());
    {
        let mut map = RECEIVED.lock().unwrap_or_else(|poison| poison.into_inner());
        map.get_or_insert_with(HashMap::new)
            .insert(function_id.clone(), Vec::new());
    }
    let fid = function_id.clone();
    iii.register_function(
        function_id.clone(),
        iii_sdk::RegisterFunction::new_async(move |payload: Value| {
            let fid = fid.clone();
            async move {
                let mut map = RECEIVED.lock().unwrap_or_else(|poison| poison.into_inner());
                map.get_or_insert_with(HashMap::new)
                    .entry(fid)
                    .or_default()
                    .push(payload);
                Ok::<_, Error>(json!({ "ok": true }))
            }
        })
        .description("BDD recorder: stashes received session events."),
    );
    function_id
}

/// Snapshot of payloads received by one recorder function.
pub fn received_by(function_id: &str) -> Vec<Value> {
    let map = RECEIVED.lock().unwrap_or_else(|poison| poison.into_inner());
    map.as_ref()
        .and_then(|m| m.get(function_id))
        .cloned()
        .unwrap_or_default()
}

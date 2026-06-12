//! Connect-or-skip wrapper around the iii SDK.
//!
//! One engine connection per test binary process via OnceCell — the
//! cost of each WebSocket handshake dwarfs cucumber's per-scenario
//! overhead. On success the production surface (trigger types +
//! functions) is registered in-process so @engine scenarios drive the
//! exact code the binary would run.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::{register_worker, IIIError, InitOptions, TriggerRequest, III};
use serde_json::{json, Value};
use tokio::sync::OnceCell;

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:49134";

static ENGINE: OnceCell<Option<Arc<III>>> = OnceCell::const_new();

/// Payloads received by engine-side recorder functions, keyed by the
/// recorder function id.
static RECEIVED: Mutex<Option<HashMap<String, Vec<Value>>>> = Mutex::new(None);

pub fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| DEFAULT_WS_URL.to_string())
}

async fn try_connect_raw() -> Option<Arc<III>> {
    let url = ws_url();
    let iii = Arc::new(register_worker(&url, InitOptions::default()));

    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let probe = iii
            .trigger(TriggerRequest {
                function_id: "engine::workers::list".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(800),
            })
            .await;
        match probe {
            Ok(_) => return Some(iii),
            Err(IIIError::NotConnected) => continue,
            Err(_) => continue,
        }
    }

    eprintln!(
        "[skip] iii engine not reachable at {url}; \
         set III_ENGINE_WS_URL or start `iii` to enable engine-bound BDD scenarios"
    );
    iii.shutdown_async().await;
    None
}

/// Get-or-init the shared engine handle, registering the production
/// session-manager surface in-process on first success.
pub async fn get_or_init() -> Option<Arc<III>> {
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
pub fn register_recorder_function(iii: &Arc<III>) -> String {
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
                Ok::<_, IIIError>(json!({ "ok": true }))
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

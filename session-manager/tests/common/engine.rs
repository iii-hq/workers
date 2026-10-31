//! Connect-or-skip wrapper around the iii SDK.
//!
//! One engine connection per test binary process via OnceCell — the
//! cost of each WebSocket handshake dwarfs cucumber's per-scenario
//! overhead. On success the production surface (trigger types +
//! functions) is registered in-process so @engine scenarios drive the
//! exact code the binary would run. An engine that already runs
//! session-manager, or one that cannot show it does not, is refused like
//! an unreachable one (see `engine_guard`): the scenarios would write
//! into its real store.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::runtime::{IIIConnectionState, WorkerMetadata};
use iii_sdk::{register_worker, IIIClient, InitOptions, WorkerIdentityMode};
use serde_json::{json, Value};
use tokio::sync::OnceCell;

use super::engine_guard::{harness_worker_name, skip_after_registering, skip_before_registering};

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

async fn try_connect_raw() -> Option<Arc<IIIClient>> {
    let url = ws_url();
    // An explicit identity: in the default (managed) mode the SDK takes
    // III_WORKER_NAME / III_NAMESPACE from the environment, and a run
    // under III_WORKER_NAME=session-manager would look like the real one.
    let iii = Arc::new(register_worker(
        &url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: harness_worker_name(std::process::id()),
                ..WorkerMetadata::default()
            }),
            identity: WorkerIdentityMode::Explicit,
            ..InitOptions::default()
        },
    ));

    let mut workers = Err(Error::NotConnected);
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        workers = list(&iii, "engine::workers::list", json!({}), 800).await;
        if workers.is_ok() {
            break;
        }
    }
    // Never connected: no engine there. Connected but unable to list its
    // workers: an engine that cannot be inspected (the guard skips it).
    if workers.is_err() && iii.get_connection_state() != IIIConnectionState::Connected {
        return skip(
            iii,
            format!(
                "iii engine not reachable at {url}; \
                 set III_ENGINE_WS_URL or start `iii` to enable engine-bound BDD scenarios"
            ),
        )
        .await;
    }

    // Nothing is registered yet: decide before `register_all` puts a
    // second session::* surface next to a real one.
    let workers = workers.map_err(|e| e.to_string());
    let functions = list_functions(&iii).await;
    if let Some(why) = skip_before_registering(&url, &workers, &functions) {
        return skip(iii, why).await;
    }
    Some(iii)
}

/// Report why the @engine scenarios soft-skip and drop the connection.
async fn skip(iii: Arc<IIIClient>, why: String) -> Option<Arc<IIIClient>> {
    eprintln!("[skip] {why}");
    iii.shutdown_async().await;
    None
}

async fn list(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
    timeout_ms: u64,
) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(timeout_ms),
    })
    .await
}

/// Every function on the engine, internal ones included (session-manager
/// registers its mutating functions as internal). A few patient tries, so
/// a busy engine does not read as one that cannot be inspected.
async fn list_functions(iii: &IIIClient) -> Result<Value, String> {
    let payload = json!({ "include_internal": true });
    let mut outcome = Err(String::new());
    for attempt in 0..3 {
        if attempt > 0 {
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        outcome = list(iii, "engine::functions::list", payload.clone(), 5_000)
            .await
            .map_err(|e| e.to_string());
        if outcome.is_ok() {
            break;
        }
    }
    outcome
}

/// Get-or-init the shared engine handle, registering the production
/// session-manager surface in-process on first success.
pub async fn get_or_init() -> Option<Arc<IIIClient>> {
    ENGINE
        .get_or_init(|| async {
            let iii = try_connect_raw().await?;
            super::workers::register_all(&iii).await;
            // Again, now that the surface is in: a session-manager that
            // (re)connected since the first check owns the ids it got to
            // first, and the engine refuses ours without failing the run.
            let harness = harness_worker_name(std::process::id());
            let functions = list_functions(&iii).await;
            if let Some(why) = skip_after_registering(&ws_url(), &harness, &functions) {
                return skip(iii, why).await;
            }
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

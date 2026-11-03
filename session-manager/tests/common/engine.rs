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
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions, WorkerIdentityMode};
use serde_json::{json, Value};
use tokio::sync::OnceCell;

use super::engine_guard::{
    harness_worker_name, skip_after_probe, skip_after_registering, skip_before_registering,
};
use super::workers::{wait_until_routable, LAST_REGISTERED};

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
                name: harness(),
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
    let workers = workers.map_err(|e| e.to_string());
    if let Some(why) = skip_after_probe(&url, iii.get_connection_state(), &workers) {
        return skip(&iii, why);
    }

    // Nothing is registered yet: decide before `register_all` puts a
    // second session::* surface next to a real one.
    let functions = list_functions(&iii).await;
    if let Some(why) = skip_before_registering(&url, &workers, &functions) {
        return skip(&iii, why);
    }
    Some(iii)
}

/// Report why the @engine scenarios soft-skip and close the connection
/// (joined, so the harness's registrations are gone when this returns).
fn skip(iii: &IIIClient, why: String) -> Option<Arc<IIIClient>> {
    eprintln!("[skip] {why}");
    iii.shutdown();
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

/// The engine listing once `function_id` appears in it (registered by
/// anyone), or why it did not within 10 s.
async fn wait_until_listed(iii: &IIIClient, function_id: &str) -> Result<Value, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let outcome = list_functions(iii).await;
        let listed = outcome.as_ref().is_ok_and(|listing| {
            listing["functions"]
                .as_array()
                .is_some_and(|rows| rows.iter().any(|f| f["function_id"] == function_id))
        });
        if listed {
            return outcome;
        }
        if std::time::Instant::now() >= deadline {
            return outcome.and_then(|_| Err(format!("{function_id} was not listed within 10 s")));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Get-or-init the shared engine handle, registering the production
/// session-manager surface in-process on first success.
pub async fn get_or_init() -> Option<Arc<IIIClient>> {
    ENGINE
        .get_or_init(|| async {
            let iii = try_connect_raw().await?;
            super::workers::register_all(&iii).await;
            // Again, now that the surface is in, and before any session::*
            // call: a session-manager that (re)connected since the first
            // check owns the ids it got to first, and the engine refuses
            // ours without failing the run.
            let functions = wait_until_listed(&iii, LAST_REGISTERED).await;
            if let Some(why) = skip_after_registering(&ws_url(), &harness(), &functions) {
                return skip(&iii, why);
            }
            if let Err(why) = wait_until_routable(&iii, LAST_REGISTERED).await {
                return skip(&iii, why);
            }
            Some(iii)
        })
        .await
        .clone()
}

/// Set once a check before a scenario found `session::*` functions that
/// are not the harness's: the engine is withdrawn for the rest of the run.
static WITHDRAWN: OnceLock<()> = OnceLock::new();

/// The engine handle for one scenario. For an @engine scenario the
/// ownership check runs again first: an engine restart makes the SDK
/// replay the harness's registrations, a fresh race a real session-manager
/// can win (the engine then refuses ours, and the SDK only logs it). On a
/// failed check every later scenario gets `None` (soft skip). A takeover
/// in the middle of a scenario still reaches the rest of that scenario.
pub async fn for_scenario(engine_scenario: bool) -> Option<Arc<IIIClient>> {
    let iii = get_or_init().await?;
    if WITHDRAWN.get().is_some() {
        return None;
    }
    if engine_scenario {
        let functions = list_functions(&iii).await;
        if let Some(why) = skip_after_registering(&ws_url(), &harness(), &functions) {
            if WITHDRAWN.set(()).is_ok() {
                skip(&iii, why);
            }
            return None;
        }
    }
    Some(iii)
}

fn harness() -> String {
    harness_worker_name(std::process::id())
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

//! Unit tests of the @engine scenarios' live-engine guard
//! (`common/engine_guard.rs`). The cucumber binary has no test harness and
//! never runs `#[test]`s, so this target compiles the guard on its own.

#[path = "common/engine_guard.rs"]
mod engine_guard;

use engine_guard::session_manager_on_engine;
use serde_json::{json, Value};

/// `engine::workers::list` of a bare engine (`iii --config` with only
/// `iii-worker-manager`), trimmed to the fields the guard reads.
fn bare_workers() -> Value {
    json!({ "workers": [
        { "id": "configuration", "name": "configuration", "runtime": "engine" },
        { "id": "iii-engine-functions", "name": "iii-engine-functions", "runtime": "engine" },
        { "id": "iii-observability", "name": "iii-observability", "runtime": "engine" },
        { "id": "iii-worker-manager", "name": "iii-worker-manager", "runtime": "engine" },
    ]})
}

fn bare_functions() -> Value {
    json!({ "functions": [
        { "function_id": "configuration::get", "worker_name": "configuration" },
        { "function_id": "engine::traces::list", "worker_name": "iii-observability" },
    ]})
}

fn with_worker(mut workers: Value, name: &str) -> Value {
    workers["workers"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "id": "1f4c43fc", "name": name, "runtime": "rust" }));
    workers
}

fn with_functions(mut functions: Value, worker: &str, ids: &[&str]) -> Value {
    let list = functions["functions"].as_array_mut().unwrap();
    for id in ids {
        list.push(json!({ "function_id": id, "worker_name": worker }));
    }
    functions
}

#[test]
fn a_bare_engine_runs_the_scenarios() {
    assert_eq!(
        session_manager_on_engine(&bare_workers(), &bare_functions()),
        None
    );
}

#[test]
fn an_engine_running_session_manager_skips_them() {
    let workers = with_worker(bare_workers(), "session-manager");
    let functions = with_functions(
        bare_functions(),
        "session-manager",
        &[
            "session::list",
            "session::create",
            "session::store::get-entry",
        ],
    );
    let evidence = session_manager_on_engine(&workers, &functions).expect("must skip");
    assert_eq!(
        evidence,
        "a worker named `session-manager` is connected; worker `session-manager` \
         already serves `session::create` and 2 more `session::*` function(s)"
    );
}

// A session-manager under another name (III_WORKER_NAME, a compose
// container named otherwise) still owns the function ids.
#[test]
fn session_functions_under_another_worker_name_skip_them() {
    let workers = with_worker(bare_workers(), "sessions-prod");
    let functions = with_functions(bare_functions(), "sessions-prod", &["session::create"]);
    let evidence = session_manager_on_engine(&workers, &functions).expect("must skip");
    assert_eq!(
        evidence,
        "worker `sessions-prod` already serves `session::create`"
    );
}

// An engine that ignores `include_internal` still lists the public
// reads (`session::list`, `session::get`).
#[test]
fn public_session_functions_alone_skip_them() {
    let functions = with_functions(bare_functions(), "session-manager", &["session::list"]);
    assert!(session_manager_on_engine(&bare_workers(), &functions).is_some());
}

// A session-manager that has connected but not registered yet (it is
// booting, or restarting under the engine).
#[test]
fn a_connected_session_manager_without_functions_skips_them() {
    let workers = with_worker(bare_workers(), "session-manager");
    assert_eq!(
        session_manager_on_engine(&workers, &bare_functions()).as_deref(),
        Some("a worker named `session-manager` is connected")
    );
}

#[test]
fn look_alike_names_do_not_skip_them() {
    let workers = with_worker(bare_workers(), "session-manager-ui");
    let functions = with_functions(
        bare_functions(),
        "agent",
        &[
            "sessions::list",
            "agent::session::get",
            "session-manager::ping",
        ],
    );
    assert_eq!(session_manager_on_engine(&workers, &functions), None);
}

#[test]
fn empty_or_unexpected_listings_do_not_panic() {
    assert_eq!(session_manager_on_engine(&json!({}), &json!({})), None);
    assert_eq!(session_manager_on_engine(&Value::Null, &json!([])), None);
    let odd = json!({ "functions": [{ "function_id": 7 }, { "worker_name": "x" }] });
    assert_eq!(
        session_manager_on_engine(&json!({ "workers": "?" }), &odd),
        None
    );
}

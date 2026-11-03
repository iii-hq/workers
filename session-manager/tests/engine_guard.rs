//! Unit tests of the @engine scenarios' live-engine guard
//! (`common/engine_guard.rs`). The cucumber binary has no test harness and
//! never runs `#[test]`s, so this target compiles the guard on its own.

#[path = "common/engine_guard.rs"]
mod engine_guard;

use engine_guard::{
    harness_worker_name, skip_after_probe, skip_after_registering, skip_before_registering,
};
use iii_sdk::runtime::IIIConnectionState;
use serde_json::{json, Value};

const URL: &str = "ws://127.0.0.1:49611";
const HARNESS: &str = "session-manager-bdd:4242";

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

fn before(workers: Value, functions: Value) -> Option<String> {
    skip_before_registering(URL, &Ok(workers), &Ok(functions))
}

fn after(functions: Result<Value, String>) -> Option<String> {
    skip_after_registering(URL, HARNESS, &functions)
}

#[test]
fn a_bare_engine_runs_the_scenarios() {
    assert_eq!(before(bare_workers(), bare_functions()), None);
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
    let why = before(workers, functions).expect("must skip");
    assert!(why.starts_with(
        "iii engine at ws://127.0.0.1:49611 already runs session-manager (a worker named \
         `session-manager` is connected; worker `session-manager` already serves \
         `session::create` and 2 more `session::*` function(s))."
    ));
    assert!(why
        .ends_with("with no session-manager (e.g. a bare `iii --config <file>` on a free port)."));
}

// A session-manager under another name (III_WORKER_NAME, a compose
// container named otherwise) still owns the function ids.
#[test]
fn session_functions_under_another_worker_name_skip_them() {
    let workers = with_worker(bare_workers(), "sessions-prod");
    let functions = with_functions(bare_functions(), "sessions-prod", &["session::create"]);
    let why = before(workers, functions).expect("must skip");
    assert!(why.contains("(worker `sessions-prod` already serves `session::create`)"));
}

// An engine that ignores `include_internal` still lists the public reads.
#[test]
fn public_session_functions_alone_skip_them() {
    let functions = with_functions(bare_functions(), "session-manager", &["session::list"]);
    assert!(before(bare_workers(), functions).is_some());
}

// A session-manager that has connected but not registered yet.
#[test]
fn a_connected_session_manager_without_functions_skips_them() {
    let workers = with_worker(bare_workers(), "session-manager");
    let why = before(workers, bare_functions()).expect("must skip");
    assert!(why.contains("(a worker named `session-manager` is connected)"));
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
    assert_eq!(before(workers, functions), None);
}

// Fail closed: a listing in another shape (an RBAC proxy filtering the
// answer, an engine version that renamed the key) proves nothing.
#[test]
fn a_listing_without_its_array_skips_them() {
    let why = before(json!({}), bare_functions()).expect("must skip");
    assert!(why.contains("engine::workers::list answered without a `workers` list"));
    let why = before(bare_workers(), json!({ "functions": "?" })).expect("must skip");
    assert!(why.contains("engine::functions::list answered without a `functions` list"));
    let why = before(bare_workers(), json!([])).expect("must skip");
    assert!(why.contains("engine::functions::list answered without a `functions` list"));
    let why = before(Value::Null, bare_functions()).expect("must skip");
    assert!(why.contains("engine::workers::list answered without a `workers` list"));
}

#[test]
fn a_listing_that_failed_skips_them() {
    let why = skip_before_registering(URL, &Err("timeout".into()), &Ok(bare_functions()))
        .expect("must skip");
    assert!(why.starts_with(
        "cannot tell whether the iii engine at ws://127.0.0.1:49611 runs session-manager: \
         engine::workers::list failed (timeout). Every @engine scenario is skipped."
    ));
    let why = skip_before_registering(URL, &Ok(bare_workers()), &Err("forbidden".into()))
        .expect("must skip");
    assert!(why.contains("engine::functions::list failed (forbidden)"));
}

// Fail closed on rows too: a row the guard cannot read could be the
// session-manager it is looking for.
#[test]
fn a_row_without_its_field_skips_them() {
    let mut functions = bare_functions();
    functions["functions"]
        .as_array_mut()
        .unwrap()
        .push(json!({ "function_id": 7, "worker_name": "x" }));
    let why = before(bare_workers(), functions.clone()).expect("must skip");
    assert!(why.contains(
        "engine::functions::list listed an entry without a string `function_id`: \
         {\"function_id\":7,\"worker_name\":\"x\"}"
    ));
    assert!(after(Ok(functions)).is_some());

    let mut workers = bare_workers();
    workers["workers"].as_array_mut().unwrap().push(json!(null));
    let why = before(workers, bare_functions()).expect("must skip");
    assert!(why.contains("engine::workers::list listed an entry without a string `name`: null"));
}

#[test]
fn the_probe_tells_no_engine_from_an_engine_that_cannot_be_inspected() {
    let listed = Ok(bare_workers());
    assert_eq!(
        skip_after_probe(URL, IIIConnectionState::Connected, &listed),
        None
    );

    let failed: Result<Value, String> = Err("invocation timed out".into());
    for state in [
        IIIConnectionState::Disconnected,
        IIIConnectionState::Connecting,
        IIIConnectionState::Reconnecting,
    ] {
        let why = skip_after_probe(URL, state, &failed).expect("must skip");
        assert!(why.starts_with("iii engine not reachable at ws://127.0.0.1:49611;"));
    }
    let why = skip_after_probe(URL, IIIConnectionState::Connected, &failed).expect("must skip");
    assert!(why.contains(
        "cannot tell whether the iii engine at ws://127.0.0.1:49611 runs session-manager: \
         engine::workers::list failed (invocation timed out)"
    ));
    // A fatal rejection (e.g. a worker-name conflict): an engine answered.
    let why = skip_after_probe(URL, IIIConnectionState::Failed, &failed).expect("must skip");
    assert!(why.contains("the engine refused the harness's connection (invocation timed out)"));
}

#[test]
fn after_registering_the_harness_own_surface_runs_them() {
    let functions = with_functions(
        bare_functions(),
        HARNESS,
        &["session::create", "session::store::get-entry"],
    );
    assert_eq!(after(Ok(functions)), None);
}

// A session-manager that came back (a compose restart) between the first
// check and the registrations got some ids first.
#[test]
fn after_registering_a_foreign_session_function_skips_them() {
    let functions = with_functions(bare_functions(), HARNESS, &["session::list"]);
    let functions = with_functions(functions, "session-manager", &["session::create"]);
    let why = after(Ok(functions)).expect("must skip");
    assert!(why.starts_with(
        "another worker serves session::* functions on the iii engine at ws://127.0.0.1:49611 \
         next to the BDD harness (worker `session-manager` already serves `session::create`)"
    ));
    assert!(why.contains("Every @engine scenario from here on is skipped."));
}

#[test]
fn after_registering_a_session_function_without_a_worker_skips_them() {
    let functions = json!({ "functions": [{ "function_id": "session::create" }] });
    let why = after(Ok(functions)).expect("must skip");
    assert!(why.contains("worker `<unnamed>` already serves `session::create`"));
}

#[test]
fn after_registering_an_unreadable_listing_skips_them() {
    assert!(after(Err("timeout".into()))
        .expect("must skip")
        .contains("engine::functions::list failed (timeout)"));
    assert!(after(Ok(json!({ "functions": null })))
        .expect("must skip")
        .contains("answered without a `functions` list"));
}

#[test]
fn the_harness_never_looks_like_session_manager() {
    let name = harness_worker_name(4242);
    assert_eq!(name, HARNESS);
    assert_ne!(name, "session-manager");
    assert_ne!(harness_worker_name(1), harness_worker_name(2));
}

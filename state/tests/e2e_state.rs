//! End-to-end coverage against a running engine: `state::*` functions called
//! and `state::on-changed` observed through the REAL bus path
//! (`IIIClient::call` / `IIIClient::observe`), the same path every other
//! worker uses.
//!
//! Connect-or-skip like every e2e suite (see `common::engine`). Every test
//! uses a DEDICATED connection (`engine::connect_fresh`), never the shared
//! one: `boot::start` registers fixed function ids (`state::set`,
//! `state::get`, ...) on every call, and the SDK panics if the same id is
//! registered twice on one client.

mod common;

use std::time::Duration;

use common::engine;
use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, Deliver, IIIClient, ObserverHandle, RegisterFunction, Source};
use iii_state::config::{EARLIER_EVENTS_KEY, StateConfig};
use iii_state::configuration;
use iii_state::functions::ConfigCell;
use iii_state::on_changed::ON_CHANGED_FN;
use serde_json::{Value, json};
use serial_test::serial;
use uuid::Uuid;

/// A fresh, uuid-suffixed scope/key namespace per test so reruns (and, within
/// a test, sibling scopes) never collide.
fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4())
}

/// Boot config for e2e: the shipped default store_method is file_based, so an
/// unpinned boot would write `./data/state` in the test cwd and leak state
/// across tests and runs. Every boot here pins in_memory.
fn in_memory_config() -> StateConfig {
    // Extend the default (keeping events_enabled etc.) rather than building
    // from bare JSON, which would drop the default knobs to None.
    StateConfig {
        adapter: Some(
            serde_json::from_value(json!({"name": "kv", "config": {"store_method": "in_memory"}}))
                .expect("valid in-memory adapter config"),
        ),
        ..StateConfig::default()
    }
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, Error> {
    iii.call(CallRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(5_000),
    })
    .await
}

/// Register `target` as a sink that forwards every event it receives.
fn event_sink(iii: &IIIClient, target: &str) -> tokio::sync::mpsc::UnboundedReceiver<Value> {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    iii.register_function(
        target,
        RegisterFunction::new_async(move |payload: Value| {
            let tx = tx.clone();
            async move {
                let _ = tx.send(payload);
                Ok::<Value, Error>(json!({"ok": true}))
            }
        }),
    );
    rx
}

/// Observe `state::on-changed` with `filters`, delivering the event itself to
/// `target`, and wait until the observer is active: observing is
/// asynchronous (engine -> host -> ack), so a write before that would race.
async fn observe_changes(iii: &IIIClient, filters: Value, target: &str) -> ObserverHandle {
    let handle = iii.observe(
        Source::function(ON_CHANGED_FN)
            .filters(filters)
            .deliver(Deliver::Output),
        target,
    );
    tokio::time::timeout(Duration::from_secs(5), handle.ready())
        .await
        .expect("observer never became active")
        .expect("observing state::on-changed");
    handle
}

/// Poll the live config cell until `max_value_bytes` matches `expected`.
async fn wait_for_max_value_bytes(config: &ConfigCell, expected: Option<usize>) {
    for _ in 0..40 {
        if config.read().await.max_value_bytes == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("max_value_bytes reload never propagated to the config cell");
}

/// Poll the live config cell until `events_enabled` matches `expected`.
async fn wait_for_events_enabled(config: &ConfigCell, expected: Option<bool>) {
    for _ in 0..40 {
        if config.read().await.events_enabled == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("events_enabled reload never propagated to the config cell");
}

#[tokio::test]
#[serial]
async fn set_then_get_roundtrip_with_old_value_parity() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), in_memory_config())
        .await
        .expect("state worker should boot");

    let scope = unique("roundtrip");
    let key = "k";

    let first = call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": key, "value": {"name": "Alice"}}),
    )
    .await
    .expect("first set");
    assert_eq!(first["old_value"], Value::Null);
    assert_eq!(first["new_value"], json!({"name": "Alice"}));

    let second = call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": key, "value": 2}),
    )
    .await
    .expect("second set");
    assert_eq!(second["old_value"], json!({"name": "Alice"}));
    assert_eq!(second["new_value"], json!(2));

    let got = call(&iii, "state::get", json!({"scope": scope, "key": key}))
        .await
        .expect("get");
    assert_eq!(got, json!(2));

    let missing = call(
        &iii,
        "state::get",
        json!({"scope": scope, "key": "missing"}),
    )
    .await
    .expect("get missing");
    assert_eq!(missing, Value::Null);

    boot.shutdown().await;
    iii.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn set_accepts_data_alias() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), in_memory_config())
        .await
        .expect("state worker should boot");

    let scope = unique("alias");
    call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": "k", "data": "hello"}),
    )
    .await
    .expect("set via data alias");

    let got = call(&iii, "state::get", json!({"scope": scope, "key": "k"}))
        .await
        .expect("get");
    assert_eq!(got, json!("hello"));

    boot.shutdown().await;
    iii.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn delete_returns_old_value_and_missing_is_null() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), in_memory_config())
        .await
        .expect("state worker should boot");

    let scope = unique("delete");
    call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": "k", "value": {"n": 1}}),
    )
    .await
    .expect("set");

    let deleted = call(&iii, "state::delete", json!({"scope": scope, "key": "k"}))
        .await
        .expect("delete");
    assert_eq!(deleted, json!({"n": 1}));

    let got = call(&iii, "state::get", json!({"scope": scope, "key": "k"}))
        .await
        .expect("get after delete");
    assert_eq!(got, Value::Null);

    let deleted_again = call(&iii, "state::delete", json!({"scope": scope, "key": "k"}))
        .await
        .expect("delete of missing key must not error");
    assert_eq!(deleted_again, Value::Null);

    boot.shutdown().await;
    iii.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn update_applies_ops_and_reports_errors() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), in_memory_config())
        .await
        .expect("state worker should boot");

    let scope = unique("update");

    call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": "k", "value": {"count": 0}}),
    )
    .await
    .expect("set");

    let updated = call(
        &iii,
        "state::update",
        json!({"scope": scope, "key": "k", "ops": [{"type": "increment", "path": "count", "by": 2}]}),
    )
    .await
    .expect("update increment");
    assert_eq!(updated["new_value"]["count"], 2);
    assert!(
        updated.get("errors").is_none(),
        "no errors expected: {updated:?}"
    );

    call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": "bad", "value": {"count": "not-a-number"}}),
    )
    .await
    .expect("seed bad value");

    let failed = call(
        &iii,
        "state::update",
        json!({"scope": scope, "key": "bad", "ops": [{"type": "increment", "path": "count", "by": 1}]}),
    )
    .await
    .expect("update on a non-numeric path still succeeds the call, with errors[]");
    let errors = failed["errors"].as_array().expect("errors array present");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["code"], "increment.not_number");
    // The failed op leaves the value untouched.
    assert_eq!(failed["new_value"]["count"], "not-a-number");

    boot.shutdown().await;
    iii.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn list_and_list_groups() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), in_memory_config())
        .await
        .expect("state worker should boot");

    let main_scope = unique("list-main");
    for k in ["a", "b", "c"] {
        call(
            &iii,
            "state::set",
            json!({"scope": main_scope, "key": k, "value": k}),
        )
        .await
        .expect("set for list");
    }
    let listed = call(&iii, "state::list", json!({"scope": main_scope}))
        .await
        .expect("list");
    assert_eq!(listed.as_array().expect("array").len(), 3);

    let alpha = unique("alpha");
    let beta = unique("beta");
    let gamma = unique("gamma");
    // Two writes into `alpha` — list_groups must still dedup it to one entry.
    call(
        &iii,
        "state::set",
        json!({"scope": alpha, "key": "k1", "value": 1}),
    )
    .await
    .expect("set alpha 1");
    call(
        &iii,
        "state::set",
        json!({"scope": alpha, "key": "k2", "value": 2}),
    )
    .await
    .expect("set alpha 2");
    call(
        &iii,
        "state::set",
        json!({"scope": beta, "key": "k1", "value": 1}),
    )
    .await
    .expect("set beta");
    call(
        &iii,
        "state::set",
        json!({"scope": gamma, "key": "k1", "value": 1}),
    )
    .await
    .expect("set gamma");

    let groups_val = call(&iii, "state::list_groups", json!({}))
        .await
        .expect("list_groups");
    let mut groups: Vec<String> = groups_val["groups"]
        .as_array()
        .expect("groups array")
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    let mut expected = vec![
        main_scope.clone(),
        alpha.clone(),
        beta.clone(),
        gamma.clone(),
    ];
    expected.sort();
    groups.sort();
    assert_eq!(groups, expected, "expected sorted, deduped groups");

    boot.shutdown().await;
    iii.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn observers_receive_each_change_event() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), in_memory_config())
        .await
        .expect("state worker should boot");

    let scope = unique("events");
    let key = "k";

    let mut rx = event_sink(&iii, "e2e::on_state");
    let observer = observe_changes(
        &iii,
        json!({"input.scope": {"$eq": scope}}),
        "e2e::on_state",
    )
    .await;

    call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": key, "value": null}),
    )
    .await
    .expect("first set");
    call(
        &iii,
        "state::compare-and-set",
        json!({"scope": scope, "key": key, "value": {"name": "Bob"}}),
    )
    .await
    .expect("compare-and-set over stored null");
    call(&iii, "state::delete", json!({"scope": scope, "key": key}))
        .await
        .expect("delete");

    let mut by_type = std::collections::HashMap::new();
    for _ in 0..3 {
        let payload = tokio::time::timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("event delivery timed out")
            .expect("channel closed");
        let event_type = payload["event_type"].as_str().unwrap().to_string();
        by_type.insert(event_type, payload);
    }

    let created = &by_type["state:created"];
    assert_eq!(created["type"], "state");
    assert_eq!(created["scope"], scope);
    assert_eq!(created["key"], key);
    assert_eq!(created["old_value"], Value::Null);
    assert_eq!(created["new_value"], Value::Null);

    let updated = &by_type["state:updated"];
    assert_eq!(updated["old_value"], Value::Null);
    assert_eq!(updated["new_value"], json!({"name": "Bob"}));

    let deleted = &by_type["state:deleted"];
    assert_eq!(deleted["old_value"], json!({"name": "Bob"}));
    assert_eq!(deleted["new_value"], Value::Null);

    // `state::on-changed` is private: another worker cannot forge a change.
    // The other worker has its own name: the engine refuses a second live
    // worker under this process's default name.
    if let Some(other) = engine::connect_fresh_named(&unique("e2e-forger")).await {
        let err = call(
            &other,
            ON_CHANGED_FN,
            json!({"type": "state", "event_type": "state:created", "scope": scope,
                   "key": "forged", "old_value": null, "new_value": 1}),
        )
        .await
        .expect_err("a call from another worker must be refused")
        .to_string();
        assert!(err.contains("FORBIDDEN"), "{err}");
        other.shutdown_async().await;
    }
    let forged = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await;
    assert!(forged.is_err(), "a refused call reaches no observer");

    observer.dispose().await.expect("dispose observer");
    boot.shutdown().await;
    iii.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn filters_select_events_by_key_and_event_type() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), in_memory_config())
        .await
        .expect("state worker should boot");

    let scope = unique("filters");

    let mut status_rx = event_sink(&iii, "e2e::on_status");
    let mut deleted_rx = event_sink(&iii, "e2e::on_deleted");
    let by_key = observe_changes(
        &iii,
        json!({"input.scope": {"$eq": scope}, "input.key": {"$eq": "status"}}),
        "e2e::on_status",
    )
    .await;
    let by_type = observe_changes(
        &iii,
        json!({"input.scope": {"$eq": scope}, "input.event_type": {"$eq": "state:deleted"}}),
        "e2e::on_deleted",
    )
    .await;

    call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": "status", "value": "pending"}),
    )
    .await
    .expect("set status");
    call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": "other", "value": 1}),
    )
    .await
    .expect("set other");
    call(
        &iii,
        "state::delete",
        json!({"scope": scope, "key": "other"}),
    )
    .await
    .expect("delete other");

    let status = tokio::time::timeout(Duration::from_secs(3), status_rx.recv())
        .await
        .expect("status event timed out")
        .expect("channel closed");
    assert_eq!(status["key"], "status");
    assert_eq!(status["event_type"], "state:created");

    let deleted = tokio::time::timeout(Duration::from_secs(3), deleted_rx.recv())
        .await
        .expect("deleted event timed out")
        .expect("channel closed");
    assert_eq!(deleted["key"], "other");
    assert_eq!(deleted["old_value"], json!(1));

    // Nothing else matched either observer.
    let extra_status = tokio::time::timeout(Duration::from_millis(300), status_rx.recv()).await;
    assert!(extra_status.is_err(), "only key `status` reaches by_key");
    let extra_deleted = tokio::time::timeout(Duration::from_millis(100), deleted_rx.recv()).await;
    assert!(extra_deleted.is_err(), "only deletes reach by_type");

    by_key.dispose().await.expect("dispose observer");
    by_type.dispose().await.expect("dispose observer");
    boot.shutdown().await;
    iii.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn max_value_bytes_rejects_oversized_set() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), in_memory_config())
        .await
        .expect("state worker should boot");

    configuration::register_config(&iii, None)
        .await
        .expect("register state configuration schema");
    let config_observer =
        configuration::observe_config_changes(&iii, boot.ctx.clone(), boot.apply_lock.clone());
    tokio::time::timeout(Duration::from_secs(5), config_observer.ready())
        .await
        .expect("configuration observer never became active")
        .expect("observing configuration::on-updated");

    let scope = unique("config");

    // A listener wired up before any config changes, so the events-disabled
    // assertion later actually proves something (rather than being
    // vacuously true because nothing was listening).
    let mut rx = event_sink(&iii, "e2e::on_state_cfg");
    let observer = observe_changes(
        &iii,
        json!({"input.scope": {"$eq": scope}}),
        "e2e::on_state_cfg",
    )
    .await;

    // Flip `max_value_bytes` to 10 via the bus and wait for the reload to
    // land on the live config cell.
    call(
        &iii,
        "configuration::set",
        json!({"id": configuration::config_id(), "value": {"max_value_bytes": 10}}),
    )
    .await
    .expect("configuration::set max_value_bytes");
    wait_for_max_value_bytes(&boot.config, Some(10)).await;

    // Oversized value (well over 10 serialized bytes) is rejected.
    let oversized = call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": "big", "value": {"payload": "abcdefghijklmnopqrstuvwxyz"}}),
    )
    .await;
    let err = oversized
        .expect_err("oversized value must be rejected")
        .to_string();
    assert!(
        err.contains("VALUE_TOO_LARGE"),
        "unexpected error message: {err}"
    );

    // A small value still fits and gets written.
    call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": "small", "value": 5}),
    )
    .await
    .expect("small value should still be written");

    // Drain the events produced by the sets above before asserting silence
    // under the disabled gate.
    while tokio::time::timeout(Duration::from_millis(50), rx.recv())
        .await
        .is_ok()
    {}

    // Disable the change events entirely via the bus; the running worker
    // must apply it live (no restart).
    call(
        &iii,
        "configuration::set",
        json!({"id": configuration::config_id(), "value": {"events_enabled": false}}),
    )
    .await
    .expect("configuration::set events_enabled=false");
    wait_for_events_enabled(&boot.config, Some(false)).await;

    call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": "silent", "value": 1}),
    )
    .await
    .expect("set while events are disabled");
    let silence = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await;
    assert!(
        silence.is_err(),
        "no event should be emitted while events_enabled=false"
    );

    // Restore defaults so a subsequent run against the same engine process
    // starts from a clean slate.
    call(
        &iii,
        "configuration::set",
        json!({"id": configuration::config_id(), "value": in_memory_config().to_json()}),
    )
    .await
    .expect("restore default configuration");
    wait_for_events_enabled(&boot.config, Some(true)).await;
    wait_for_max_value_bytes(&boot.config, None).await;

    observer.dispose().await.expect("dispose observer");
    config_observer
        .dispose()
        .await
        .expect("dispose configuration observer");
    boot.shutdown().await;
    iii.shutdown_async().await;
}

/// An entry an earlier release left in the configuration worker: its own
/// schema, and the events knob under [`EARLIER_EVENTS_KEY`].
async fn store_earlier_release_entry(iii: &IIIClient, id: &str, value: Value) {
    call(
        iii,
        "configuration::ensure",
        json!({
            "id": id,
            "name": "State",
            "description": "the entry as an earlier state release registered it",
            "schema": {"type": "object"},
            "initial_value": value,
        }),
    )
    .await
    .expect("configuration::ensure the earlier entry");
}

#[tokio::test]
#[serial]
async fn an_entry_stored_by_an_earlier_release_loads_through_the_configuration_worker() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };

    // The knob under its earlier name only: it becomes `events_enabled`.
    let id = unique("state-upgrade");
    store_earlier_release_entry(
        &iii,
        &id,
        json!({ EARLIER_EVENTS_KEY: false, "max_value_bytes": 64 }),
    )
    .await;
    configuration::register_config_as(&iii, &id, None)
        .await
        .expect("registering over the earlier entry");
    let loaded = configuration::fetch_config_from(&iii, &id)
        .await
        .expect("the earlier entry loads through configuration::get");
    assert_eq!(loaded.events_enabled, Some(false));
    assert_eq!(loaded.max_value_bytes, Some(64));
    let stored = call(&iii, "configuration::get", json!({"id": id, "raw": true}))
        .await
        .expect("raw configuration::get");
    assert_eq!(
        stored["value"],
        json!({"events_enabled": false, "max_value_bytes": 64}),
        "the stored entry is rewritten to the published schema"
    );
    let schema = call(&iii, "configuration::schema", json!({"id": id}))
        .await
        .expect("configuration::schema");
    assert_eq!(
        schema["schema"]["additionalProperties"],
        json!(false),
        "the published schema stays closed: {schema}"
    );

    // Both names: `events_enabled` wins and the earlier key goes.
    let both = unique("state-upgrade-both");
    store_earlier_release_entry(
        &iii,
        &both,
        json!({ EARLIER_EVENTS_KEY: true, "events_enabled": false }),
    )
    .await;
    configuration::register_config_as(&iii, &both, None)
        .await
        .expect("registering over an entry naming both knobs");
    let loaded = configuration::fetch_config_from(&iii, &both)
        .await
        .expect("an entry naming both knobs loads");
    assert_eq!(loaded.events_enabled, Some(false));

    // A current entry stays as stored.
    let current = unique("state-current");
    store_earlier_release_entry(&iii, &current, json!({"events_enabled": false})).await;
    configuration::register_config_as(&iii, &current, None)
        .await
        .expect("registering over a current entry");
    let stored = call(
        &iii,
        "configuration::get",
        json!({"id": current, "raw": true}),
    )
    .await
    .expect("raw configuration::get");
    assert_eq!(stored["value"], json!({"events_enabled": false}));

    iii.shutdown_async().await;
}

/// The `## Functions` section of the worker's README.
fn readme_functions_section() -> &'static str {
    const README: &str = include_str!("../README.md");
    let start = README
        .find("\n## Functions\n")
        .expect("the README has a Functions section");
    let rest = &README[start + 1..];
    let end = rest[3..].find("\n## ").map_or(rest.len(), |at| at + 3);
    &rest[..end]
}

#[tokio::test]
#[serial]
async fn the_readme_lists_every_function_the_worker_registers() {
    // A named connection: the engine lists each function with the name of
    // the worker hosting it, and a claim is granted to that name only.
    let name = unique("e2e-readme");
    let Some(iii) = engine::connect_fresh_named(&name).await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), in_memory_config())
        .await
        .expect("state worker should boot");
    let config_observer =
        configuration::observe_config_changes(&iii, boot.ctx.clone(), boot.apply_lock.clone());
    // A claim registers the namespace's accessors.
    call(
        &iii,
        "state::claim-namespace",
        json!({"functions_prefix": name, "scopes": [unique("readme-private")]}),
    )
    .await
    .expect("claim a namespace");

    let section = readme_functions_section();
    let accessor_prefix = format!("{name}::");
    let mut registered = Vec::new();
    for _ in 0..40 {
        // Internal functions (`metadata.internal`) are listed on request.
        let listed = call(
            &iii,
            "engine::functions::list",
            json!({"worker": name, "include_internal": true}),
        )
        .await
        .expect("engine::functions::list");
        registered = listed["functions"]
            .as_array()
            .expect("functions array")
            .iter()
            .filter(|function| function["worker_name"] == json!(name))
            .filter_map(|function| function["function_id"].as_str())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if registered.iter().any(|id| id.starts_with(&accessor_prefix)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    // Shut down before asserting: a failed assertion would leave this
    // connection hosting state::* for every later test.
    config_observer
        .dispose()
        .await
        .expect("dispose configuration observer");
    boot.shutdown().await;
    iii.shutdown_async().await;

    for expected in [
        "state::set",
        "state::ui-content",
        "state::compare-and-set",
        "state::barrier",
        "state::claim-namespace",
        "state::on-config-change",
    ] {
        assert!(
            registered.iter().any(|id| id == expected),
            "{expected} is registered: {registered:?}"
        );
    }
    // Boot also restores the accessors of namespaces claimed earlier in this
    // process: every `<prefix>::state::*` is one README row.
    let missing: Vec<String> = registered
        .iter()
        .map(|id| match id.split_once("::state::") {
            Some((_, accessor)) => format!("`<prefix>::state::{accessor}`"),
            None => format!("`{id}`"),
        })
        .filter(|listed| !section.contains(listed.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "the README Functions table leaves out {missing:?}"
    );
}

/// Assert a public `state::*` call is locked out of a reserved scope.
async fn expect_reserved(iii: &iii_sdk::IIIClient, function_id: &str, payload: Value) {
    let err = call(iii, function_id, payload)
        .await
        .expect_err(&format!("{function_id} must reject the reserved scope"))
        .to_string();
    assert!(err.contains("RESERVED_SCOPE"), "{function_id}: {err}");
}

#[tokio::test]
#[serial]
async fn claim_namespace_lifecycle() {
    // The claim is authorized by worker identity, so this test's connection
    // carries an explicit name — the only namespace it may claim.
    let name = unique("e2e-claimant");
    let Some(iii) = engine::connect_fresh_named(&name).await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), in_memory_config())
        .await
        .expect("state worker should boot");

    let private_scope = unique("private");

    // Malformed claims are rejected before any identity work.
    let err = call(
        &iii,
        "state::claim-namespace",
        json!({"functions_prefix": "bad prefix!", "scopes": [private_scope]}),
    )
    .await
    .expect_err("a prefix must be a function-id segment")
    .to_string();
    assert!(err.contains("INVALID_PREFIX"), "{err}");
    let err = call(
        &iii,
        "state::claim-namespace",
        json!({"functions_prefix": name, "scopes": [""]}),
    )
    .await
    .expect_err("empty scopes are meaningless")
    .to_string();
    assert!(err.contains("INVALID_SCOPE"), "{err}");

    // A namespace that is not the caller's own name is forbidden, and the
    // error names the identity the engine actually saw.
    let err = call(
        &iii,
        "state::claim-namespace",
        json!({"functions_prefix": "someone-else", "scopes": [private_scope]}),
    )
    .await
    .expect_err("claiming a foreign namespace must fail")
    .to_string();
    assert!(err.contains("FORBIDDEN") && err.contains(&name), "{err}");

    // State-worker bookkeeping scopes cannot be claimed by anyone.
    let err = call(
        &iii,
        "state::claim-namespace",
        json!({"functions_prefix": name, "scopes": [iii_state::functions::CLAIMS_SCOPE]}),
    )
    .await
    .expect_err("the claims ledger itself must be unclaimable")
    .to_string();
    assert!(err.contains("RESERVED_SCOPE"), "{err}");

    // The real claim: reserves the scope and registers the accessors.
    let claimed = call(
        &iii,
        "state::claim-namespace",
        json!({"functions_prefix": name, "scopes": [private_scope]}),
    )
    .await
    .expect("claiming your own namespace");
    assert_eq!(claimed["claimed"], json!(true));
    let get_id = format!("{name}::state::get");
    let list_id = format!("{name}::state::list");
    let cas_id = format!("{name}::state::compare-and-set");
    let functions: Vec<&str> = claimed["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert_eq!(functions, vec![&get_id, &list_id, &cas_id]);

    // An observer of the private scope: private writes must NEVER reach
    // `state::on-changed` (asserted at the end).
    let mut rx = event_sink(&iii, "e2e::on_private");
    let observer = observe_changes(
        &iii,
        json!({"input.scope": {"$eq": private_scope}}),
        "e2e::on_private",
    )
    .await;

    // Every public verb is locked out of the claimed scope.
    expect_reserved(
        &iii,
        "state::set",
        json!({"scope": private_scope, "key": "k", "value": 1}),
    )
    .await;
    expect_reserved(
        &iii,
        "state::get",
        json!({"scope": private_scope, "key": "k"}),
    )
    .await;
    expect_reserved(
        &iii,
        "state::update",
        json!({"scope": private_scope, "key": "k", "ops": [{"type": "increment", "path": "n", "by": 1}]}),
    )
    .await;
    expect_reserved(
        &iii,
        "state::delete",
        json!({"scope": private_scope, "key": "k"}),
    )
    .await;
    expect_reserved(&iii, "state::list", json!({"scope": private_scope})).await;
    expect_reserved(
        &iii,
        "state::compare-and-set",
        json!({"scope": private_scope, "key": "k", "value": 1}),
    )
    .await;

    // The accessors DO reach it: claim a slot, miss on a stale expectation,
    // read it back, list the scope.
    let swap = call(
        &iii,
        &cas_id,
        json!({"scope": private_scope, "key": "slot", "value": {"owner": "a"}}),
    )
    .await
    .expect("accessor set-if-absent");
    assert_eq!(swap["swapped"], json!(true));

    let miss = call(
        &iii,
        &cas_id,
        json!({"scope": private_scope, "key": "slot", "expected": {"owner": "z"}, "value": {"owner": "b"}}),
    )
    .await
    .expect("accessor CAS miss still answers");
    assert_eq!(miss["swapped"], json!(false));
    assert_eq!(miss["current"], json!({"owner": "a"}));

    let got = call(
        &iii,
        &get_id,
        json!({"scope": private_scope, "key": "slot"}),
    )
    .await
    .expect("accessor get");
    assert_eq!(got, json!({"owner": "a"}));

    let listed = call(&iii, &list_id, json!({"scope": private_scope}))
        .await
        .expect("accessor list");
    assert_eq!(listed.as_array().expect("array").len(), 1);

    // Hard-scoped: the accessor cannot leave its own namespace.
    let err = call(&iii, &get_id, json!({"scope": "agent_state", "key": "k"}))
        .await
        .expect_err("an accessor must not read foreign scopes")
        .to_string();
    assert!(err.contains("INVALID_SCOPE"), "{err}");

    // Re-claiming what is already owned is a no-op…
    let again = call(
        &iii,
        "state::claim-namespace",
        json!({"functions_prefix": name, "scopes": [private_scope]}),
    )
    .await
    .expect("re-claim");
    assert_eq!(again["claimed"], json!(false));

    // …and growing the claim reserves only the new scope, with the existing
    // accessors serving it immediately (no re-registration).
    let extra_scope = unique("private-extra");
    let grown = call(
        &iii,
        "state::claim-namespace",
        json!({"functions_prefix": name, "scopes": [extra_scope]}),
    )
    .await
    .expect("grow the claim");
    assert_eq!(grown["claimed"], json!(true));
    assert_eq!(grown["scopes"].as_array().expect("scopes").len(), 2);
    let empty = call(&iii, &get_id, json!({"scope": extra_scope, "key": "k"}))
        .await
        .expect("accessor covers the grown scope");
    assert_eq!(empty, Value::Null);

    // Claimed scopes stay out of public listings.
    let visible_scope = unique("visible");
    call(
        &iii,
        "state::set",
        json!({"scope": visible_scope, "key": "k", "value": 1}),
    )
    .await
    .expect("public set");
    let groups = call(&iii, "state::list_groups", json!({}))
        .await
        .expect("list_groups");
    let groups: Vec<&str> = groups["groups"]
        .as_array()
        .expect("groups array")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(groups.contains(&visible_scope.as_str()));
    assert!(!groups.contains(&private_scope.as_str()));
    assert!(!groups.contains(&iii_state::functions::CLAIMS_SCOPE));

    // And none of the private writes above reached the observer.
    let silence = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await;
    assert!(
        silence.is_err(),
        "private writes must not emit state::on-changed"
    );

    observer.dispose().await.expect("dispose observer");
    boot.shutdown().await;
    iii.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn claims_survive_a_state_restart() {
    let name = unique("e2e-phoenix");
    let dir = std::env::temp_dir().join(unique("state-e2e-store"));
    let file_config = || {
        serde_json::from_value::<StateConfig>(json!({
            "adapter": {"name": "kv", "config": {
                "store_method": "file_based",
                "file_path": dir.to_string_lossy(),
                "save_interval_ms": 100,
            }},
        }))
        .expect("file-backed state config")
    };

    // First life: claim, then wait for the ledger to flush to disk (the
    // claim is the only write, so any file in the dir is the ledger).
    let Some(iii) = engine::connect_fresh_named(&name).await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), file_config())
        .await
        .expect("state worker should boot");
    let scope = unique("phoenix");
    let claimed = call(
        &iii,
        "state::claim-namespace",
        json!({"functions_prefix": name, "scopes": [scope]}),
    )
    .await
    .expect("claim");
    assert_eq!(claimed["claimed"], json!(true));
    for _ in 0..50 {
        if std::fs::read_dir(&dir)
            .map(|d| d.count() >= 1)
            .unwrap_or(false)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        std::fs::read_dir(&dir)
            .map(|d| d.count() >= 1)
            .unwrap_or(false),
        "the claims ledger never flushed to {}",
        dir.display()
    );
    boot.shutdown().await;
    iii.shutdown_async().await;

    // Second life: a fresh connection and a fresh boot over the same store.
    // Restoration must re-reserve the scope and re-register the accessors
    // without anyone re-claiming.
    let Some(iii2) = engine::connect_fresh().await else {
        return;
    };
    let boot2 = iii_state::boot::start(iii2.clone(), file_config())
        .await
        .expect("state worker should boot again");
    expect_reserved(&iii2, "state::get", json!({"scope": scope, "key": "k"})).await;
    let restored = call(
        &iii2,
        &format!("{name}::state::get"),
        json!({"scope": scope, "key": "k"}),
    )
    .await
    .expect("the restored accessor must serve without a re-claim");
    assert_eq!(restored, Value::Null);

    boot2.shutdown().await;
    iii2.shutdown_async().await;
    std::fs::remove_dir_all(&dir).ok();
}

#[tokio::test]
#[serial]
async fn cas_misses_and_barrier_fan_in_via_bus() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), in_memory_config())
        .await
        .expect("state worker should boot");

    let scope = unique("cas");

    // Set-if-absent creates…
    let created = call(
        &iii,
        "state::compare-and-set",
        json!({"scope": scope, "key": "counter", "value": 1}),
    )
    .await
    .expect("set-if-absent");
    assert_eq!(created["swapped"], json!(true));

    // …a stale expectation misses and returns the truth to retry against…
    let miss = call(
        &iii,
        "state::compare-and-set",
        json!({"scope": scope, "key": "counter", "expected": 9, "value": 2}),
    )
    .await
    .expect("a CAS miss answers rather than erroring");
    assert_eq!(miss["swapped"], json!(false));
    assert_eq!(miss["current"], json!(1));

    // …and the retry with the returned current succeeds.
    let hit = call(
        &iii,
        "state::compare-and-set",
        json!({"scope": scope, "key": "counter", "expected": 1, "value": 2}),
    )
    .await
    .expect("retry with the observed value");
    assert_eq!(hit["swapped"], json!(true));
    let got = call(
        &iii,
        "state::get",
        json!({"scope": scope, "key": "counter"}),
    )
    .await
    .expect("get");
    assert_eq!(got, json!(2));

    // A barrier invoked without its condition_config is a usage error.
    let err = call(&iii, "state::barrier", json!({"event": {}}))
        .await
        .expect_err("a barrier needs condition_config")
        .to_string();
    assert!(err.contains("BARRIER_CONFIG"), "{err}");

    // Fan-in of two arrivals: skip, then allow EXACTLY once with both
    // payloads, then skip again for late arrivals.
    let barrier_id = unique("join");
    let arrive = |key: &str, value: i64| {
        let iii = iii.clone();
        let barrier_id = barrier_id.clone();
        let key = key.to_string();
        async move {
            call(
                &iii,
                "state::barrier",
                json!({
                    "condition_config": {"id": barrier_id, "expect": 2},
                    "event": {"key": key, "new_value": value},
                }),
            )
            .await
        }
    };

    let first = arrive("a", 1).await.expect("first arrival");
    assert_eq!(first["decision"], json!("skip"));
    assert!(
        first["reason"].as_str().unwrap().contains("waiting"),
        "{first}"
    );

    let second = arrive("b", 2).await.expect("completing arrival");
    assert_eq!(second["decision"], json!("allow"));
    let results = &second["payload"]["results"];
    assert_eq!(results["a"]["new_value"], json!(1));
    assert_eq!(results["b"]["new_value"], json!(2));

    let late = arrive("c", 3).await.expect("late arrival");
    assert_eq!(late["decision"], json!("skip"));
    assert!(
        late["reason"]
            .as_str()
            .unwrap()
            .contains("already complete"),
        "{late}"
    );

    boot.shutdown().await;
    iii.shutdown_async().await;
}

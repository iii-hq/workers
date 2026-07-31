//! End-to-end coverage against a running engine: `state::*` functions and the
//! `state` trigger type invoked through the REAL bus path (`IIIClient::trigger`
//! / `register_trigger`), the same path every other worker uses.
//!
//! Connect-or-skip like every e2e suite (see `common::engine`). Every test
//! uses a DEDICATED connection (`engine::connect_fresh`), never the shared
//! one: `boot::start` registers fixed function ids (`state::set`,
//! `state::get`, ...) on every call, and the SDK panics if the same id is
//! registered twice on one client.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use common::engine;
use iii_sdk::RegisterFunction;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_state::config::StateConfig;
use iii_state::configuration;
use iii_state::functions::ConfigCell;
use iii_state::trigger::TriggerTable;
use serde_json::{Value, json};
use serial_test::serial;
use uuid::Uuid;

/// A fresh, uuid-suffixed scope/key namespace per test so reruns (and, within
/// a test, sibling scopes) never collide.
fn unique(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4())
}

async fn call(iii: &iii_sdk::IIIClient, function_id: &str, payload: Value) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(5_000),
    })
    .await
}

/// Poll the trigger fan-out table until it holds `n` bindings — trigger
/// registration propagates asynchronously (engine -> handler -> table), so a
/// fixed sleep before firing an event would be racy.
async fn wait_for_trigger_count(triggers: &TriggerTable, n: usize) {
    for _ in 0..50 {
        if triggers.read().await.len() >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("state trigger registration never propagated to the fan-out table");
}

/// Poll an `AtomicUsize` call counter until it reaches `n`.
async fn wait_for_calls(counter: &AtomicUsize, n: usize) {
    for _ in 0..50 {
        if counter.load(Ordering::SeqCst) >= n {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!(
        "expected {n} call(s), got {}",
        counter.load(Ordering::SeqCst)
    );
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

/// Poll the live config cell until `triggers_enabled` matches `expected`.
async fn wait_for_triggers_enabled(config: &ConfigCell, expected: Option<bool>) {
    for _ in 0..40 {
        if config.read().await.triggers_enabled == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("triggers_enabled reload never propagated to the config cell");
}

#[tokio::test]
#[serial]
async fn set_then_get_roundtrip_with_old_value_parity() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), StateConfig::default())
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
    let boot = iii_state::boot::start(iii.clone(), StateConfig::default())
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
    let boot = iii_state::boot::start(iii.clone(), StateConfig::default())
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
    let boot = iii_state::boot::start(iii.clone(), StateConfig::default())
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
    let boot = iii_state::boot::start(iii.clone(), StateConfig::default())
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
async fn state_trigger_fires_with_event_payload() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), StateConfig::default())
        .await
        .expect("state worker should boot");

    let scope = unique("events");
    let key = "k";

    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    iii.register_function(
        "e2e::on_state",
        RegisterFunction::new_async(move |payload: Value| {
            let tx = tx.clone();
            async move {
                let _ = tx.send(payload);
                Ok::<Value, Error>(json!({"ok": true}))
            }
        }),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_state::TRIGGER_TYPE.to_string(),
        function_id: "e2e::on_state".to_string(),
        config: json!({"scope": scope}),
        metadata: None,
    })
    .expect("register state trigger");
    wait_for_trigger_count(&boot.triggers, 1).await;

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

    boot.shutdown().await;
    iii.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn condition_false_blocks_null_passes() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), StateConfig::default())
        .await
        .expect("state worker should boot");

    let scope = unique("condition");

    iii.register_function(
        "e2e::cond_false",
        RegisterFunction::new_async(
            |_payload: Value| async move { Ok::<Value, Error>(json!(false)) },
        ),
    );
    iii.register_function(
        "e2e::cond_null",
        RegisterFunction::new_async(
            |_payload: Value| async move { Ok::<Value, Error>(Value::Null) },
        ),
    );

    let calls_false = Arc::new(AtomicUsize::new(0));
    {
        let calls_false = calls_false.clone();
        iii.register_function(
            "e2e::backend_false",
            RegisterFunction::new_async(move |_payload: Value| {
                let calls_false = calls_false.clone();
                async move {
                    calls_false.fetch_add(1, Ordering::SeqCst);
                    Ok::<Value, Error>(json!({"ok": true}))
                }
            }),
        );
    }

    let calls_null = Arc::new(AtomicUsize::new(0));
    {
        let calls_null = calls_null.clone();
        iii.register_function(
            "e2e::backend_null",
            RegisterFunction::new_async(move |_payload: Value| {
                let calls_null = calls_null.clone();
                async move {
                    calls_null.fetch_add(1, Ordering::SeqCst);
                    Ok::<Value, Error>(json!({"ok": true}))
                }
            }),
        );
    }

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_state::TRIGGER_TYPE.to_string(),
        function_id: "e2e::backend_false".to_string(),
        config: json!({"scope": scope, "condition_function_id": "e2e::cond_false"}),
        metadata: None,
    })
    .expect("register false-condition trigger");
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_state::TRIGGER_TYPE.to_string(),
        function_id: "e2e::backend_null".to_string(),
        config: json!({"scope": scope, "condition_function_id": "e2e::cond_null"}),
        metadata: None,
    })
    .expect("register null-condition trigger");
    wait_for_trigger_count(&boot.triggers, 2).await;

    call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": "k", "value": 1}),
    )
    .await
    .expect("set");

    wait_for_calls(&calls_null, 1).await;
    assert_eq!(
        calls_false.load(Ordering::SeqCst),
        0,
        "explicit `false` condition must block its binding"
    );

    boot.shutdown().await;
    iii.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn max_value_bytes_rejects_oversized_set() {
    let Some(iii) = engine::connect_fresh().await else {
        return;
    };
    let boot = iii_state::boot::start(iii.clone(), StateConfig::default())
        .await
        .expect("state worker should boot");

    configuration::register_config(&iii, None)
        .await
        .expect("register state configuration schema");
    configuration::register_config_trigger(&iii, boot.ctx.clone(), boot.apply_lock.clone())
        .expect("bind configuration reload trigger");

    let scope = unique("config");

    // A listener wired up before any config changes, so the trigger-disable
    // assertion later actually proves something (rather than being
    // vacuously true because nothing was listening).
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    iii.register_function(
        "e2e::on_state_cfg",
        RegisterFunction::new_async(move |payload: Value| {
            let tx = tx.clone();
            async move {
                let _ = tx.send(payload);
                Ok::<Value, Error>(json!({"ok": true}))
            }
        }),
    );
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_state::TRIGGER_TYPE.to_string(),
        function_id: "e2e::on_state_cfg".to_string(),
        config: json!({"scope": scope}),
        metadata: None,
    })
    .expect("register state trigger");
    wait_for_trigger_count(&boot.triggers, 1).await;

    // Flip `max_value_bytes` to 10 via the bus and wait for the reload to
    // land on the live config cell.
    call(
        &iii,
        "configuration::set",
        json!({"id": configuration::CONFIG_ID, "value": {"max_value_bytes": 10}}),
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

    // Drain the events produced by the trigger-enabled sets above before
    // asserting silence under the disabled gate.
    while tokio::time::timeout(Duration::from_millis(50), rx.recv())
        .await
        .is_ok()
    {}

    // Disable fan-out entirely via the bus; the running worker must apply it
    // live (no restart).
    call(
        &iii,
        "configuration::set",
        json!({"id": configuration::CONFIG_ID, "value": {"triggers_enabled": false}}),
    )
    .await
    .expect("configuration::set triggers_enabled=false");
    wait_for_triggers_enabled(&boot.config, Some(false)).await;

    call(
        &iii,
        "state::set",
        json!({"scope": scope, "key": "silent", "value": 1}),
    )
    .await
    .expect("set while triggers are disabled");
    let silence = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await;
    assert!(
        silence.is_err(),
        "no event should fire while triggers_enabled=false"
    );

    // Restore defaults so a subsequent run against the same engine process
    // starts from a clean slate.
    call(
        &iii,
        "configuration::set",
        json!({"id": configuration::CONFIG_ID, "value": StateConfig::default().to_json()}),
    )
    .await
    .expect("restore default configuration");
    wait_for_triggers_enabled(&boot.config, Some(true)).await;
    wait_for_max_value_bytes(&boot.config, None).await;

    boot.shutdown().await;
    iii.shutdown_async().await;
}

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
        namespace: iii.namespace(),
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
        namespace: iii.namespace(),
    })
    .expect("register false-condition trigger");
    iii.register_trigger(RegisterTriggerInput {
        trigger_type: iii_state::TRIGGER_TYPE.to_string(),
        function_id: "e2e::backend_null".to_string(),
        config: json!({"scope": scope, "condition_function_id": "e2e::cond_null"}),
        metadata: None,
        namespace: iii.namespace(),
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
        namespace: iii.namespace(),
    })
    .expect("register state trigger");
    wait_for_trigger_count(&boot.triggers, 1).await;

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
        json!({"id": configuration::config_id(), "value": {"triggers_enabled": false}}),
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
        json!({"id": configuration::config_id(), "value": StateConfig::default().to_json()}),
    )
    .await
    .expect("restore default configuration");
    wait_for_triggers_enabled(&boot.config, Some(true)).await;
    wait_for_max_value_bytes(&boot.config, None).await;

    boot.shutdown().await;
    iii.shutdown_async().await;
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
    let boot = iii_state::boot::start(iii.clone(), StateConfig::default())
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

    // A trigger bound to the private scope: private writes must NEVER reach
    // state-trigger fan-out (asserted at the end).
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Value>();
    iii.register_function(
        "e2e::on_private",
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
        function_id: "e2e::on_private".to_string(),
        config: json!({"scope": private_scope}),
        metadata: None,
    })
    .expect("register trigger on the private scope");
    wait_for_trigger_count(&boot.triggers, 1).await;

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

    // And none of the private writes above fanned out to the trigger.
    let silence = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await;
    assert!(
        silence.is_err(),
        "private writes must not fire state triggers"
    );

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
    let boot = iii_state::boot::start(iii.clone(), StateConfig::default())
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

//! End-to-end hot-reload coverage against a running engine + configuration
//! worker (mirrors `http/tests/e2e_reload.rs`).
//!
//! Skips (early-return) when no engine is reachable. Boots one bridge worker
//! with a forwards-free config, wires the `configuration:updated` reload
//! trigger, then drives the config through the real bus (`configuration::set`)
//! and asserts the running worker picks each step up WITHOUT a restart:
//!
//! 1. ADD a forward -> it becomes callable;
//! 2. REMOVE it -> calls fail with a `bridge_error` mentioning "removed";
//! 3. RE-ADD it -> callable again. This is the F1 regression: before the
//!    ever-registered id sets, the re-add looked "new", the duplicate
//!    `register_function` panicked the reload task, and the config cell never
//!    swapped — so the wait below would time out.

mod common;

use std::sync::Arc;
use std::time::Duration;

use common::engine;
use iii_bridge::configuration;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use serial_test::serial;

const FORWARD_ID: &str = "e2e::reload-fwd";
const TARGET_ID: &str = "e2e::reload-target";

fn echo_fn() -> RegisterFunction {
    RegisterFunction::new_async(|input: serde_json::Value| async move {
        Ok::<_, iii_sdk::Error>(json!({ "echo": input }))
    })
}

/// Store a new `bridge` config value through the real configuration bus —
/// this is what fires the `configuration:updated` event the reload trigger
/// listens for.
async fn set_bridge_config(iii: &Arc<IIIClient>, value: Value) {
    iii.trigger(TriggerRequest {
        function_id: "configuration::set".to_string(),
        payload: json!({ "id": configuration::config_id(), "value": value }),
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
    .expect("configuration::set");
}

/// Poll the live config cell until the forward's presence matches `present`
/// (the cell is published LAST in `on_config_change`, so this observes a
/// fully-applied reload), or panic.
async fn wait_for_forward(config: &configuration::ConfigCell, present: bool) {
    for _ in 0..100 {
        let has = config
            .read()
            .await
            .forward
            .iter()
            .any(|f| f.local_function == FORWARD_ID);
        if has == present {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("config reload (forward present={present}) never propagated to the cell");
}

#[tokio::test]
#[serial]
async fn forward_add_remove_readd_via_config_set() {
    let Some(caller) = engine::get_or_init().await else {
        return; // skip: engine absent
    };
    let Some(worker_local) = engine::connect_fresh().await else {
        return;
    };

    // The remote-side target the forward proxies to ("remote" == same engine
    // in e2e).
    caller.register_function(TARGET_ID, echo_fn());

    let url = engine::ws_url();
    let base_value = json!({ "url": url });
    let with_forward = json!({
        "url": url,
        "forward": [
            { "local_function": FORWARD_ID, "remote_function": TARGET_ID, "timeout_ms": 5000 }
        ]
    });

    // Boot forwards-free, register the schema (required before
    // `configuration::set`), pin the stored value to a known baseline (a
    // previous run may have left a forward stored), then wire the reload
    // trigger to this worker's live state.
    let config = iii_bridge::config::BridgeConfig {
        url: Some(url.clone()),
        ..Default::default()
    };
    let boot = iii_bridge::boot::start(worker_local.clone(), config)
        .await
        .expect("bridge worker should boot");
    configuration::register_config(&worker_local, None)
        .await
        .expect("register bridge configuration schema");
    set_bridge_config(&worker_local, base_value.clone()).await;
    configuration::register_config_trigger(&worker_local, &boot)
        .await
        .expect("bind configuration trigger");

    // 1. ADD the forward via configuration::set -> it becomes callable.
    set_bridge_config(&worker_local, with_forward.clone()).await;
    wait_for_forward(&boot.config, true).await;
    let res = common::trigger_until_ready(&caller, FORWARD_ID, json!({ "n": 1 }))
        .await
        .expect("forward should be callable after being added via config");
    assert_eq!(res["echo"]["n"], 1);

    // 2. REMOVE it -> the still-registered handler fails its table lookup
    // with a bridge_error mentioning "removed".
    set_bridge_config(&worker_local, base_value.clone()).await;
    wait_for_forward(&boot.config, false).await;
    let err = caller
        .trigger(TriggerRequest {
            function_id: FORWARD_ID.to_string(),
            payload: json!({ "n": 2 }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .expect_err("removed forward must fail");
    assert!(
        err.to_string().contains("removed"),
        "expected the removed-from-config bridge_error, got: {err}"
    );

    // 3. RE-ADD it -> callable again (F1 regression: a duplicate
    // register_function would panic the reload task and this wait would time
    // out because the cell never swaps).
    set_bridge_config(&worker_local, with_forward).await;
    wait_for_forward(&boot.config, true).await;
    let res = common::trigger_until_ready(&caller, FORWARD_ID, json!({ "n": 3 }))
        .await
        .expect("re-added forward should be callable again");
    assert_eq!(res["echo"]["n"], 3);

    // Leave the stored value at the baseline for the next run.
    set_bridge_config(&worker_local, base_value).await;

    boot.shutdown().await;
    worker_local.shutdown_async().await;
}

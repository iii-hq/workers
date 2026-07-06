mod common;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::RegisterFunction;
use serde_json::json;
use serial_test::serial;

use common::engine;

fn echo_fn() -> RegisterFunction {
    RegisterFunction::new_async(|input: serde_json::Value| async move {
        Ok::<_, iii_sdk::Error>(json!({ "echo": input }))
    })
}

#[tokio::test]
#[serial]
async fn bridge_invoke_forward_and_expose_roundtrip() {
    let Some(caller) = engine::get_or_init().await else {
        return; // skip: engine absent
    };
    let Some(worker_local) = engine::connect_fresh().await else {
        return;
    };

    // Backends living on the "remote" engine (same engine in e2e) and the
    // local function the bridge exposes.
    caller.register_function("e2e::bridge-target", echo_fn());
    caller.register_function("e2e::bridge-local-src", echo_fn());

    let config: iii_bridge::config::BridgeConfig = serde_yaml::from_str(&format!(
        r#"
url: {url}
expose:
  - local_function: e2e::bridge-local-src
    remote_function: e2e::bridge-exposed
forward:
  - local_function: e2e::bridge-forward
    remote_function: e2e::bridge-target
    timeout_ms: 5000
"#,
        url = engine::ws_url()
    ))
    .unwrap();

    let boot = iii_bridge::boot::start(worker_local.clone(), config)
        .await
        .expect("bridge worker should boot");

    // 1. bridge.invoke -> remote function, result comes back.
    let res = common::trigger_until_ready(
        &caller,
        "bridge.invoke",
        json!({ "function_id": "e2e::bridge-target", "data": { "n": 1 } }),
    )
    .await
    .expect("bridge.invoke");
    assert_eq!(res["echo"]["n"], 1);

    // 2. forward local function -> remote function.
    let res = common::trigger_until_ready(&caller, "e2e::bridge-forward", json!({ "n": 2 }))
        .await
        .expect("forward");
    assert_eq!(res["echo"]["n"], 2);

    // 3. exposed function (registered by the worker's REMOTE client) -> local function.
    let res = common::trigger_until_ready(&caller, "e2e::bridge-exposed", json!({ "n": 3 }))
        .await
        .expect("expose");
    assert_eq!(res["echo"]["n"], 3);

    // 4. bridge.invoke_async -> fire-and-forget, returns null.
    let res = common::trigger_until_ready(
        &caller,
        "bridge.invoke_async",
        json!({ "function_id": "e2e::bridge-target", "data": { "n": 4 } }),
    )
    .await
    .expect("invoke_async");
    assert!(res.is_null(), "NoResult parity: invoke_async returns null");

    boot.shutdown().await;
    worker_local.shutdown_async().await;
}

#[tokio::test]
#[serial]
async fn bridge_invoke_bad_input_surfaces_deserialization_error() {
    let Some(caller) = engine::get_or_init().await else {
        return;
    };
    let Some(worker_local) = engine::connect_fresh().await else {
        return;
    };

    let config = iii_bridge::config::BridgeConfig {
        url: Some(engine::ws_url()),
        ..Default::default()
    };
    let boot = iii_bridge::boot::start(worker_local.clone(), config)
        .await
        .expect("bridge worker should boot");

    // Make sure bridge.invoke is routable first (valid probe), then send garbage.
    let _ = common::trigger_until_ready(
        &caller,
        "bridge.invoke",
        json!({ "function_id": "engine::workers::list", "data": {} }),
    )
    .await
    .expect("probe invoke");

    let err = caller
        .trigger(TriggerRequest {
            function_id: "bridge.invoke".to_string(),
            payload: json!({ "bad": true }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .expect_err("missing function_id must fail");
    assert!(
        err.to_string().contains("Failed to parse invoke input"),
        "builtin error message parity, got: {err}"
    );

    boot.shutdown().await;
    worker_local.shutdown_async().await;
}

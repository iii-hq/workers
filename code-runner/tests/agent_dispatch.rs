//! Exercise the production host bridges, with no connected engine: denials
//! must happen before a guest's request can cross the worker boundary.
use code_runner::{node_bus::IIIEngine, python_bus::IIIBridge};
use iii_node_core::engine::Engine;
use iii_python_core::runner::GuestBridge;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const FORBIDDEN: &[&str] = &[
    "provider::openai-codex::login::start",
    "provider::openai-codex::login::poll",
    "provider::openai-codex::login::cancel",
    "provider::openai-codex::auth::status",
    "provider::openai-codex::auth::logout",
    "provider::openai-codex::auth::future-operation",
    "provider-openai-codex::state::get",
    "provider-openai-codex::state::list",
    "provider-openai-codex::state::compare-and-set",
    "provider-openai-codex::state::future-operation",
    "harness::state::get",
    "harness::state::compare-and-set",
    "state::claim-namespace",
    "engine::observers::dispose",
];

async fn denied<T: std::fmt::Debug>(
    target: &str,
    call: impl std::future::Future<Output = Result<T, String>>,
) {
    let error = tokio::time::timeout(Duration::from_millis(100), call)
        .await
        .expect("operator-only calls must not wait on the engine")
        .expect_err("operator-only calls must be denied");
    assert!(
        error.contains("operator-only") && error.contains(target),
        "{error}"
    );
}

#[tokio::test]
async fn node_and_python_reject_operator_calls_including_queued_and_void_calls() {
    let iii = Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:0"));
    let node = IIIEngine::new(iii.clone());
    let python = IIIBridge::new(iii);
    for &target in FORBIDDEN {
        for action in [
            None,
            Some(r#"{"type":"void"}"#.into()),
            Some(r#"{"type":"enqueue","queue":"q"}"#.into()),
        ] {
            denied(target, node.call(target.into(), json!({}), 10_000, action)).await;
        }
        denied(target, python.call(target.into(), json!({}), 10_000)).await;
    }
}

/// A guest observer can neither observe an operator-only function (its
/// inputs and outputs would reach the guest's target) nor target one (it
/// would run on every call of the guest's source).
#[tokio::test]
async fn guest_observers_cannot_observe_or_target_operator_functions() {
    let iii = Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:0"));
    let node = IIIEngine::new(iii);
    for &operator in FORBIDDEN {
        denied(
            operator,
            node.observe(json!({
                "source": { "function_id": operator },
                "target": "app::react",
                "options": { "id": "app::watch" },
            })),
        )
        .await;
        denied(
            operator,
            node.observe(json!({
                "source": { "function_id": "state::on-changed" },
                "target": operator,
                "options": { "id": "app::watch" },
            })),
        )
        .await;
    }
}

#[tokio::test]
async fn ordinary_calls_still_reach_the_transport() {
    let iii = Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:0"));
    let node = IIIEngine::new(iii.clone());
    let python = IIIBridge::new(iii);
    for target in [
        "state::get",
        "state::set",
        "provider::openai-codex::stream",
        "provider::openai-codex::authenticate",
        "provider-openai-codex::state-info",
    ] {
        for call in [
            node.call(target.into(), Value::Null, 1, None),
            python.call(target.into(), Value::Null, 1),
        ] {
            let error = tokio::time::timeout(Duration::from_secs(1), call)
                .await
                .expect("disconnected transport returns promptly")
                .expect_err("engine is disconnected");
            assert!(!error.contains("operator-only"), "{target}: {error}");
        }
    }
}

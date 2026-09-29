//! Bus contract of the SemIf provider over a mocked engine socket.
#[path = "../../judge-typesafe/tests/support/fake_engine.rs"]
mod fake_engine;
mod support;
use iii_sdk::{register_worker, InitOptions};
use judge_semif::{SemifClient, SemifConfig};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::timeout};

async fn invoke(payload: Value) -> (Value, Value) {
    invoke_function("judge-semif::evaluate", payload).await
}

async fn invoke_function(function_id: &'static str, payload: Value) -> (Value, Value) {
    invoke_client(
        function_id,
        payload,
        judge_semif::configuration::new_cell(SemifConfig::default()),
        support::tiny_client(),
    )
    .await
}

async fn invoke_client(
    function_id: &'static str,
    payload: Value,
    config: judge_semif::SharedConfig,
    client: SemifClient,
) -> (Value, Value) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut payload = Some(payload);
    let engine = fake_engine::start(move |message| {
        if message["type"] == "registerfunction" && message["id"] == function_id {
            tx.send(message).unwrap();
            let data = payload.take().expect("registered once");
            vec![json!({"type":"invokefunction","invocation_id":"00000000-0000-0000-0000-000000000001","function_id":function_id,"data":data})]
        } else {
            if message["type"] == "invocationresult" {
                tx.send(message).unwrap();
            }
            vec![]
        }
    })
    .await;
    let iii = Arc::new(register_worker(&engine.url, InitOptions::default()));
    // Loaded before the call, as a pinned provider is.
    let slot = iii_llama_runtime::ModelSlot::new(move || Ok(client.clone()));
    slot.get(std::time::Instant::now() + Duration::from_secs(10))
        .await
        .unwrap();
    judge_semif::register(&iii, config, slot);
    let registration = timeout(Duration::from_secs(2), rx.recv())
        .await
        .expect("requested function must be registered")
        .unwrap();
    let response = timeout(Duration::from_secs(2), rx.recv())
        .await
        .unwrap()
        .unwrap();
    iii.shutdown_async().await;
    (registration, response)
}

#[tokio::test]
async fn evaluate_registration_has_strict_schemas_and_answers_typed() {
    let (registration, response) = invoke(json!({
        "_caller_worker_id":"engine-worker", "timeout_ms":1000,
        "evaluations":[{"id":"t","state":"Refund me","questions":{"refund":{"type":"noul","instructions":"Asks for a refund?"}}}]
    }))
    .await;
    assert_eq!(
        registration["request_format"]["additionalProperties"],
        false
    );
    for field in ["api_key", "endpoint", "_caller_worker_id", "provider"] {
        assert!(registration["request_format"]["properties"]
            .get(field)
            .is_none());
    }
    assert_eq!(
        registration["response_format"]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(response.get("error").is_none());
    assert_eq!(response["result"]["status"], "ok", "{response}");
    assert!(response["result"]["results"]["t"]["answers"]["refund"]["noul"].is_f64());
    assert_eq!(response["result"]["stats"]["usage_complete"], true);
}

#[tokio::test]
async fn malformed_requests_fail_before_inference() {
    for payload in [
        json!({"_caller_worker_id":"engine-worker"}),
        json!({"timeout_ms":0,"evaluations":[]}),
        json!({"timeout_ms":1000,"evaluations":[{"id":"t","state":"x","questions":{}}]}),
        json!({"timeout_ms":1000,"evaluations":[{"id":"t","state":"x","questions":{"q":{"type":"noul"}}}],"api_key":"test-marker"}),
    ] {
        let (_, response) = invoke(payload).await;
        assert_eq!(response["result"]["code"], "invalid_request", "{response}");
        assert_eq!(response["result"]["stats"]["attempts"], 0);
        assert!(!response.to_string().contains("test-marker"));
    }
}

#[tokio::test]
async fn models_and_cancel_are_registered_with_the_provider_ids() {
    let (registration, response) = invoke_function(
        "judge-semif::models::list",
        json!({"_caller_worker_id":"w"}),
    )
    .await;
    assert!(registration["request_format"]["properties"]["timeout_ms"].is_object());
    assert_eq!(response["result"]["models"][0]["name"], "qwen3.5-4b");
    let (_, response) = invoke_function(
        "judge-semif::cancel",
        json!({"_caller_worker_id":"w","request_id":"nothing-active"}),
    )
    .await;
    assert_eq!(response["result"], json!({"status":"ok","cancelled":false}));
    let (_, response) =
        invoke_function("judge-semif::cancel", json!({"request_id":"no-caller"})).await;
    assert_eq!(response["result"]["code"], "invalid_request");
}

#[test]
fn provider_ids_follow_the_hub_convention() {
    use judge_contract::{
        provider_function_id, CANCEL_FUNCTION_ID, FUNCTION_ID, MODELS_FUNCTION_ID,
    };
    use judge_semif::{CANCEL_ID, EVALUATE_ID, MODELS_ID, PROVIDER};
    assert_eq!(provider_function_id(PROVIDER, FUNCTION_ID), EVALUATE_ID);
    assert_eq!(
        provider_function_id(PROVIDER, MODELS_FUNCTION_ID),
        MODELS_ID
    );
    assert_eq!(
        provider_function_id(PROVIDER, CANCEL_FUNCTION_ID),
        CANCEL_ID
    );
}

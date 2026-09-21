//! Routing, validation and contract enforcement over a mocked engine socket.
use futures_util::{SinkExt, StreamExt};
use iii_sdk::{register_worker, InitOptions};
use judge_contract::{CANCEL_FUNCTION_ID, FUNCTION_ID, MODELS_FUNCTION_ID};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::{net::TcpListener, sync::mpsc, time::timeout};
use tokio_tungstenite::{accept_async, tungstenite::Message};

/// Drive one public call through the hub. The fake engine answers the hub's
/// forwarded invocation with `provider_reply` (`Err(code)` = remote error) and
/// returns the registration frame, the forwarded frame if any, and the hub's
/// invocation result.
async fn invoke(
    function_id: &'static str,
    payload: Value,
    provider_reply: Result<Value, &'static str>,
) -> (Value, Option<Value>, Value) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = format!("ws://{}", listener.local_addr().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        socket
            .send(Message::Text(
                json!({"type":"workerregistered","worker_id":"judge-test-worker"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        while let Some(Ok(frame)) = socket.next().await {
            let Message::Text(text) = frame else {
                continue;
            };
            let message: Value = serde_json::from_str(&text).unwrap();
            match message["type"].as_str() {
                Some("registerfunction") if message["id"] == function_id => {
                    tx.send(("registration", message.clone())).unwrap();
                    socket.send(Message::Text(json!({"type":"invokefunction","invocation_id":"00000000-0000-0000-0000-000000000001","function_id":function_id,"data":payload}).to_string().into())).await.unwrap();
                }
                // The SDK also fires a void `engine::workers::register` at
                // connect; only awaited invocations are forwarded calls.
                Some("invokefunction") if !message["invocation_id"].is_null() => {
                    let mut reply = json!({"type":"invocationresult","invocation_id":message["invocation_id"],"function_id":message["function_id"]});
                    match &provider_reply {
                        Ok(result) => reply["result"] = result.clone(),
                        Err(code) => reply["error"] = json!({"code":code,"message":"mock"}),
                    }
                    tx.send(("forwarded", message)).unwrap();
                    socket
                        .send(Message::Text(reply.to_string().into()))
                        .await
                        .unwrap();
                }
                Some("invocationresult") => tx.send(("response", message)).unwrap(),
                _ => {}
            }
        }
    });
    let iii = Arc::new(register_worker(&address, InitOptions::default()));
    judge::register(
        &iii,
        judge::configuration::new_cell(judge::JudgeConfig::default()),
    );
    let (mut registration, mut forwarded, mut response) = (None, None, None);
    while response.is_none() {
        let (kind, frame) = timeout(Duration::from_secs(3), rx.recv())
            .await
            .expect("hub answers the invocation")
            .unwrap();
        match kind {
            "registration" => registration = Some(frame),
            "forwarded" => forwarded = Some(frame),
            _ => response = Some(frame),
        }
    }
    iii.shutdown_async().await;
    server.abort();
    (registration.unwrap(), forwarded, response.unwrap())
}

fn evaluation() -> Value {
    json!({
        "_caller_worker_id":"engine-worker", "request_id":"run-1", "timeout_ms":1000,
        "evaluations":[{"id":"ticket","state":{},"questions":{"urgent":{"type":"noul"}}}]
    })
}

fn ok_reply() -> Value {
    json!({
        "status":"ok", "model":"jev-1.13.0",
        "results":{"ticket":{"answers":{"urgent":{"type":"noul","noul":0.5}}}},
        "stats":{"attempts":1,"requests":1,"questions":1,"input_tokens":1,"output_tokens":1,"elapsed_ms":1,"usage_complete":true}
    })
}

#[tokio::test]
async fn evaluate_forwards_to_the_default_provider_with_a_caller_scoped_id() {
    let (registration, forwarded, response) =
        invoke(FUNCTION_ID, evaluation(), Ok(ok_reply())).await;
    assert_eq!(
        registration["request_format"]["additionalProperties"],
        false
    );
    assert_eq!(
        registration["request_format"]["properties"]["provider"]["type"],
        "string"
    );
    assert_eq!(
        registration["response_format"]["oneOf"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let forwarded = forwarded.expect("forwarded to the provider");
    assert_eq!(forwarded["function_id"], "judge-typesafe::evaluate");
    assert_eq!(forwarded["data"]["request_id"], "13:engine-worker/run-1");
    assert_eq!(forwarded["data"]["timeout_ms"], 1000);
    assert!(forwarded["data"].get("provider").is_none());
    assert!(forwarded["data"].get("_caller_worker_id").is_none());
    assert_eq!(response["result"], ok_reply());
}

#[tokio::test]
async fn provider_override_selects_the_worker_and_is_validated() {
    let mut payload = evaluation();
    payload["provider"] = json!("other");
    let (_, forwarded, response) = invoke(FUNCTION_ID, payload, Ok(ok_reply())).await;
    assert_eq!(forwarded.unwrap()["function_id"], "judge-other::evaluate");
    assert_eq!(response["result"]["status"], "ok");
    for provider in [
        json!(""),
        json!("A"),
        json!("../x"),
        json!("a::b"),
        json!("x".repeat(65)),
        json!(3),
    ] {
        let mut payload = evaluation();
        payload["provider"] = provider.clone();
        let (_, forwarded, response) = invoke(FUNCTION_ID, payload, Ok(ok_reply())).await;
        assert!(forwarded.is_none(), "{provider}: {forwarded:?}");
        assert_eq!(response["result"]["code"], "invalid_request", "{provider}");
        assert_eq!(response["result"]["stats"]["attempts"], 0);
    }
}

#[tokio::test]
async fn identified_calls_require_trusted_caller_metadata() {
    for (payload, function_id) in [
        (
            json!({"request_id":"run-1","timeout_ms":1000,"evaluations":[]}),
            FUNCTION_ID,
        ),
        (
            json!({"request_id":"x".repeat(129),"_caller_worker_id":"w"}),
            CANCEL_FUNCTION_ID,
        ),
        (json!({"request_id":"run-1"}), CANCEL_FUNCTION_ID),
        (json!("not an object"), MODELS_FUNCTION_ID),
    ] {
        let (_, forwarded, response) = invoke(function_id, payload.clone(), Ok(ok_reply())).await;
        assert!(forwarded.is_none(), "{payload}: {forwarded:?}");
        assert_eq!(response["result"]["code"], "invalid_request", "{payload}");
    }
    let (_, forwarded, response) = invoke(
        CANCEL_FUNCTION_ID,
        json!({"_caller_worker_id":"w","request_id":"run-1"}),
        Ok(json!({"status":"ok","cancelled":true})),
    )
    .await;
    let forwarded = forwarded.unwrap();
    assert_eq!(forwarded["function_id"], "judge-typesafe::cancel");
    assert_eq!(forwarded["data"], json!({"request_id":"1:w/run-1"}));
    assert_eq!(response["result"], json!({"status":"ok","cancelled":true}));
}

#[tokio::test]
async fn composed_ids_are_length_prefixed_so_slashes_cannot_collide() {
    let mut ids = Vec::new();
    for (caller, id) in [("a/b", "c"), ("a", "b/c")] {
        let (_, forwarded, _) = invoke(
            CANCEL_FUNCTION_ID,
            json!({"_caller_worker_id":caller,"request_id":id}),
            Ok(json!({"status":"ok","cancelled":false})),
        )
        .await;
        ids.push(
            forwarded.unwrap()["data"]["request_id"]
                .as_str()
                .unwrap()
                .to_owned(),
        );
    }
    assert_eq!(ids, ["3:a/b/c", "1:a/b/c"]);
}

#[tokio::test]
async fn provider_failures_map_to_typed_errors() {
    let (_, _, response) = invoke(FUNCTION_ID, evaluation(), Err("function_not_found")).await;
    assert_eq!(response["result"]["code"], "provider_unavailable");
    assert_eq!(response["result"]["stats"]["usage_complete"], false);
    let (_, _, response) = invoke(
        MODELS_FUNCTION_ID,
        json!({"_caller_worker_id":"w"}),
        Ok(json!({"status":"ok","bogus":1})),
    )
    .await;
    assert_eq!(response["result"]["code"], "invalid_response");
    let (_, _, response) = invoke(
        CANCEL_FUNCTION_ID,
        json!({"_caller_worker_id":"w","request_id":"r"}),
        Err("internal"),
    )
    .await;
    assert!(
        response.get("error").is_some(),
        "other bus failures stay bus failures: {response}"
    );
}

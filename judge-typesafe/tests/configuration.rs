//! Verify JEV's configuration RPC contract against a mock service.
//! The mock implements ensure's documented preservation rule; these tests do
//! not exercise engine persistence, serialization, or atomicity.
use futures_util::{SinkExt, StreamExt};
use iii_sdk::{register_worker, InitOptions};
use judge_typesafe::{configuration, JevConfig};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::{net::TcpListener, sync::mpsc, time::timeout};
use tokio_tungstenite::{accept_async, tungstenite::Message};

async fn register_and_fetch(
    mut stored: Value,
    seed: Option<JevConfig>,
    ensure_error: Option<Value>,
) -> (Result<JevConfig, String>, Vec<Value>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (tx, mut requests) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        socket
            .send(Message::Text(
                json!({"type":"workerregistered","worker_id":"configuration-test"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        while let Some(Ok(frame)) = socket.next().await {
            let Message::Text(text) = frame else {
                continue;
            };
            let request: Value = serde_json::from_str(&text).unwrap();
            if request["type"] != "invokefunction" {
                continue;
            }
            // The SDK also emits telemetry RPCs, unrelated to this contract.
            if !request["function_id"]
                .as_str()
                .unwrap()
                .starts_with("configuration::")
            {
                continue;
            }
            tx.send(request.clone()).unwrap();
            let result = match request["function_id"].as_str().unwrap() {
                "configuration::ensure" => match &ensure_error {
                    Some(error) => Err(error.clone()),
                    None => {
                        let action = if stored.is_null() {
                            stored = request["data"]["initial_value"].clone();
                            "seeded"
                        } else {
                            "preserved"
                        };
                        Ok(
                            json!({"action":action,"entry":{"id":request["data"]["id"],"value":stored}}),
                        )
                    }
                },
                "configuration::get" => Ok(json!({"value":stored})),
                _ => Err(json!({"code":"unexpected_rpc","message":"unexpected configuration RPC"})),
            };
            let mut response = json!({
                "type":"invocationresult",
                "invocation_id":request["invocation_id"],
                "function_id":request["function_id"],
            });
            match result {
                Ok(result) => response["result"] = result,
                Err(error) => response["error"] = error,
            }
            socket
                .send(Message::Text(response.to_string().into()))
                .await
                .unwrap();
        }
    });
    let iii = register_worker(&url, InitOptions::default());
    let result = timeout(Duration::from_secs(10), async {
        configuration::register_config(&iii, seed.as_ref()).await?;
        configuration::fetch_config(&iii).await
    })
    .await;
    tokio::task::spawn_blocking(move || iii.shutdown())
        .await
        .unwrap();
    server.abort();
    let mut calls = Vec::new();
    while let Ok(request) = requests.try_recv() {
        calls.push(request);
    }
    (result.expect("configuration RPCs finish"), calls)
}

fn assert_ensure_then_fetch(calls: &[Value], candidate: &JevConfig) {
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["function_id"], "configuration::ensure");
    assert_eq!(calls[1]["function_id"], "configuration::get");
    for call in calls {
        assert_eq!(call["namespace"], "default");
        assert_eq!(call["data"]["id"], configuration::config_id());
    }
    assert_eq!(calls[0]["data"]["metadata"]["ui_form"], "judge-typesafe");
    assert_eq!(calls[0]["data"]["schema"], JevConfig::json_schema());
    assert_eq!(calls[0]["data"]["initial_value"], candidate.to_json());
}

#[tokio::test]
async fn empty_entry_receives_default_or_explicit_seed_in_one_ensure() {
    for seed in [
        None,
        Some(JevConfig {
            model: "seed-model".into(),
            ..JevConfig::default()
        }),
    ] {
        let candidate = seed.clone().unwrap_or_default();
        let (config, calls) = register_and_fetch(Value::Null, seed, None).await;
        assert_eq!(config.unwrap().to_json(), candidate.to_json());
        assert_ensure_then_fetch(&calls, &candidate);
    }
}

#[tokio::test]
async fn stored_value_is_fetched_instead_of_default_or_explicit_seed() {
    let stored = JevConfig {
        api_key: Some("stored-test-marker".into()),
        model: "operator-model".into(),
        max_timeout_ms: 1234,
        ..JevConfig::default()
    };
    for seed in [
        None,
        Some(JevConfig {
            api_key: Some("seed-test-marker".into()),
            model: "seed-model".into(),
            ..JevConfig::default()
        }),
    ] {
        let candidate = seed.clone().unwrap_or_default();
        let (config, calls) = register_and_fetch(stored.to_json(), seed, None).await;
        assert_eq!(config.unwrap().to_json(), stored.to_json());
        assert_ensure_then_fetch(&calls, &candidate);
    }
}

#[tokio::test]
async fn missing_ensure_fails_closed_with_actionable_error_and_no_legacy_fallback() {
    let (result, calls) = register_and_fetch(
        Value::Null,
        None,
        Some(json!({"code":"function_not_found","message":"remote-test-secret"})),
    )
    .await;
    assert_eq!(result.unwrap_err(), iii_config_client::ENSURE_UNAVAILABLE);
    assert!(!calls.is_empty());
    assert!(calls
        .iter()
        .all(|call| call["function_id"] == "configuration::ensure"));
}

#[tokio::test]
async fn other_ensure_errors_are_redacted_even_when_the_message_mentions_missing_ensure() {
    for code in ["ADAPTER_ERROR", "SCHEMA_INVALID", "NOT_FOUND"] {
        let (result, calls) = register_and_fetch(
            Value::Null,
            None,
            Some(json!({
                "code":code,
                "message":format!("remote-test-secret: remote error (function_not_found): {}", iii_config_client::ENSURE_UNAVAILABLE),
            })),
        ).await;
        assert_eq!(result.unwrap_err(), "JEV configuration registration failed");
        assert!(!calls.is_empty());
        assert!(calls
            .iter()
            .all(|call| call["function_id"] == "configuration::ensure"));
    }
}

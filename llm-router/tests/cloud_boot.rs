//! Cloud boot contract: configuration registration is restricted to ensure/get.
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{SinkExt, StreamExt};
use iii_sdk::{register_worker, InitOptions};
use llm_router::config::entry::register_entry;
use llm_router::config::state::snapshot;
use llm_router::register::register_router;
use serde_json::{json, Value};
use tokio::{net::TcpListener, time::timeout};
use tokio_tungstenite::{accept_async, tungstenite::Message};

#[tokio::test(flavor = "multi_thread")]
async fn router_boots_when_register_is_denied_and_ensure_preserves_configuration() {
    let stored = json!({
        "default_provider": "custom",
        "providers": { "custom": { "region": "us-east-1" } },
        "settings": { "retry_max": 4 }
    });
    let calls = Arc::new(Mutex::new(Vec::new()));
    let calls_for_engine = calls.clone();
    let stored_for_engine = stored.clone();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        socket
            .send(Message::Text(
                json!({ "type": "workerregistered", "worker_id": "cloud-boot-test" })
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
            calls_for_engine.lock().unwrap().push(request.clone());
            let result = match request["function_id"].as_str().unwrap() {
                "state::get" => Err(json!({
                    "code": "function_not_found",
                    "message": "state is intentionally absent"
                })),
                "configuration::get" => Ok(json!({ "value": stored_for_engine })),
                "configuration::ensure" => Ok(json!({
                    "action": "preserved",
                    "entry": { "id": request["data"]["id"], "value": stored_for_engine }
                })),
                "configuration::register" => Err(json!({
                    "code": "FORBIDDEN",
                    "message": "configuration::register is unavailable to application workers"
                })),
                "engine::functions::list" => Ok(json!({ "functions": [] })),
                _ => Ok(json!({})),
            };
            let mut response = json!({
                "type": "invocationresult",
                "invocation_id": request["invocation_id"],
                "function_id": request["function_id"],
            });
            match result {
                Ok(value) => response["result"] = value,
                Err(error) => response["error"] = error,
            }
            socket
                .send(Message::Text(response.to_string().into()))
                .await
                .unwrap();
        }
    });

    let iii = register_worker(&url, InitOptions::default());
    let refs = timeout(Duration::from_secs(10), register_router(iii.clone()))
        .await
        .expect("router boot completes")
        .expect("router boots with ensure/get permissions");
    assert_eq!(snapshot(&refs.config).value(), &stored);

    let mut schemas = BTreeMap::new();
    schemas.insert(
        "custom".into(),
        json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "region": { "type": "string" } }
        }),
    );
    register_entry(&iii, &schemas)
        .await
        .expect("dynamic provider schema refresh uses ensure");

    let calls = calls.lock().unwrap();
    assert!(calls
        .iter()
        .all(|call| call["function_id"] != "configuration::register"));
    let ensure = calls
        .iter()
        .rev()
        .find(|call| call["function_id"] == "configuration::ensure")
        .expect("configuration schema is ensured");
    assert_eq!(
        ensure["data"]["schema"]["properties"]["providers"]["properties"]["custom"]["properties"]
            ["region"]["type"],
        "string"
    );
    assert!(ensure["data"].get("initial_value").is_none());
    drop(calls);

    drop(refs);
    iii.shutdown();
    server.abort();
}

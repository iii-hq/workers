//! Real SDK frames against an isolated protocol fixture: no external engine,
//! host configuration, or CLI process is involved.
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use iii_sdk::{register_worker, InitOptions};
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// Exercise stored, absent and failed lookups with a fresh process-local configuration ID.
#[test]
fn configuration_delivery_preserves_compose_and_publishes_identity() {
    for scenario in ["stored", "missing", "unavailable", "service_error"] {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "configuration_delivery_child",
                "--ignored",
                "--nocapture",
            ])
            .env("III_CONFIG_NAME", "project-console-0123456789abcdef")
            .env("CONFIG_DELIVERY_CASE", scenario)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{scenario}: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

/// Child-process fixture keeps the environment and cached identity isolated per scenario.
#[tokio::test]
#[ignore = "isolated subprocess invoked by the parent"]
async fn configuration_delivery_child() {
    tokio::time::timeout(Duration::from_secs(10), exercise_delivery())
        .await
        .unwrap();
}

/// Drive real SDK frames to verify identity metadata and whether registration includes a seed.
async fn exercise_delivery() {
    let scenario = std::env::var("CONFIG_DELIVERY_CASE").unwrap();
    let id = std::env::var("III_CONFIG_NAME").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let frames = Arc::new(Mutex::new(Vec::<Value>::new()));
    let captured = frames.clone();
    let fixture_id = id.clone();
    let unavailable = scenario == "unavailable";
    // A service failure mentioning NOT_FOUND is not permission to use legacy registration.
    let service_error = scenario == "service_error";
    let stored = (scenario == "stored").then(|| json!({"http_port":3213,"other":"${UNCHANGED}"}));
    let (identity_tx, identity_rx) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(async move {
        let (socket, _) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
        ws.send(Message::Text(
            json!({"type":"workerregistered","worker_id":"fixture"}).to_string(),
        ))
        .await
        .unwrap();
        let mut stored = stored;
        let mut identity_tx = Some(identity_tx);
        let invocation = "00000000-0000-4000-8000-000000000001";
        while let Some(Ok(message)) = ws.next().await {
            let Message::Text(text) = message else {
                continue;
            };
            let frame: Value = serde_json::from_str(&text).unwrap();
            captured.lock().unwrap().push(frame.clone());
            match frame["type"].as_str() {
                Some("registerfunction") if frame["id"] == "console::configuration-id" => {
                    assert_eq!(frame["metadata"]["internal"], true);
                    assert_eq!(frame["request_format"]["type"], "object");
                    assert_eq!(
                        frame["response_format"]["properties"]["id"]["type"],
                        "string"
                    );
                    ws.send(Message::Text(
                        json!({
                            "type":"invokefunction", "function_id":"console::configuration-id",
                            "invocation_id":invocation,
                            "data":{"_caller_worker_id":"00000000-0000-4000-8000-000000000002"},
                            "namespace":"project"
                        })
                        .to_string(),
                    ))
                    .await
                    .unwrap();
                }
                Some("invocationresult") if frame["invocation_id"] == invocation => {
                    identity_tx
                        .take()
                        .unwrap()
                        .send(frame["result"].clone())
                        .unwrap();
                }
                Some("invokefunction") => {
                    let function = frame["function_id"].as_str().unwrap();
                    if !function.starts_with("configuration::") {
                        ws.send(Message::Text(
                            json!({
                                "type":"invocationresult", "function_id":function,
                                "invocation_id":frame["invocation_id"], "result":{}
                            })
                            .to_string(),
                        ))
                        .await
                        .unwrap();
                        continue;
                    }
                    assert_eq!(frame["namespace"], "default");
                    assert_eq!(frame["data"]["id"], fixture_id);
                    let mut reply = json!({"type":"invocationresult", "function_id":function,
                        "invocation_id":frame["invocation_id"]});
                    if function == "configuration::ensure" {
                        if unavailable {
                            reply["error"] =
                                json!({"code":"function_not_found","message":"ensure absent"});
                        } else if service_error {
                            reply["error"] = json!({"code":"OTHER","message":"remote error (NOT_FOUND): mentioned only"});
                        } else {
                            let empty = stored.as_ref().is_none_or(Value::is_null);
                            if empty {
                                stored = Some(frame["data"]["initial_value"].clone());
                            }
                            reply["result"] = json!({
                                "action": if empty { "seeded" } else { "preserved" },
                                "entry": { "id":fixture_id,"value":stored }
                            });
                        }
                    } else if function == "configuration::get" {
                        if let Some(value) = &stored {
                            reply["result"] = json!({"id":fixture_id,"value":value});
                        } else {
                            reply["error"] = json!({"code":"NOT_FOUND","message":"entry absent"});
                        }
                    } else {
                        panic!("unexpected RPC {function}; no legacy registration is allowed");
                    }
                    ws.send(Message::Text(reply.to_string())).await.unwrap();
                }
                Some("ping") => {
                    ws.send(Message::Text(json!({"type":"pong"}).to_string()))
                        .await
                        .unwrap();
                }
                _ => {}
            }
        }
    });
    let iii = register_worker(
        &url,
        InitOptions {
            namespace: Some("project".into()),
            ..Default::default()
        },
    );
    iii.wait_until_registered(Duration::from_secs(3))
        .await
        .unwrap();
    let result = ade::configuration::register_console_config(&iii, 3113).await;
    assert_eq!(identity_rx.await.unwrap(), json!({"id":id}));
    if unavailable || service_error {
        let error = result.expect_err("failed ensure must not become successful initialization");
        if unavailable {
            assert!(error.contains("upgrade engine"), "{error}");
        } else {
            assert!(error.contains("OTHER"), "{error}");
        }
        assert!(!frames
            .lock()
            .unwrap()
            .iter()
            .any(|f| f["function_id"] == "configuration::register"));
    } else {
        result.unwrap();
        let runtime = ade::configuration::fetch_runtime_config(&iii, 3113)
            .await
            .unwrap();
        let expected = if scenario == "stored" { 3213 } else { 3113 };
        assert_eq!(runtime.http_port, expected);
        let snapshot = frames.lock().unwrap();
        let registration = snapshot
            .iter()
            .find(|f| f["type"] == "invokefunction" && f["function_id"] == "configuration::ensure")
            .unwrap();
        assert_eq!(registration["data"]["metadata"]["ui_form"], "console");
        assert_eq!(registration["data"]["initial_value"]["http_port"], 3113);
        assert!(!snapshot
            .iter()
            .any(|f| f["function_id"] == "configuration::register"));
    }
    iii.shutdown_async().await;
    server.abort();
}

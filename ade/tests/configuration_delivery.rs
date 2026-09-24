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
    for scenario in [
        "stored",
        "missing",
        "legacy_stored",
        "legacy_missing",
        "legacy_null",
        "legacy_false",
        "legacy_zero",
        "legacy_empty",
        "legacy_malformed",
        "legacy_get_error",
        "legacy_register_error",
        "legacy_workspace",
        "service_error",
        "adapter_error",
        "schema_error",
    ] {
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
    // An entry written by a Console that still kept the tab layout in the
    // configuration entry (a legacy *entry* on a modern engine, not a legacy
    // engine). The only scenario in which a `configuration::set` is
    // legitimate: the one that lifts `workspace` out.
    let legacy_workspace = scenario == "legacy_workspace";
    // Every other `legacy_*` scenario runs against an engine that lacks
    // `configuration::ensure` and must fall back to get/register.
    let unavailable = scenario.starts_with("legacy_") && !legacy_workspace;
    let fixture_scenario = scenario.clone();
    // A service failure mentioning NOT_FOUND is not permission to use legacy registration.
    let service_error = scenario == "service_error";
    let legacy_layout = json!({
        "tabs": [{ "id": "tab-home", "columns": 2, "screens": ["chat", "traces"] }],
        "activeTabId": "tab-home"
    });
    let stored = match scenario.as_str() {
        "stored" | "legacy_stored" => Some(json!({"http_port":3213,"other":"${UNCHANGED}"})),
        "legacy_workspace" => Some(json!({
            "http_port":3213,
            "other":"${UNCHANGED}",
            "workspace": legacy_layout.clone()
        })),
        "legacy_null" => Some(Value::Null),
        "legacy_false" => Some(json!(false)),
        "legacy_zero" => Some(json!(0)),
        "legacy_empty" => Some(json!("")),
        _ => None,
    };
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
        let mut ensure_seen = false;
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
                        ensure_seen = true;
                        if unavailable {
                            reply["error"] =
                                json!({"code":"function_not_found","message":"ensure absent"});
                        } else if fixture_scenario == "adapter_error"
                            || fixture_scenario == "schema_error"
                        {
                            reply["error"] = json!({"code": if fixture_scenario == "adapter_error" { "ADAPTER_ERROR" } else { "SCHEMA_INVALID" }, "message":"function_not_found"});
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
                        if ensure_seen
                            && frame["data"]["raw"] == true
                            && fixture_scenario == "legacy_malformed"
                        {
                            reply["result"] = json!({"id":fixture_id});
                        } else if ensure_seen
                            && frame["data"]["raw"] == true
                            && fixture_scenario == "legacy_get_error"
                        {
                            reply["error"] =
                                json!({"code":"function_not_found", "message":"get absent"});
                        } else if let Some(value) = &stored {
                            reply["result"] = json!({"id":fixture_id,"value":value});
                        } else {
                            reply["error"] = json!({"code":"NOT_FOUND","message":"entry absent"});
                        }
                    } else if function == "configuration::register" {
                        assert!(unavailable, "modern path must not register");
                        if fixture_scenario == "legacy_register_error" {
                            reply["error"] =
                                json!({"code":"SCHEMA_INVALID", "message":"register rejected"});
                        } else {
                            if let Some(seed) = frame["data"].get("initial_value") {
                                stored = Some(seed.clone());
                            }
                            reply["result"] = json!({"id":fixture_id,"value":stored});
                        }
                    } else if function == "configuration::set" && legacy_workspace {
                        stored = Some(frame["data"]["value"].clone());
                        reply["result"] = json!({"id":fixture_id,"value":stored});
                    } else {
                        panic!("unexpected RPC {function}");
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
    let result = ade::configuration::register_console_config(&iii, 3113, "data/ade").await;
    assert_eq!(identity_rx.await.unwrap(), json!({"id":id}));
    let failed = service_error || scenario.ends_with("error") || scenario == "legacy_malformed";
    if failed {
        result.expect_err("failed initialization must propagate");
        if scenario != "legacy_register_error" {
            assert!(!frames
                .lock()
                .unwrap()
                .iter()
                .any(|f| f["function_id"] == "configuration::register"));
        }
    } else {
        result.unwrap();
        let snapshot = frames.lock().unwrap().clone();
        let operations: Vec<_> = snapshot
            .iter()
            .filter(|f| {
                f["type"] == "invokefunction"
                    && f["function_id"]
                        .as_str()
                        .is_some_and(|id| id.starts_with("configuration::"))
            })
            .collect();
        let ensure_index = operations
            .iter()
            .position(|f| f["function_id"] == "configuration::ensure")
            .unwrap();
        let initialization = &operations[ensure_index..];
        // ADE's intentional pre-existing port migration read precedes this helper.
        assert_eq!(
            initialization
                .iter()
                .map(|f| f["function_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            if unavailable {
                vec![
                    "configuration::ensure",
                    "configuration::get",
                    "configuration::register",
                ]
            } else {
                vec!["configuration::ensure"]
            }
        );
        let candidate = &initialization[0]["data"];
        assert_eq!(candidate["metadata"]["ui_form"], "console");
        assert_eq!(candidate["initial_value"]["http_port"], 3113);
        assert_eq!(candidate["initial_value"]["data_dir"], "data/ade");
        assert!(candidate["initial_value"].get("workspace").is_none());
        assert_eq!(
            candidate["schema"]["properties"]["data_dir"]["type"],
            "string"
        );
        if unavailable {
            assert_eq!(initialization[1]["data"]["raw"], true);
            let mut expected = candidate.clone();
            if !matches!(scenario.as_str(), "legacy_missing" | "legacy_null") {
                expected.as_object_mut().unwrap().remove("initial_value");
            }
            assert_eq!(initialization[2]["data"], expected);
        } else {
            assert!(!snapshot
                .iter()
                .any(|f| f["function_id"] == "configuration::register"));
        }
        if matches!(
            scenario.as_str(),
            "stored"
                | "missing"
                | "legacy_stored"
                | "legacy_missing"
                | "legacy_null"
                | "legacy_workspace"
        ) {
            let runtime = ade::configuration::fetch_runtime_config(&iii, 3113, "data/ade")
                .await
                .unwrap();
            assert_eq!(
                runtime.http_port,
                if scenario.ends_with("stored") || legacy_workspace {
                    3213
                } else {
                    3113
                }
            );
            // A stored entry that predates `data_dir` keeps the seed; no backfill
            // write is issued for it (the fixture panics on `configuration::set`).
            assert_eq!(runtime.data_dir, "data/ade");
        }

        let set_frames = || {
            frames
                .lock()
                .unwrap()
                .iter()
                .filter(|f| f["function_id"] == "configuration::set")
                .map(|f| f["data"]["value"].clone())
                .collect::<Vec<Value>>()
        };
        if legacy_workspace {
            let dir = std::env::temp_dir().join(format!(
                "ade-legacy-workspace-{}-{}",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            let store = ade::workspace_store::WorkspaceStore::new(dir.clone());
            let moved = ade::configuration::migrate_legacy_workspace(&iii, &store)
                .await
                .unwrap();
            assert!(moved, "the legacy section must be lifted out");
            // The layout landed in `<data_dir>/workspace.json`, flat.
            assert_eq!(store.load().await.unwrap(), Some(legacy_layout.clone()));
            // The entry lost ONLY `workspace`; siblings (including env
            // templates) are written back verbatim.
            assert_eq!(
                set_frames(),
                vec![json!({"http_port":3213,"other":"${UNCHANGED}"})],
                "exactly one rewrite of the entry"
            );
            // Idempotent: a second boot finds nothing to move and issues no write.
            let again = ade::configuration::migrate_legacy_workspace(&iii, &store)
                .await
                .unwrap();
            assert!(!again);
            assert_eq!(set_frames().len(), 1);
            let _ = std::fs::remove_dir_all(&dir);
        } else {
            assert!(set_frames().is_empty());
        }
    }
    iii.shutdown_async().await;
    server.abort();
}

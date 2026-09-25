//! Exercise the real binary (tiny local GGUF) against an isolated configuration/engine websocket.
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::{net::TcpListener, sync::mpsc, time::timeout};
use tokio_tungstenite::{accept_async, tungstenite::Message};
struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn boot_registers_config_schemas_reload_and_optional_console_assets() {
    check_boot(None).await;
}

#[cfg(unix)]
#[tokio::test]
async fn sigint_waits_for_sdk_shutdown_before_exiting() {
    check_boot(Some("-INT")).await;
}

#[cfg(unix)]
#[tokio::test]
async fn sigterm_waits_for_sdk_shutdown_before_exiting() {
    check_boot(Some("-TERM")).await;
}

async fn check_boot(shutdown_signal: Option<&str>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}", listener.local_addr().unwrap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        socket
            .send(Message::Text(
                json!({"type":"workerregistered","worker_id":"boot-test"})
                    .to_string()
                    .into(),
            ))
            .await
            .unwrap();
        let mut stored = Value::Null;
        while let Some(Ok(frame)) = socket.next().await {
            let Message::Text(text) = frame else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).unwrap();
            tx.send(value.clone()).unwrap();
            if value["type"] == "invokefunction" {
                let result = match value["function_id"].as_str().unwrap() {
                    "configuration::get" => json!({"value":stored}),
                    "configuration::ensure" => {
                        let action = if stored.is_null() {
                            stored = value["data"]["initial_value"].clone();
                            "seeded"
                        } else {
                            "preserved"
                        };
                        json!({"action":action,"entry":{"id":value["data"]["id"],"value":stored}})
                    }
                    _ => json!({}),
                };
                if value["invocation_id"].is_string() {
                    socket.send(Message::Text(json!({"type":"invocationresult","invocation_id":value["invocation_id"],"function_id":value["function_id"],"result":result}).to_string().into())).await.unwrap();
                }
            }
            if value["type"] == "registerfunction" && value["id"] == "judge-decider::evaluate" {
                socket.send(Message::Text(json!({"type":"invokefunction","invocation_id":"00000000-0000-0000-0000-000000000009","function_id":"judge-decider::evaluate","data":{"timeout_ms":1000,"evaluations":[{"id":"ticket","state":{},"questions":{"urgent":{"type":"noul","instructions":"Is this urgent?"}}}]}}).to_string().into())).await.unwrap();
            }
            if value["type"] == "registerfunction"
                && value["id"] == "judge-decider::configuration-id"
            {
                socket.send(Message::Text(json!({"type":"invokefunction","invocation_id":"00000000-0000-0000-0000-000000000010","function_id":"judge-decider::configuration-id","data":{"_caller_worker_id":"console-test"}}).to_string().into())).await.unwrap();
            }
        }
    });
    let mut worker = Worker(
        Command::new(env!("CARGO_BIN_EXE_judge-decider"))
            .env("III_URL", url)
            .env("III_CONFIG_NAME", "decider-boot-test")
            .env(
                "III_DECIDER_GGUF",
                concat!(
                    env!("CARGO_MANIFEST_DIR"),
                    "/tests/fixtures/tiny-qwen3.gguf"
                ),
            )
            .env_remove("III_DECIDER_UI_WATCH")
            .env("RUST_LOG", "info")
            .env("OTEL_ENABLED", "true")
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap(),
    );
    let stdout = worker.0.stdout.take().unwrap();
    let (log_tx, mut logs) = mpsc::unbounded_channel();
    let log_reader = std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if log_tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let console_ui_enabled = cfg!(feature = "console-ui");
    timeout(Duration::from_secs(10), async {
        let mut expected_functions = BTreeSet::from([
            "judge-decider::evaluate".to_owned(),
            "judge-decider::models::list".to_owned(),
            "judge-decider::cancel".to_owned(),
            "judge-decider::on-config-change".to_owned(),
        ]);
        if console_ui_enabled {
            expected_functions.insert("judge-decider::ui-content".to_owned());
            expected_functions.insert("judge-decider::configuration-id".to_owned());
        }
        let mut functions = BTreeSet::new();
        let mut identity = !console_ui_enabled;
        let (mut config, mut reload, mut script, mut style, mut evaluated) = (
            false,
            false,
            !console_ui_enabled,
            !console_ui_enabled,
            false,
        );
        while !(config
            && functions == expected_functions
            && reload
            && script
            && style
            && identity
            && evaluated)
        {
            let value = rx.recv().await.expect("worker stays connected");
            match value["type"].as_str().unwrap() {
                "invokefunction" if value["function_id"] == "configuration::ensure" => {
                    assert_eq!(value["namespace"], "default");
                    assert_eq!(value["data"]["id"], "decider-boot-test");
                    assert_eq!(value["data"]["metadata"]["ui_form"], "judge-decider");
                    assert_eq!(value["data"]["initial_value"]["model"], "decider-4b-v2");
                    config = true;
                }
                "invokefunction" if value["function_id"] == "configuration::register" => {
                    panic!("boot must never fall back to legacy configuration::register");
                }
                "registerfunction" => {
                    let id = value["id"].as_str().unwrap();
                    assert!(expected_functions.contains(id), "unexpected function {id}");
                    for field in ["request_format", "response_format"] {
                        let schema = value[field].as_object().expect("schema is an object");
                        assert!(
                            [
                                "type",
                                "$ref",
                                "oneOf",
                                "anyOf",
                                "allOf",
                                "enum",
                                "properties"
                            ]
                            .iter()
                            .any(|key| schema.contains_key(*key)),
                            "{id}.{field} must be typed"
                        );
                    }
                    if matches!(
                        id,
                        "judge-decider::on-config-change"
                            | "judge-decider::ui-content"
                            | "judge-decider::configuration-id"
                    ) {
                        assert_eq!(value["metadata"]["internal"], true);
                    }
                    functions.insert(id.to_owned());
                }
                "registertrigger" if value["trigger_type"] == "configuration" => {
                    assert_eq!(value["function_id"], "judge-decider::on-config-change");
                    assert_eq!(
                        value["config"],
                        json!({
                            "configuration_id": "decider-boot-test",
                            "event_types": ["configuration:updated"]
                        })
                    );
                    reload = true;
                }
                "registertrigger" if value["trigger_type"] == "console:script" => {
                    assert!(console_ui_enabled);
                    assert_eq!(value["function_id"], "judge-decider::ui-content");
                    assert_eq!(value["config"]["path"], "judge-decider/page.js");
                    script = true;
                }
                "registertrigger" if value["trigger_type"] == "console:style" => {
                    assert!(console_ui_enabled);
                    assert_eq!(value["function_id"], "judge-decider::ui-content");
                    assert_eq!(value["config"]["path"], "judge-decider/styles.css");
                    style = true;
                }
                "invocationresult" if value["function_id"] == "judge-decider::evaluate" => {
                    assert_eq!(value["result"]["status"], "ok", "{value}");
                    assert_eq!(value["result"]["model"], "decider-4b-v2");
                    assert!(
                        value["result"]["results"]["ticket"]["answers"]["urgent"]["noul"].is_f64()
                    );
                    assert!(value.get("error").is_none());
                    evaluated = true;
                }
                "invocationresult" if value["function_id"] == "judge-decider::configuration-id" => {
                    assert!(console_ui_enabled);
                    assert_eq!(value["result"], json!({"id":"decider-boot-test"}));
                    assert!(value.get("error").is_none());
                    identity = true;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("boot is ready with the local GGUF");

    if let Some(signal) = shutdown_signal {
        timeout(Duration::from_secs(10), async {
            while let Some(line) = logs.recv().await {
                if line.contains("judge-decider ready") {
                    return;
                }
            }
            panic!("worker exited before reporting readiness");
        })
        .await
        .expect("worker reports readiness");
        assert!(Command::new("kill")
            .args([signal, &worker.0.id().to_string()])
            .status()
            .unwrap()
            .success());
        // Exit status alone does not prove the SDK's background thread finished.
        // This record comes from its final telemetry shutdown, after flushing.
        let flushed = timeout(Duration::from_secs(20), async {
            let mut flushed = false;
            while let Some(line) = logs.recv().await {
                flushed |= line.contains("OpenTelemetry shut down");
            }
            flushed
        })
        .await
        .expect("worker exits after bounded SDK cleanup");
        assert!(worker.0.wait().unwrap().success(), "clean exit on {signal}");
        assert!(
            flushed,
            "{signal}: worker exited before SDK cleanup finished"
        );
    }
    drop(worker);
    log_reader.join().unwrap();
    server.abort();
}

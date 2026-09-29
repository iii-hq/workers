//! Exercise the real hub binary against an isolated configuration/engine websocket.
#[path = "../../judge-typesafe/tests/support/fake_engine.rs"]
mod fake_engine;

use serde_json::{json, Value};
use std::{
    collections::BTreeSet,
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio::{sync::mpsc, time::timeout};

struct Worker(Child);
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn boot_seeds_the_provider_registers_schemas_reload_and_console_assets() {
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
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut stored = Value::Null;
    let engine = fake_engine::start(move |value| {
        tx.send(value.clone()).unwrap();
        let mut replies = Vec::new();
        if value["type"] == "invokefunction" && value["invocation_id"].is_string() {
            let function_id = value["function_id"].as_str().unwrap();
            let mut reply = json!({"type":"invocationresult","invocation_id":value["invocation_id"],"function_id":function_id});
            match function_id {
                "configuration::get" => reply["result"] = json!({"value":stored}),
                "configuration::ensure" => {
                    let action = if stored.is_null() {
                        stored = value["data"]["initial_value"].clone();
                        "seeded"
                    } else {
                        "preserved"
                    };
                    reply["result"] = json!({"action":action,"entry":{"id":value["data"]["id"],"value":stored}});
                }
                // The seeded provider is not running: the hub must answer typed.
                id if id.starts_with("judge-") => {
                    reply["error"] = json!({"code":"function_not_found","message":"no such function"});
                }
                _ => reply["result"] = json!({}),
            }
            replies.push(reply);
        }
        if value["type"] == "registerfunction" && value["id"] == "judge::evaluate" {
            replies.push(json!({"type":"invokefunction","invocation_id":"00000000-0000-0000-0000-000000000009","function_id":"judge::evaluate","data":{"_caller_worker_id":"console-test","timeout_ms":1000,"evaluations":[{"id":"ticket","state":{},"questions":{"urgent":{"type":"noul","instructions":"Is this urgent?"}}}]}}));
        }
        if value["type"] == "registerfunction" && value["id"] == "judge::configuration-id" {
            replies.push(json!({"type":"invokefunction","invocation_id":"00000000-0000-0000-0000-000000000010","function_id":"judge::configuration-id","data":{"_caller_worker_id":"console-test"}}));
        }
        replies
    })
    .await;
    let mut worker = Worker(
        Command::new(env!("CARGO_BIN_EXE_judge"))
            .env("III_URL", &engine.url)
            .env("III_CONFIG_NAME", "judge-boot-test")
            .env("JUDGE_PROVIDER", "local-llm")
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
            "judge::evaluate".to_owned(),
            "judge::models::list".to_owned(),
            "judge::cancel".to_owned(),
            "judge::on-config-change".to_owned(),
        ]);
        if console_ui_enabled {
            expected_functions.insert("judge::ui-content".to_owned());
            expected_functions.insert("judge::configuration-id".to_owned());
        }
        let mut functions = BTreeSet::new();
        let mut identity = !console_ui_enabled;
        let (mut config, mut reload, mut script, mut style, mut forwarded, mut unavailable) = (
            false,
            false,
            !console_ui_enabled,
            !console_ui_enabled,
            false,
            false,
        );
        while !(config
            && functions == expected_functions
            && reload
            && script
            && style
            && identity
            && forwarded
            && unavailable)
        {
            let value = rx.recv().await.expect("worker stays connected");
            match value["type"].as_str().unwrap() {
                "invokefunction" if value["function_id"] == "configuration::ensure" => {
                    assert_eq!(value["namespace"], "default");
                    assert_eq!(value["data"]["id"], "judge-boot-test");
                    assert_eq!(value["data"]["metadata"]["ui_form"], "judge");
                    assert_eq!(
                        value["data"]["initial_value"],
                        json!({"provider": "local-llm"})
                    );
                    config = true;
                }
                "invokefunction" if value["function_id"] == "configuration::register" => {
                    panic!("boot must never fall back to legacy configuration::register");
                }
                "invokefunction" if value["function_id"] == "judge-local-llm::evaluate" => {
                    // Forwarded with the caller-scoped id, without the engine metadata.
                    assert!(value["data"].get("_caller_worker_id").is_none());
                    assert!(value["data"].get("provider").is_none());
                    assert_eq!(value["data"]["timeout_ms"], 1000);
                    forwarded = true;
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
                    // The hub's own surface is what callers discover; plumbing is internal.
                    let internal = matches!(
                        id,
                        "judge::on-config-change"
                            | "judge::ui-content"
                            | "judge::configuration-id"
                            | "judge::models::list"
                    );
                    assert_eq!(
                        value["metadata"]["internal"].as_bool().unwrap_or(false),
                        internal,
                        "{id} internal flag"
                    );
                    if id == "judge::evaluate" {
                        assert_eq!(
                            value["request_format"]["properties"]["provider"]["pattern"],
                            "^[a-z0-9-]{1,64}$"
                        );
                    }
                    functions.insert(id.to_owned());
                }
                "registertrigger" if value["trigger_type"] == "configuration" => {
                    assert_eq!(value["function_id"], "judge::on-config-change");
                    assert_eq!(
                        value["config"],
                        json!({
                            "configuration_id": "judge-boot-test",
                            "event_types": ["configuration:updated"]
                        })
                    );
                    reload = true;
                }
                "registertrigger" if value["trigger_type"] == "console:script" => {
                    assert!(console_ui_enabled);
                    assert_eq!(value["function_id"], "judge::ui-content");
                    assert_eq!(value["config"]["path"], "judge/page.js");
                    script = true;
                }
                "registertrigger" if value["trigger_type"] == "console:style" => {
                    assert!(console_ui_enabled);
                    assert_eq!(value["function_id"], "judge::ui-content");
                    assert_eq!(value["config"]["path"], "judge/styles.css");
                    style = true;
                }
                "invocationresult" if value["function_id"] == "judge::evaluate" => {
                    assert_eq!(value["result"]["code"], "provider_unavailable");
                    assert!(value.get("error").is_none());
                    unavailable = true;
                }
                "invocationresult" if value["function_id"] == "judge::configuration-id" => {
                    assert!(console_ui_enabled);
                    assert_eq!(value["result"], json!({"id":"judge-boot-test"}));
                    assert!(value.get("error").is_none());
                    identity = true;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("boot is ready without a running provider");

    if let Some(signal) = shutdown_signal {
        timeout(Duration::from_secs(10), async {
            while let Some(line) = logs.recv().await {
                if line.contains("judge hub ready") {
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
    drop(engine);
}

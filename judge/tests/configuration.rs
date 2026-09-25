//! Verify the hub's configuration RPC contract against a mock service.
#[path = "../../judge-typesafe/tests/support/fake_engine.rs"]
mod fake_engine;

use iii_sdk::{register_worker, InitOptions};
use judge::{configuration, JudgeConfig};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::{sync::mpsc, time::timeout};

async fn register_and_fetch(
    mut stored: Value,
    seed: Option<JudgeConfig>,
    ensure_error: Option<Value>,
) -> (Result<JudgeConfig, String>, Vec<Value>) {
    let (tx, mut requests) = mpsc::unbounded_channel();
    let engine = fake_engine::start(move |request| {
        if request["type"] != "invokefunction"
            || !request["function_id"]
                .as_str()
                .unwrap()
                .starts_with("configuration::")
        {
            return vec![];
        }
        tx.send(request.clone()).unwrap();
        let result = match request["function_id"].as_str().unwrap() {
            "configuration::ensure" => match &ensure_error {
                Some(error) => Err(error.clone()),
                None => {
                    if stored.is_null() {
                        stored = request["data"]["initial_value"].clone();
                    }
                    Ok(json!({"entry":{"id":request["data"]["id"],"value":stored}}))
                }
            },
            "configuration::get" => Ok(json!({"value":stored})),
            "configuration::register" => {
                if stored.is_null() {
                    stored = request["data"]["initial_value"].clone();
                }
                Ok(json!({"id":request["data"]["id"]}))
            }
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
        vec![response]
    })
    .await;
    let iii = register_worker(&engine.url, InitOptions::default());
    let result = timeout(Duration::from_secs(10), async {
        configuration::register_config(&iii, seed.as_ref()).await?;
        configuration::fetch_config(&iii).await
    })
    .await;
    tokio::task::spawn_blocking(move || iii.shutdown())
        .await
        .unwrap();
    let mut calls = Vec::new();
    while let Ok(request) = requests.try_recv() {
        calls.push(request);
    }
    (result.expect("configuration RPCs finish"), calls)
}

fn ids(calls: &[Value]) -> Vec<&str> {
    calls
        .iter()
        .map(|call| call["function_id"].as_str().unwrap())
        .collect()
}

#[tokio::test]
async fn empty_entry_receives_the_seed_and_stored_values_win_afterwards() {
    let seed = JudgeConfig {
        provider: "local-llm".into(),
        preload_all: false,
    };
    let (config, calls) = register_and_fetch(Value::Null, Some(seed.clone()), None).await;
    assert_eq!(config.unwrap().provider, "local-llm");
    assert_eq!(ids(&calls), ["configuration::ensure", "configuration::get"]);
    assert_eq!(calls[0]["data"]["id"], configuration::config_id());
    assert_eq!(calls[0]["data"]["metadata"]["ui_form"], "judge");
    assert_eq!(calls[0]["data"]["schema"], JudgeConfig::json_schema());
    assert_eq!(calls[0]["data"]["initial_value"], seed.to_json());

    let stored = json!({"provider": "operator-choice"});
    let (config, _) = register_and_fetch(stored, Some(seed), None).await;
    assert_eq!(config.unwrap().provider, "operator-choice");
    let (config, _) = register_and_fetch(Value::Null, None, None).await;
    assert_eq!(config.unwrap().provider, judge::DEFAULT_PROVIDER);
}

#[tokio::test]
async fn missing_ensure_falls_back_to_legacy_get_then_register_seeding() {
    let (config, calls) = register_and_fetch(
        Value::Null,
        None,
        Some(json!({"code":"function_not_found","message":"remote-test-secret"})),
    )
    .await;
    assert_eq!(config.unwrap().provider, judge::DEFAULT_PROVIDER);
    assert_eq!(
        ids(&calls),
        [
            "configuration::ensure",
            "configuration::get",
            "configuration::register",
            "configuration::get"
        ]
    );
    assert_eq!(
        calls[2]["data"]["initial_value"],
        JudgeConfig::default().to_json()
    );
}

#[tokio::test]
async fn other_ensure_errors_are_redacted_and_invalid_stored_values_fail_closed() {
    let (result, _) = register_and_fetch(
        Value::Null,
        None,
        Some(json!({"code":"ADAPTER_ERROR","message":"remote-test-secret"})),
    )
    .await;
    let message = result.unwrap_err();
    assert!(!message.contains("remote-test-secret"));
    assert_eq!(message, "judge configuration registration failed");
    let (result, _) = register_and_fetch(json!({"provider": "Not A Worker"}), None, None).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn reload_applies_valid_values_and_keeps_the_last_valid_snapshot() {
    let cell = configuration::new_cell(JudgeConfig::default());
    assert!(
        configuration::apply_config(
            &cell,
            JudgeConfig {
                provider: "local-llm".into(),
                preload_all: false,
            }
        )
        .await
    );
    assert!(
        !configuration::apply_config(
            &cell,
            JudgeConfig {
                provider: "".into(),
                preload_all: false,
            }
        )
        .await
    );
    assert_eq!(cell.read().await.provider, "local-llm");
}

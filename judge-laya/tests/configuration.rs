//! Verify laya's configuration RPC contract against a mock service.
//! The mock implements ensure's documented preservation rule; these tests do
//! not exercise engine persistence, serialization, or atomicity.
#[path = "../../judge-typesafe/tests/support/fake_engine.rs"]
mod fake_engine;
use iii_sdk::{register_worker, InitOptions};
use judge_laya::{configuration, LayaConfig};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::{sync::mpsc, time::timeout};

async fn register_and_fetch(
    mut stored: Value,
    seed: Option<LayaConfig>,
    ensure_error: Option<Value>,
) -> (Result<LayaConfig, String>, Vec<Value>) {
    let (tx, mut requests) = mpsc::unbounded_channel();
    // The mock implements ensure's preservation rule and the legacy register path.
    let engine = fake_engine::start(move |request| {
        // The SDK also emits telemetry RPCs, unrelated to this contract.
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
                    let action = if stored.is_null() {
                        stored = request["data"]["initial_value"].clone();
                        "seeded"
                    } else {
                        "preserved"
                    };
                    Ok(json!({"action":action,"entry":{"id":request["data"]["id"],"value":stored}}))
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

fn assert_ensure_then_fetch(calls: &[Value], candidate: &LayaConfig) {
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["function_id"], "configuration::ensure");
    assert_eq!(calls[1]["function_id"], "configuration::get");
    for call in calls {
        assert_eq!(call["namespace"], "default");
        assert_eq!(call["data"]["id"], configuration::config_id());
    }
    assert_eq!(calls[0]["data"]["metadata"]["ui_form"], "judge-laya");
    assert_eq!(calls[0]["data"]["schema"], LayaConfig::json_schema());
    assert_eq!(calls[0]["data"]["initial_value"], candidate.to_json());
}

#[tokio::test]
async fn empty_entry_receives_default_or_explicit_seed_in_one_ensure() {
    for seed in [
        None,
        Some(LayaConfig {
            batch_questions: 3,
            ..LayaConfig::default()
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
    let stored = LayaConfig {
        model: "laya-multilingual".into(),
        max_timeout_ms: 1234,
        ..LayaConfig::default()
    };
    for seed in [
        None,
        Some(LayaConfig {
            batch_questions: 3,
            ..LayaConfig::default()
        }),
    ] {
        let candidate = seed.clone().unwrap_or_default();
        let (config, calls) = register_and_fetch(stored.to_json(), seed, None).await;
        assert_eq!(config.unwrap().to_json(), stored.to_json());
        assert_ensure_then_fetch(&calls, &candidate);
    }
}

#[tokio::test]
async fn missing_ensure_falls_back_to_legacy_get_then_register_seeding() {
    let (config, calls) = register_and_fetch(
        Value::Null,
        None,
        Some(json!({"code":"function_not_found","message":"remote-test-secret"})),
    )
    .await;
    assert_eq!(config.unwrap().to_json(), LayaConfig::default().to_json());
    let ids: Vec<_> = calls
        .iter()
        .map(|call| call["function_id"].as_str().unwrap())
        .collect();
    assert_eq!(
        ids,
        [
            "configuration::ensure",
            "configuration::get",
            "configuration::register",
            "configuration::get"
        ]
    );
    assert_eq!(calls[1]["data"]["raw"], true);
    assert_eq!(
        calls[2]["data"]["initial_value"],
        LayaConfig::default().to_json()
    );
    assert_eq!(calls[2]["data"]["metadata"]["ui_form"], "judge-laya");
}

#[tokio::test]
async fn other_ensure_errors_are_redacted_even_when_the_message_mentions_missing_ensure() {
    for code in ["ADAPTER_ERROR", "SCHEMA_INVALID", "NOT_FOUND"] {
        let (result, calls) = register_and_fetch(
            Value::Null,
            None,
            Some(json!({
                "code":code,
                "message":"remote-test-secret: remote error (function_not_found): configuration::ensure unavailable",
            })),
        ).await;
        assert_eq!(
            result.unwrap_err(),
            "laya configuration registration failed"
        );
        assert!(!calls.is_empty());
        assert!(calls
            .iter()
            .all(|call| call["function_id"] == "configuration::ensure"));
    }
}

use super::initialization::*;
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

fn payload() -> Value {
    json!({"id":"compat-contract", "name":"Name", "description":"Description",
        "schema":{"type":"object"}, "metadata":{"ui_form":"family"},
        "initial_value":{"seed":"${UNCHANGED:default}"}})
}

async fn run_case(
    ensure_error: Option<&str>,
    lookup: Result<Value, String>,
    registration: Result<Value, String>,
) -> (Result<(), String>, Vec<(&'static str, Value)>) {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let recorded = calls.clone();
    let result = ensure_with(payload(), |function, payload| {
        recorded.lock().unwrap().push((function, payload));
        let response = match function {
            "configuration::ensure" => ensure_error.map_or(Ok(json!({})), |e| Err(e.into())),
            "configuration::get" => lookup.clone(),
            "configuration::register" => registration.clone(),
            _ => panic!("unexpected operation"),
        };
        std::future::ready(response)
    })
    .await;
    let snapshot = calls.lock().unwrap().clone();
    (result, snapshot)
}

#[tokio::test]
async fn modern_path_forwards_candidate_and_metadata_without_preliminary_read() {
    let (result, calls) = run_case(
        None,
        Err("must not read".into()),
        Err("must not register".into()),
    )
    .await;
    result.unwrap();
    assert_eq!(calls, vec![("configuration::ensure", payload())]);
}

#[tokio::test]
async fn legacy_preserves_every_non_null_value_without_copying_it() {
    for value in [
        json!(false),
        json!(0),
        json!(""),
        json!({}),
        json!({"template":"${UNCHANGED}"}),
    ] {
        let (result, calls) = run_case(
            Some("remote error (function_not_found): ensure absent"),
            Ok(json!({"value":value})),
            Ok(json!({})),
        )
        .await;
        result.unwrap();
        let mut expected = payload();
        expected.as_object_mut().unwrap().remove("initial_value");
        assert_eq!(
            calls,
            vec![
                ("configuration::ensure", payload()),
                (
                    "configuration::get",
                    json!({"id":"compat-contract", "raw":true})
                ),
                ("configuration::register", expected),
            ]
        );
    }
}

#[tokio::test]
async fn legacy_seeds_only_null_or_exact_missing_entry() {
    for lookup in [
        Ok(json!({"value":null})),
        Err("NOT_FOUND".into()),
        Err("remote error (NOT_FOUND): entry absent".into()),
        Err(
            "configuration::get failed after 3 attempts: remote error (NOT_FOUND): entry absent"
                .into(),
        ),
    ] {
        let (result, calls) = run_case(Some("configuration::ensure failed after 3 attempts: remote error (function_not_found): ensure absent"),
            lookup, Ok(json!({}))).await;
        result.unwrap();
        assert_eq!(calls.len(), 3);
        assert_eq!(calls[2], ("configuration::register", payload()));
    }
}

#[tokio::test]
async fn malformed_legacy_response_never_writes() {
    for response in [
        json!({}),
        json!([]),
        json!(null),
        json!(false),
        json!({"entry":{"value":null}}),
    ] {
        let (result, calls) =
            run_case(Some("function_not_found"), Ok(response), Ok(json!({}))).await;
        assert_eq!(
            result.unwrap_err(),
            "configuration::get returned no `value` field"
        );
        assert_eq!(calls.len(), 2);
    }
}

#[tokio::test]
async fn ensure_failure_classification_is_anchored_and_contextual() {
    for error in [
        "timeout",
        "connection lost",
        "NOT_FOUND",
        "remote error (NOT_FOUND): absent",
        "remote error (SCHEMA_INVALID): function_not_found",
        "remote error (ADAPTER_ERROR): function_not_found",
        "remote error (FORBIDDEN): function_not_found",
        "remote error (OTHER): remote error (function_not_found): buried",
        "remote error (RESOURCE_NOT_FOUND): absent",
        "remote error (FUNCTION_NOT_FOUND): absent",
        "configuration::get failed after 3 attempts: remote error (function_not_found): absent",
        "configuration::ensure failed after 2 attempts: remote error (function_not_found): absent",
        "remote error (function_not_found):suffix",
    ] {
        let (result, calls) = run_case(Some(error), Ok(json!({"value":null})), Ok(json!({}))).await;
        assert_eq!(result.unwrap_err(), error);
        assert_eq!(calls.len(), 1);
    }
}

#[tokio::test]
async fn lookup_failure_and_register_failure_propagate_without_false_success() {
    for error in [
        "timeout",
        "connection lost",
        "function_not_found",
        "NOT_REGISTERED",
        "remote error (function_not_found): configuration::get",
        "remote error (OTHER): NOT_FOUND",
        "remote error (RESOURCE_NOT_FOUND): absent",
        "remote error (ADAPTER_ERROR): NOT_FOUND",
        "configuration::ensure failed after 3 attempts: remote error (NOT_FOUND): absent",
    ] {
        let (result, calls) =
            run_case(Some("function_not_found"), Err(error.into()), Ok(json!({}))).await;
        assert_eq!(result.unwrap_err(), error);
        assert_eq!(calls.len(), 2);
    }
    let (result, calls) = run_case(
        Some("function_not_found"),
        Ok(json!({"value":null})),
        Err("remote error (SCHEMA_INVALID): rejected".into()),
    )
    .await;
    assert_eq!(
        result.unwrap_err(),
        "remote error (SCHEMA_INVALID): rejected"
    );
    assert_eq!(calls.len(), 3);
}

#[tokio::test]
async fn capability_is_rechecked_after_legacy_initialization() {
    let mut calls = Vec::new();
    for modern in [false, true] {
        ensure_with(payload(), |function, value| {
            calls.push((function, value));
            std::future::ready(match function {
                "configuration::ensure" if !modern => Err("function_not_found".into()),
                "configuration::get" => Ok(json!({"value":false})),
                _ => Ok(json!({})),
            })
        })
        .await
        .unwrap();
    }
    assert_eq!(
        calls
            .iter()
            .map(|(function, _)| *function)
            .collect::<Vec<_>>(),
        [
            "configuration::ensure",
            "configuration::get",
            "configuration::register",
            "configuration::ensure"
        ]
    );
}

#[tokio::test]
async fn legacy_read_register_race_is_explicitly_not_an_atomic_guarantee() {
    let store = Arc::new(Mutex::new(Value::Null));
    let shared = store.clone();
    ensure_with(payload(), |function, value| {
        let mut store = shared.lock().unwrap();
        let reply = match function {
            "configuration::ensure" => Err("function_not_found".into()),
            "configuration::get" => {
                // Another writer wins after the read snapshot and before register.
                *store = json!({"operator":"concurrent"});
                Ok(json!({"value":null}))
            }
            "configuration::register" => {
                *store = value["initial_value"].clone();
                Ok(json!({}))
            }
            _ => unreachable!(),
        };
        std::future::ready(reply)
    })
    .await
    .unwrap();
    assert_eq!(*store.lock().unwrap(), payload()["initial_value"]);
}

#[tokio::test]
async fn real_sdk_remote_codes_are_distinct_from_local_errors() {
    use iii_sdk::errors::Error;
    let missing = Error::Remote {
        code: "function_not_found".into(),
        message: "configuration::ensure".into(),
        stacktrace: None,
    };
    let (result, calls) = run_case(
        Some(&missing.to_string()),
        Ok(json!({"value":false})),
        Ok(json!({})),
    )
    .await;
    result.unwrap();
    assert_eq!(calls.len(), 3);
    for error in [
        Error::Timeout,
        Error::NotConnected,
        Error::Handler("function_not_found".into()),
        Error::Remote {
            code: "ADAPTER_ERROR".into(),
            message: missing.to_string(),
            stacktrace: None,
        },
    ] {
        let message = error.to_string();
        let (result, calls) =
            run_case(Some(&message), Ok(json!({"value":null})), Ok(json!({}))).await;
        assert_eq!(result.unwrap_err(), message);
        assert_eq!(calls.len(), 1);
    }
}

//! `api_key: "secret://NAME"` over the bus: the reference resolves through
//! `secrets::resolve` (answered by the fake engine) at call time, the resolved
//! value is the bearer sent upstream, and a failed reference is an actionable
//! `missing_key` that never falls back to the boot key.
#[path = "support/fake_engine.rs"]
mod fake_engine;
use iii_sdk::{register_worker, InitOptions};
use judge_typesafe::{configuration, JevClient, JevConfig};
use serde_json::{json, Value};
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::timeout};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, ResponseTemplate,
};

/// Call `function_id` once with `api_key` configured; `secrets::resolve`
/// answers with `resolved` (`Ok` result or `Err` error body). Returns the
/// function's result and the resolve requests the worker made.
async fn call_with_secret(
    function_id: &'static str,
    payload: Value,
    api_key: &str,
    resolved: Result<Value, Value>,
    client: JevClient,
) -> (Value, Vec<Value>) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut payload = Some(payload);
    let engine = fake_engine::start(move |message| {
        match message["type"].as_str() {
            Some("registerfunction") if message["id"] == function_id => {
                let data = payload.take().expect("registered once");
                vec![json!({"type":"invokefunction","invocation_id":"00000000-0000-0000-0000-000000000001","function_id":function_id,"data":data})]
            }
            Some("invokefunction") if message["function_id"] == "secrets::resolve" => {
                tx.send(json!({"resolve": message["data"]})).unwrap();
                let mut reply = json!({"type":"invocationresult","invocation_id":message["invocation_id"],"function_id":"secrets::resolve"});
                match &resolved {
                    Ok(result) => reply["result"] = result.clone(),
                    Err(error) => reply["error"] = error.clone(),
                }
                vec![reply]
            }
            Some("invocationresult") => {
                tx.send(json!({"result": message})).unwrap();
                vec![]
            }
            _ => vec![],
        }
    })
    .await;
    let iii = Arc::new(register_worker(&engine.url, InitOptions::default()));
    let config = configuration::new_cell(JevConfig {
        api_key: Some(api_key.into()),
        ..JevConfig::default()
    });
    judge_typesafe::register(&iii, config, client);
    let mut resolves = Vec::new();
    let result = timeout(Duration::from_secs(5), async {
        loop {
            let event = rx.recv().await.unwrap();
            if let Some(resolve) = event.get("resolve") {
                resolves.push(resolve.clone());
            } else {
                return event["result"].clone();
            }
        }
    })
    .await
    .expect("the call answers");
    iii.shutdown_async().await;
    assert!(result.get("error").is_none(), "{result}");
    (result["result"].clone(), resolves)
}

#[tokio::test]
async fn a_resolved_reference_is_the_bearer_sent_upstream() {
    let provider = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer ts-from-secrets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"models": []})))
        .expect(1)
        .mount(&provider)
        .await;
    let client = JevClient::with_endpoint(
        Some("boot-env-key".into()),
        format!("{}/v1/evaluate", provider.uri()),
    );
    let (result, resolves) = call_with_secret(
        "judge-typesafe::models::list",
        json!({"timeout_ms": 5000}),
        "secret://TYPESAFE_API_KEY",
        Ok(json!({"name": "TYPESAFE_API_KEY", "value": "ts-from-secrets"})),
        client,
    )
    .await;
    assert_eq!(result["status"], "ok", "{result}");
    assert_eq!(resolves.len(), 1);
    assert_eq!(resolves[0]["ref"], "secret://TYPESAFE_API_KEY");
}

#[tokio::test]
async fn an_unresolvable_reference_is_missing_key_with_the_reason_and_no_fallback() {
    let provider = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&provider)
        .await;
    for (error, expected) in [
        (
            json!({"code": "SECRET_FORBIDDEN", "message": "caller not in consumers"}),
            "judge-typesafe is not allowed to read secret TYPESAFE_API_KEY; add judge-typesafe to the secret's consumers",
        ),
        (
            json!({"code": "SECRET_NOT_FOUND", "message": "no such secret"}),
            "secret TYPESAFE_API_KEY not found in the secrets worker; store it there or fix the reference",
        ),
        (
            json!({"code": "function_not_found", "message": "Function secrets::resolve not found"}),
            "secrets worker is not running; start it to resolve secret://TYPESAFE_API_KEY",
        ),
    ] {
        let client = JevClient::with_endpoint(
            Some("boot-env-key".into()),
            format!("{}/v1/evaluate", provider.uri()),
        );
        let (result, _) = call_with_secret(
            "judge-typesafe::evaluate",
            json!({"timeout_ms":1000,"evaluations":[{"id":"t","state":{},"questions":{"q":{"type":"noul","instructions":"Is this urgent?"}}}]}),
            "secret://TYPESAFE_API_KEY",
            Err(error),
            client,
        )
        .await;
        assert_eq!(result["status"], "error");
        assert_eq!(result["code"], "missing_key");
        assert_eq!(result["provider_error"]["message"], expected);
        assert!(!result.to_string().contains("boot-env-key"));
    }
}

#[tokio::test]
async fn a_malformed_reference_is_rejected_without_a_lookup_or_an_echo() {
    let (result, resolves) = call_with_secret(
        "judge-typesafe::models::list",
        json!({"timeout_ms": 1000}),
        "secret://ts pasted key",
        Ok(json!({})),
        JevClient::new(Some("boot-env-key".into())),
    )
    .await;
    assert_eq!(result["code"], "missing_key");
    assert!(resolves.is_empty());
    let message = result["provider_error"]["message"].as_str().unwrap();
    assert!(message.contains("malformed secret reference"), "{message}");
    assert!(!message.contains("pasted"), "{message}");
}

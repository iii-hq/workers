//! `api_key: "secret://NAME"` over the bus: the reference resolves through
//! `secrets::resolve` (answered by the fake engine) at call time, the resolved
//! value is the bearer sent upstream, and a failed reference is an actionable
//! `missing_key` that never falls back to the boot key.
#[allow(dead_code)]
#[path = "../../judge-typesafe/tests/support/fake_engine.rs"]
mod fake_engine;
use iii_sdk::{register_worker, InitOptions};
use judge_openai::{configuration, DecisionsClient, OpenAiConfig};
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
    client: DecisionsClient,
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
    let config = configuration::new_cell(OpenAiConfig {
        api_key: Some(api_key.into()),
        ..OpenAiConfig::default()
    });
    judge_openai::register(&iii, config, client);
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
        .and(header("authorization", "Bearer sk-from-secrets"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
        .expect(1)
        .mount(&provider)
        .await;
    let client = DecisionsClient::with_endpoint(
        Some("boot-env-key".into()),
        format!("{}/v1/decisions", provider.uri()),
    );
    let (result, resolves) = call_with_secret(
        "judge-openai::models::list",
        json!({"timeout_ms": 5000}),
        "secret://OPENAI_API_KEY",
        Ok(json!({"name": "OPENAI_API_KEY", "value": "sk-from-secrets"})),
        client,
    )
    .await;
    assert_eq!(result["status"], "ok", "{result}");
    assert_eq!(resolves.len(), 1);
    assert_eq!(resolves[0]["ref"], "secret://OPENAI_API_KEY");
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
            "judge-openai is not allowed to read secret OPENAI_API_KEY; add judge-openai to the secret's consumers",
        ),
        (
            json!({"code": "SECRET_NOT_FOUND", "message": "no such secret"}),
            "secret OPENAI_API_KEY not found in the secrets worker; store it there or fix the reference",
        ),
        (
            json!({"code": "function_not_found", "message": "Function secrets::resolve not found"}),
            "secrets worker is not running; start it to resolve secret://OPENAI_API_KEY",
        ),
    ] {
        let client = DecisionsClient::with_endpoint(
            Some("boot-env-key".into()),
            format!("{}/v1/decisions", provider.uri()),
        );
        let (result, _) = call_with_secret(
            "judge-openai::evaluate",
            json!({"timeout_ms":1000,"evaluations":[{"id":"t","state":{},"questions":{"q":{"type":"noul","instructions":"Is this urgent?"}}}]}),
            "secret://OPENAI_API_KEY",
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
        "judge-openai::models::list",
        json!({"timeout_ms": 1000}),
        "secret://sk pasted key",
        Ok(json!({})),
        DecisionsClient::new(Some("boot-env-key".into())),
    )
    .await;
    assert_eq!(result["code"], "missing_key");
    assert!(resolves.is_empty());
    let message = result["provider_error"]["message"].as_str().unwrap();
    assert!(message.contains("malformed secret reference"), "{message}");
    assert!(!message.contains("pasted"), "{message}");
}

#[tokio::test]
async fn a_secrets_changed_event_rotates_the_cached_key() {
    let provider = MockServer::start().await;
    for key in ["sk-first", "sk-rotated"] {
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("authorization", format!("Bearer {key}").as_str()))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .expect(1)
            .mount(&provider)
            .await;
    }
    let invoke = |id: u64, function_id: &str, data: Value| json!({"type":"invokefunction","invocation_id":format!("00000000-0000-0000-0000-{id:012}"),"function_id":function_id,"data":data});
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut values = vec!["sk-rotated", "sk-first"];
    let engine = fake_engine::start(move |message| match message["type"].as_str() {
        // Registered last: every handler is in place.
        Some("registerfunction") if message["id"] == judge_openai::SECRET_CHANGED_ID => {
            vec![invoke(1, judge_openai::MODELS_ID, json!({"timeout_ms": 5000}))]
        }
        Some("invokefunction") if message["function_id"] == "secrets::resolve" => {
            let value = values.pop().expect("resolved once per rotation");
            vec![json!({"type":"invocationresult","invocation_id":message["invocation_id"],"function_id":"secrets::resolve","result":{"name":"OPENAI_API_KEY","value":value}})]
        }
        Some("invocationresult") => {
            tx.send(message["result"].clone()).unwrap();
            match message["invocation_id"].as_str() {
                Some("00000000-0000-0000-0000-000000000001") => vec![invoke(
                    2,
                    judge_openai::SECRET_CHANGED_ID,
                    json!({"name": "OPENAI_API_KEY", "action": "updated"}),
                )],
                Some("00000000-0000-0000-0000-000000000002") => {
                    vec![invoke(3, judge_openai::MODELS_ID, json!({"timeout_ms": 5000}))]
                }
                _ => vec![],
            }
        }
        _ => vec![],
    })
    .await;
    let iii = Arc::new(register_worker(&engine.url, InitOptions::default()));
    let config = configuration::new_cell(OpenAiConfig {
        api_key: Some("secret://OPENAI_API_KEY".into()),
        ..OpenAiConfig::default()
    });
    let secrets = judge_openai::register(
        &iii,
        config,
        DecisionsClient::with_endpoint(None, format!("{}/v1/decisions", provider.uri())),
    );
    judge_provider::secrets::register_secret_trigger(
        &iii,
        secrets,
        judge_openai::SECRET_CHANGED_ID,
    );
    let results = timeout(Duration::from_secs(5), async {
        let mut results = Vec::new();
        while results.len() < 3 {
            results.push(rx.recv().await.unwrap());
        }
        results
    })
    .await
    .expect("list, evict and list again");
    iii.shutdown_async().await;
    assert_eq!(results[0]["status"], "ok", "{results:?}");
    assert_eq!(results[1], json!({"ok": true}));
    assert_eq!(results[2]["status"], "ok", "{results:?}");
}

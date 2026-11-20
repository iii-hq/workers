//! `auth: codex` over the bus: each call asks `provider::openai-codex::auth::token`
//! (answered by the fake engine) for the bearer, never uses api_key or the boot key,
//! and a token that cannot be had is an actionable `missing_key` with no HTTP.
#[allow(dead_code)]
#[path = "../../judge-typesafe/tests/support/fake_engine.rs"]
mod fake_engine;
use iii_sdk::{InitOptions, register_worker};
use judge_openai::{Auth, DecisionsClient, OpenAiConfig, configuration};
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::{sync::mpsc, time::timeout};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path},
};

const TOKEN_ID: &str = "provider::openai-codex::auth::token";

/// Call `function_id` once with `auth: codex`; the token function answers with
/// `answer` (`Ok` result or `Err` error body). Returns the function's result and
/// the token requests the worker made.
async fn call_with_codex(
    function_id: &'static str,
    payload: Value,
    answer: Result<Value, Value>,
    client: DecisionsClient,
) -> (Value, Vec<Value>) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let mut payload = Some(payload);
    let engine = fake_engine::start(move |message| match message["type"].as_str() {
        Some("registerfunction") if message["id"] == function_id => {
            let data = payload.take().expect("registered once");
            vec![
                json!({"type":"invokefunction","invocation_id":"00000000-0000-0000-0000-000000000001","function_id":function_id,"data":data}),
            ]
        }
        Some("invokefunction") if message["function_id"] == TOKEN_ID => {
            tx.send(json!({"token": message["data"]})).unwrap();
            let mut reply = json!({"type":"invocationresult","invocation_id":message["invocation_id"],"function_id":TOKEN_ID});
            match &answer {
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
    })
    .await;
    let iii = Arc::new(register_worker(&engine.url, InitOptions::default()));
    let config = configuration::new_cell(OpenAiConfig {
        auth: Auth::Codex,
        api_key: Some("configured-key".into()),
        ..OpenAiConfig::default()
    });
    judge_openai::register(&iii, config, client);
    let mut requests = Vec::new();
    let result = timeout(Duration::from_secs(5), async {
        loop {
            let event = rx.recv().await.unwrap();
            if let Some(request) = event.get("token") {
                requests.push(request.clone());
            } else {
                return event["result"].clone();
            }
        }
    })
    .await
    .expect("the call answers");
    iii.shutdown_async().await;
    assert!(result.get("error").is_none(), "{result}");
    (result["result"].clone(), requests)
}

fn client(provider: &MockServer) -> DecisionsClient {
    DecisionsClient::with_endpoint(
        Some("boot-env-key".into()),
        format!("{}/v1/decisions", provider.uri()),
    )
}

fn evaluation() -> Value {
    json!({"timeout_ms":1000,"evaluations":[{"id":"t","state":{},"questions":{"q":{"type":"noul","instructions":"Is this urgent?"}}}]})
}

#[tokio::test]
async fn evaluate_sends_the_codex_token_as_the_bearer() {
    let provider = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/decisions"))
        .and(header("authorization", "Bearer codex-access"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"model": "gpt-6-luna",
            "answers": [{"type": "predicate", "name": "q", "probability": 0.75}],
            "usage": {"input_tokens": 1, "output_tokens": 0}})),
        )
        .expect(1)
        .mount(&provider)
        .await;
    let (result, requests) = call_with_codex(
        "judge-openai::evaluate",
        evaluation(),
        Ok(json!({"access_token": "codex-access"})),
        client(&provider),
    )
    .await;
    assert_eq!(result["status"], "ok", "{result}");
    assert_eq!(requests, vec![json!({})]);
}

#[tokio::test]
async fn models_list_and_status_need_only_a_codex_session() {
    let provider = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&provider)
        .await;
    let (result, requests) = call_with_codex(
        "judge-openai::models::list",
        json!({"timeout_ms": 1000}),
        Ok(json!({"access_token": "codex-access"})),
        client(&provider),
    )
    .await;
    assert_eq!(result["status"], "ok", "{result}");
    assert_eq!(result["models"][0]["name"], "gpt-6-luna");
    assert_eq!(result["stats"]["requests"], 0);
    assert_eq!(requests.len(), 1);
    let (result, _) = call_with_codex(
        "judge-openai::status",
        json!({}),
        Ok(json!({"access_token": "codex-access"})),
        client(&provider),
    )
    .await;
    assert_eq!(result, json!({"configured": true}));
}

#[tokio::test]
async fn a_token_that_cannot_be_had_is_missing_key_with_the_reason_and_no_fallback() {
    let provider = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&provider)
        .await;
    for (answer, expected) in [
        (
            Err(json!({"code": "function_not_found", "message": "Function not found"})),
            "provider-openai-codex is not running, or older than the version with auth::token; start or upgrade it, or set auth: api_key",
        ),
        (
            Ok(json!({"access_token": null})),
            "not signed in to Codex; sign in with ChatGPT in the provider-openai-codex settings",
        ),
        (
            Err(json!({"code": "provider/auth_expired", "message": "codex-secret-marker"})),
            "the Codex session expired; sign in with ChatGPT again",
        ),
        (
            Err(json!({"code": "provider/forbidden", "message": "codex-secret-marker"})),
            "provider-openai-codex refused judge-openai (different namespace?)",
        ),
        (
            Err(json!({"code": "provider/storage_unavailable", "message": "codex-secret-marker"})),
            "could not read the Codex session (storage_unavailable); retry",
        ),
        (
            Ok(json!({"token": "codex-secret-marker"})),
            "could not read the Codex session (malformed_response); retry",
        ),
    ] {
        let (result, _) = call_with_codex(
            "judge-openai::evaluate",
            evaluation(),
            answer.clone(),
            client(&provider),
        )
        .await;
        assert_eq!(result["status"], "error");
        assert_eq!(result["code"], "missing_key");
        assert_eq!(result["provider_error"]["message"], expected);
        assert_eq!(result["stats"]["attempts"], 0);
        assert!(!result.to_string().contains("codex-secret-marker"));
        let (status, _) =
            call_with_codex("judge-openai::status", json!({}), answer, client(&provider)).await;
        assert_eq!(status, json!({"configured": false, "reason": expected}));
    }
}

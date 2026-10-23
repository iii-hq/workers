//! Standalone judge-openai over a real engine with loopback-only Decisions mocks.
//! Run the two ignored cases with .github/scripts/judge-e2e.sh.
#[allow(dead_code)]
#[path = "../../judge-typesafe/tests/support/mod.rs"]
mod support;

use std::time::Duration;

use judge_contract::EvaluateResponse;
use judge_openai::{
    CANCEL_ID as CANCEL_FUNCTION_ID, EVALUATE_ID as FUNCTION_ID, MODELS_ID as MODELS_FUNCTION_ID,
};
use serde_json::{json, Value};
use support::{connect, invoke, wait_for, Engine};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn register(provider: &iii_sdk::IIIClient, server: &MockServer) {
    judge_openai::register(
        provider,
        judge_openai::configuration::new_cell(judge_openai::OpenAiConfig::default()),
        // In-process clients never read OPENAI_API_KEY from the environment.
        judge_openai::DecisionsClient::with_endpoint(
            Some("local-test-key".into()),
            format!("{}/v1/decisions", server.uri()),
        ),
    );
}

#[tokio::test]
#[ignore = "requires III_ENGINE_BIN; starts an isolated real engine, uses only local mocked HTTP"]
async fn independent_consumer_evaluates_mixed_decisions_and_lists_models() {
    let engine = Engine::start("openai_independent_consumer").await;
    let server = MockServer::start().await;
    let provider = connect(&engine.url, "judge-openai").await;
    let consumer = connect(&engine.url, "ticket-consumer").await;
    register(&provider, &server);
    wait_for(&consumer, FUNCTION_ID).await;

    let questions = json!({
        "urgent": {"type":"noul", "instructions":null, "criteria":{"true":["urgent"]}},
        "department": {"type":"choice", "criteria":{"billing":null,"support":{"scope":"bugs"}}},
        "owner": {"type":"choice", "criteria":{"oncall":"Whoever is on call"}},
        "severity": {"type":"score", "instructions":["Assess impact"], "criteria":["routine",{"impact":"blocking"}]}
    });
    Mock::given(method("POST"))
        .and(path("/v1/decisions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model": "gpt-6-luna",
            "answers": [
                {"type":"predicate", "name":"urgent", "probability":0.9},
                {"type":"score", "name":"severity", "score":0.75, "probabilities":[
                    {"value":0, "label":"routine", "probability":0.25},
                    {"value":1, "label":"{\"impact\":\"blocking\"}", "probability":0.75}], "confidence":0.5},
                {"type":"choice", "name":"department", "choice":"support", "probabilities":[
                    {"value":"billing", "probability":0.1}, {"value":"support", "probability":0.9}], "confidence":0.8}
            ],
            "usage": {"input_tokens":12, "input_tokens_details":{"cached_tokens":0,"cache_write_tokens":0},
                      "output_tokens":0, "output_tokens_details":{"reasoning_tokens":0}, "total_tokens":12}
        })))
        .expect(1)
        .mount(&server)
        .await;
    let evaluated = invoke(
        &consumer,
        FUNCTION_ID,
        json!({
            "timeout_ms":3000, "evaluations":[{
                "id":"ticket", "state":{"message":"Sign-in is blocked"}, "questions":questions
            }]
        }),
    )
    .await
    .unwrap();
    assert_eq!(evaluated["status"], "ok", "{evaluated}");
    assert_eq!(evaluated["model"], "gpt-6-luna");
    assert_eq!(
        evaluated["results"]["ticket"]["answers"],
        json!({
            "urgent": {"type":"noul", "noul":0.9},
            "department": {"type":"choice", "choice":"support", "probabilities":{"billing":0.1,"support":0.9}, "confidence":0.8},
            "owner": {"type":"choice", "choice":"oncall", "probabilities":{"oncall":1.0}, "confidence":1.0},
            "severity": {"type":"score", "score":0.75, "probabilities":{"0":0.25,"1":0.75}, "confidence":0.5,
                         "legend":{"0":"routine","1":{"impact":"blocking"}}}
        })
    );
    assert_eq!(
        evaluated["results"]["ticket"]["usage"],
        json!({"input_tokens": 12, "output_tokens": 0})
    );
    assert_eq!(evaluated["stats"]["requests"], 1);
    assert_eq!(evaluated["stats"]["questions"], 4);
    assert_eq!(evaluated["stats"]["input_tokens"], 12);
    assert_eq!(evaluated["stats"]["usage_complete"], true);
    let received = server.received_requests().await.unwrap();
    assert_eq!(
        received[0].headers["authorization"],
        "Bearer local-test-key"
    );
    let sent: Value = serde_json::from_slice(&received[0].body).unwrap();
    assert_eq!(
        sent,
        json!({
            "model": "gpt-6-luna",
            "input": "{\"message\":\"Sign-in is blocked\"}",
            "questions": [
                {"type":"choice", "name":"department", "instructions":"Which choice best fits the input?",
                 "choices":[{"value":"billing"}, {"value":"support", "description":"{\"scope\":\"bugs\"}"}]},
                {"type":"score", "name":"severity", "instructions":"[\"Assess impact\"]",
                 "levels":[{"label":"routine"}, {"label":"{\"impact\":\"blocking\"}"}]},
                {"type":"predicate", "name":"urgent", "instructions":"Is this true of the input?\n\nTrue means: [\"urgent\"]"}
            ]
        })
    );

    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(move |request: &Request| {
            assert!(request.body.is_empty());
            ResponseTemplate::new(200).set_body_json(json!({"object":"list", "data":[
                {"id":"gpt-6-luna", "object":"model", "created":1789406102, "owned_by":"system", "shutdown_date":null},
                {"id":"gpt-5.6-luna", "object":"model", "created":1780000000, "owned_by":"system"}
            ]}))
        })
        .expect(1)
        .mount(&server)
        .await;
    wait_for(&consumer, MODELS_FUNCTION_ID).await;
    let listed = invoke(&consumer, MODELS_FUNCTION_ID, json!({"timeout_ms":3000}))
        .await
        .unwrap();
    assert_eq!(listed["status"], "ok", "{listed}");
    assert_eq!(
        listed["models"],
        json!([{"name":"gpt-6-luna", "description":"OpenAI Decisions (beta)",
                "release_date":"2026-09-14", "context_window":922000}])
    );
    assert_eq!(listed["stats"]["input_tokens"], 0);

    let mut invalid_questions = questions;
    invalid_questions["severity"]["criteria"][0] = Value::Null;
    let invalid = invoke(
        &consumer,
        FUNCTION_ID,
        json!({
            "timeout_ms":3000, "evaluations":[{
                "id":"ticket", "state":{"message":"Sign-in is blocked"},
                "questions":invalid_questions
            }]
        }),
    )
    .await
    .unwrap();
    assert_eq!(invalid["status"], "error");
    assert_eq!(invalid["code"], "invalid_request");
    assert_eq!(invalid["stats"]["attempts"], 0);
    assert!(invalid.get("results").is_none());
    server.verify().await;
    server.reset().await;

    for (http_method, endpoint, function_id, mut payload) in [
        (
            "POST",
            "/v1/decisions",
            FUNCTION_ID,
            json!({
                "timeout_ms": 3000, "evaluations": [{
                    "id": "ticket", "state": {}, "questions": {"urgent": {"type": "noul"}}
                }]
            }),
        ),
        (
            "GET",
            "/v1/models",
            MODELS_FUNCTION_ID,
            json!({"timeout_ms": 3000}),
        ),
    ] {
        // Production policy: 429 is retried twice, honoring the 250 ms hint.
        Mock::given(method(http_method))
            .and(path(endpoint))
            .respond_with(|_: &Request| {
                ResponseTemplate::new(429)
                    .insert_header("retry-after-ms", "250")
                    .set_body_json(json!({"error": {
                        "message": "Rate limit reached for local-test-key",
                        "type": "requests", "param": null, "code": "rate_limit_exceeded"
                    }}))
            })
            .expect(3)
            .mount(&server)
            .await;
        payload["options"] = json!({"attempt_timeout_ms": 1000});
        let error = invoke(&consumer, function_id, payload).await.unwrap();
        assert_eq!(error["code"], "http", "{error}");
        assert_eq!(error["http_status"], 429);
        assert_eq!(error["retry_after_ms"], 250);
        assert_eq!(
            error["provider_error"]["detail"]["error"]["code"],
            "rate_limit_exceeded"
        );
        assert_eq!(error["provider_error"]["truncated"], false);
        assert!(!error.to_string().contains("local-test-key"));
        assert_eq!(error["stats"]["attempts"], 3);
        if function_id == FUNCTION_ID {
            serde_json::from_value::<EvaluateResponse>(error).unwrap();
        } else {
            serde_json::from_value::<judge_contract::ModelsResponse>(error).unwrap();
        }
        server.verify().await;
        server.reset().await;
    }
    provider.shutdown_async().await;
    consumer.shutdown_async().await;
}

#[tokio::test]
#[ignore = "requires III_ENGINE_BIN; starts an isolated real engine, uses only local mocked HTTP"]
async fn cancellation_is_scoped_to_the_persistent_engine_caller() {
    let engine = Engine::start("openai_persistent_caller_cancellation").await;
    let server = MockServer::start().await;
    let provider = connect(&engine.url, "judge-openai").await;
    let owner = connect(&engine.url, "ticket-owner").await;
    let other = connect(&engine.url, "other-consumer").await;
    register(&provider, &server);
    wait_for(&owner, FUNCTION_ID).await;
    wait_for(&owner, CANCEL_FUNCTION_ID).await;
    Mock::given(method("POST"))
        .and(path("/v1/decisions"))
        .respond_with(|request: &Request| {
            let payload: Value = serde_json::from_slice(&request.body).unwrap();
            assert!(payload.get("request_id").is_none());
            assert!(payload.get("_caller_worker_id").is_none());
            ResponseTemplate::new(200)
                .set_body_json(json!({
                    "model": "gpt-6-luna",
                    "answers": [{"type": "predicate", "name": "urgent", "probability": 0.9}],
                    "usage": {"input_tokens": 12, "output_tokens": 0}
                }))
                .set_delay(Duration::from_secs(30))
        })
        .expect(1)
        .mount(&server)
        .await;

    let evaluating_owner = owner.clone();
    let evaluation = tokio::spawn(async move {
        invoke(
            &evaluating_owner,
            FUNCTION_ID,
            json!({
                "request_id": "ticket-run-42", "timeout_ms": 3000,
                "evaluations": [{
                    "id": "ticket", "state": {"message": "Sign-in is blocked"},
                    "questions": {"urgent": {"type": "noul"}}
                }]
            }),
        )
        .await
        .unwrap()
    });
    // Observe the active upstream request before cancelling, avoiding a start/cancel race.
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.received_requests().await.unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("identified evaluation starts HTTP");

    let cancel_payload = json!({"request_id": "ticket-run-42"});
    let denied = invoke(&other, CANCEL_FUNCTION_ID, cancel_payload.clone())
        .await
        .unwrap();
    assert_eq!(denied, json!({"status": "ok", "cancelled": false}));
    assert!(
        !evaluation.is_finished(),
        "other caller cannot cancel the active evaluation"
    );
    let accepted = invoke(&owner, CANCEL_FUNCTION_ID, cancel_payload.clone())
        .await
        .unwrap();
    assert_eq!(accepted, json!({"status": "ok", "cancelled": true}));
    let cancelled = tokio::time::timeout(Duration::from_secs(2), evaluation)
        .await
        .expect("cancellation interrupts the pending HTTP response")
        .unwrap();
    assert_eq!(cancelled["status"], "error", "{cancelled}");
    assert_eq!(cancelled["code"], "cancelled");
    assert!(cancelled.get("results").is_none());
    assert_eq!(cancelled["stats"]["attempts"], 1);
    assert_eq!(cancelled["stats"]["requests"], 0);
    assert_eq!(cancelled["stats"]["usage_complete"], false);
    serde_json::from_value::<EvaluateResponse>(cancelled).unwrap();
    let completed = invoke(&owner, CANCEL_FUNCTION_ID, cancel_payload)
        .await
        .unwrap();
    assert_eq!(completed, json!({"status": "ok", "cancelled": false}));
    server.verify().await;
    provider.shutdown_async().await;
    owner.shutdown_async().await;
    other.shutdown_async().await;
}

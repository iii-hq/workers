//! Standalone JEV over a real engine with loopback-only provider mocks.
//! Run the two ignored cases with .github/scripts/jev-e2e.sh.
mod support;

use std::time::Duration;

use jev_contract::{EvaluateResponse, CANCEL_FUNCTION_ID, FUNCTION_ID, MODELS_FUNCTION_ID};
use serde_json::{json, Value};
use support::{connect, invoke, wait_for, Engine};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

#[tokio::test]
#[ignore = "requires III_ENGINE_BIN; starts an isolated real engine, uses only local mocked HTTP"]
async fn independent_consumer_evaluates_mixed_primitives_and_lists_models() {
    let engine = Engine::start("independent_consumer").await;
    let server = MockServer::start().await;
    let provider = connect(&engine.url, "jev").await;
    let consumer = connect(&engine.url, "ticket-consumer").await;
    jev::register(
        &provider,
        jev::configuration::new_cell(jev::JevConfig::default()),
        jev::JevClient::with_endpoint(
            Some("local-test-key".into()),
            format!("{}/v1/systemone", server.uri()),
        ),
    );
    wait_for(&consumer, FUNCTION_ID).await;

    let questions = json!({
        "urgent": {"type":"noul", "instructions":null, "criteria":{"true":["urgent"]}},
        "department": {"type":"choice", "criteria":{"billing":null,"support":{"scope":"bugs"}}},
        "severity": {"type":"score", "instructions":["Assess impact"], "criteria":["routine",{"impact":"blocking"}]}
    });
    let answers = json!({
        "urgent": {"type":"noul", "noul":0.9},
        "department": {"type":"choice", "choice":"support", "probabilities":{"billing":0.1,"support":0.9}, "confidence":0.8},
        "severity": {"type":"score", "score":0.75, "probabilities":{"0":0.25,"1":0.75}, "confidence":0.5, "legend":{"0":"routine","1":{"impact":"blocking"}}}
    });
    let expected_questions = questions.clone();
    let upstream_answers = answers.clone();
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(move |request: &Request| {
            assert_eq!(request.headers["authorization"], "Bearer local-test-key");
            let payload: Value = serde_json::from_slice(&request.body).unwrap();
            assert_eq!(payload["state"], json!({"message":"Sign-in is blocked"}));
            let mut expected = expected_questions.clone();
            expected["department"]["instructions"] = Value::Null;
            assert_eq!(payload["questions"], expected);
            assert!(payload.get("_caller_worker_id").is_none());
            ResponseTemplate::new(200).set_body_json(json!({
                "model":"jev-1.13.0", "answers":upstream_answers,
                "usage":{"input_tokens":12,"output_tokens":null}
            }))
        })
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
    assert_eq!(evaluated["results"]["ticket"]["answers"], answers);
    assert_eq!(
        evaluated["results"]["ticket"]["usage"],
        json!({
            "input_tokens": 12, "output_tokens": null
        })
    );
    assert_eq!(evaluated["stats"]["requests"], 1);
    assert_eq!(evaluated["stats"]["questions"], 3);
    assert_eq!(evaluated["stats"]["input_tokens"], 12);
    assert_eq!(evaluated["stats"]["output_tokens"], 0);
    assert_eq!(evaluated["stats"]["usage_complete"], false);

    let models = json!([
        {"name":"jev-latest", "description":"Latest JEV model", "release_date":"2026-09-01"},
        {"name":"future-model", "description":"A new provider model", "release_date":"2026-09-19"}
    ]);
    let upstream_models = models.clone();
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(move |request: &Request| {
            assert_eq!(request.headers["authorization"], "Bearer local-test-key");
            assert!(request.body.is_empty());
            ResponseTemplate::new(200).set_body_json(json!({"models":upstream_models}))
        })
        .expect(1)
        .mount(&server)
        .await;
    wait_for(&consumer, MODELS_FUNCTION_ID).await;
    let listed = invoke(&consumer, MODELS_FUNCTION_ID, json!({"timeout_ms":3000}))
        .await
        .unwrap();
    assert_eq!(listed["status"], "ok", "{listed}");
    assert_eq!(listed["models"], models);
    assert_eq!(listed["stats"]["input_tokens"], 0);
    assert_eq!(listed["stats"]["output_tokens"], 0);

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
    assert_eq!(invalid["stats"]["requests"], 0);
    assert!(invalid.get("results").is_none());
    server.verify().await;
    server.reset().await;

    for (http_method, endpoint, function_id, mut payload) in [
        (
            "POST",
            "/v1/systemone",
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
        Mock::given(method(http_method))
            .and(path(endpoint))
            .respond_with(|request: &Request| {
                assert_eq!(request.headers["x-request-tag"], "private-request-tag");
                ResponseTemplate::new(429)
                    .insert_header("retry-after-ms", "250")
                    .set_body_json(json!({"detail": [{
                        "loc": ["body", "questions"],
                        "msg": "Rate limited: local-test-key private-request-tag"
                    }]}))
            })
            .expect(1)
            .mount(&server)
            .await;
        payload["options"] = json!({
            "headers": {"x-request-tag": "private-request-tag"},
            "retry": {"max_retries": 0}
        });
        let error = invoke(&consumer, function_id, payload).await.unwrap();
        assert_eq!(error["code"], "http", "{error}");
        assert_eq!(error["http_status"], 429);
        assert_eq!(error["retry_after_ms"], 250);
        assert_eq!(
            error["provider_error"]["detail"][0]["loc"],
            json!(["body", "questions"])
        );
        assert_eq!(error["provider_error"]["truncated"], false);
        assert!(!error.to_string().contains("local-test-key"));
        assert!(!error.to_string().contains("private-request-tag"));
        assert_eq!(error["stats"]["attempts"], 1);
        if function_id == FUNCTION_ID {
            serde_json::from_value::<EvaluateResponse>(error).unwrap();
        } else {
            serde_json::from_value::<jev_contract::ModelsResponse>(error).unwrap();
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
    let engine = Engine::start("persistent_caller_cancellation").await;
    let server = MockServer::start().await;
    let provider = connect(&engine.url, "jev").await;
    let owner = connect(&engine.url, "ticket-owner").await;
    let other = connect(&engine.url, "other-consumer").await;
    jev::register(
        &provider,
        jev::configuration::new_cell(jev::JevConfig::default()),
        jev::JevClient::with_endpoint(
            Some("local-test-key".into()),
            format!("{}/v1/systemone", server.uri()),
        ),
    );
    wait_for(&owner, FUNCTION_ID).await;
    wait_for(&owner, CANCEL_FUNCTION_ID).await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(|request: &Request| {
            let payload: Value = serde_json::from_slice(&request.body).unwrap();
            assert!(payload.get("request_id").is_none());
            assert!(payload.get("_caller_worker_id").is_none());
            ResponseTemplate::new(200)
                .set_body_json(json!({
                    "model": "jev-1.13.0", "answers": {"urgent": {"type": "noul", "noul": 0.9}},
                    "usage": {"input_tokens": 12, "output_tokens": 2}
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
                "options": {"retry": {"max_retries": 0}},
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

//! `judge::*` over a real engine, forwarded to in-process judge-typesafe and
//! judge-openai providers with loopback mocks. Run with .github/scripts/judge-e2e.sh.
#[path = "../../judge-typesafe/tests/support/mod.rs"]
mod support;

use std::{sync::Arc, time::Duration};

use judge_contract::{EvaluateResponse, CANCEL_FUNCTION_ID, FUNCTION_ID, MODELS_FUNCTION_ID};
use judge_typesafe::{JevClient, JevConfig};
use serde_json::{json, Value};
use support::{connect, invoke, wait_for, Engine};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, Request, ResponseTemplate};

fn upstream_answer() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(json!({
        "model":"jev-1.13.0", "answers":{"urgent":{"type":"noul","noul":0.9}},
        "usage":{"input_tokens":12,"output_tokens":2}
    }))
}

#[tokio::test]
#[ignore = "requires III_ENGINE_BIN; starts an isolated real engine, uses only local mocked HTTP"]
async fn hub_forwards_to_the_provider_and_scopes_cancellation_to_the_original_caller() {
    let engine = Engine::start("judge_hub").await;
    let server = MockServer::start().await;
    let provider = connect(&engine.url, "judge-typesafe").await;
    let hub = Arc::new(connect(&engine.url, "judge").await);
    let owner = connect(&engine.url, "ticket-owner").await;
    let other = connect(&engine.url, "other-consumer").await;
    judge_typesafe::register(
        &provider,
        judge_typesafe::configuration::new_cell(JevConfig::default()),
        JevClient::with_endpoint(
            Some("local-test-key".into()),
            format!("{}/v1/systemone", server.uri()),
        ),
    );
    judge::register(
        &hub,
        judge::configuration::new_cell(judge::JudgeConfig::default()),
    );
    for id in [
        FUNCTION_ID,
        MODELS_FUNCTION_ID,
        CANCEL_FUNCTION_ID,
        judge_typesafe::EVALUATE_ID,
        judge_typesafe::MODELS_ID,
        judge_typesafe::CANCEL_ID,
    ] {
        wait_for(&owner, id).await;
    }

    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(|request: &Request| {
            let payload: Value = serde_json::from_slice(&request.body).unwrap();
            assert!(payload.get("request_id").is_none());
            assert!(payload.get("provider").is_none());
            upstream_answer()
        })
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "models":[{"name":"jev-latest","description":"Latest","release_date":"2026-09-01","context_window":4096}]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let evaluation = json!({
        "timeout_ms":3000, "evaluations":[{
            "id":"ticket", "state":{"message":"Sign-in is blocked"}, "questions":{"urgent":{"type":"noul"}}
        }]
    });
    let evaluated = invoke(&owner, FUNCTION_ID, evaluation.clone())
        .await
        .unwrap();
    assert_eq!(evaluated["status"], "ok", "{evaluated}");
    assert_eq!(
        evaluated["results"]["ticket"]["answers"]["urgent"]["noul"],
        0.9
    );
    assert_eq!(evaluated["stats"]["usage_complete"], true);
    let listed = invoke(&owner, MODELS_FUNCTION_ID, json!({"timeout_ms":3000}))
        .await
        .unwrap();
    assert_eq!(listed["models"][0]["name"], "jev-latest", "{listed}");
    // Optional card fields (laya publishes its window) survive the hub hop.
    assert_eq!(listed["models"][0]["context_window"], 4096, "{listed}");
    let mut elsewhere = evaluation.clone();
    elsewhere["provider"] = json!("missing");
    let unavailable = invoke(&owner, FUNCTION_ID, elsewhere).await.unwrap();
    assert_eq!(unavailable["code"], "provider_unavailable", "{unavailable}");
    server.verify().await;
    server.reset().await;

    // Cancellation through the hub stays scoped to the original engine caller.
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(upstream_answer().set_delay(Duration::from_secs(30)))
        .expect(1)
        .mount(&server)
        .await;
    let mut identified = evaluation;
    identified["request_id"] = json!("ticket-run-42");
    let evaluating_owner = owner.clone();
    let pending = tokio::spawn(async move {
        invoke(&evaluating_owner, FUNCTION_ID, identified)
            .await
            .unwrap()
    });
    tokio::time::timeout(Duration::from_secs(2), async {
        while server.received_requests().await.unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("identified evaluation starts HTTP");
    let cancel = json!({"request_id":"ticket-run-42"});
    assert_eq!(
        invoke(&other, CANCEL_FUNCTION_ID, cancel.clone())
            .await
            .unwrap(),
        json!({"status":"ok","cancelled":false})
    );
    assert!(
        !pending.is_finished(),
        "other caller cannot cancel through the hub"
    );
    assert_eq!(
        invoke(&owner, CANCEL_FUNCTION_ID, cancel.clone())
            .await
            .unwrap(),
        json!({"status":"ok","cancelled":true})
    );
    let cancelled = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .expect("cancellation interrupts the pending HTTP response")
        .unwrap();
    assert_eq!(cancelled["code"], "cancelled", "{cancelled}");
    serde_json::from_value::<EvaluateResponse>(cancelled).unwrap();
    assert_eq!(
        invoke(&owner, CANCEL_FUNCTION_ID, cancel).await.unwrap(),
        json!({"status":"ok","cancelled":false})
    );
    server.verify().await;
    for worker in [provider, owner, other] {
        worker.shutdown_async().await;
    }
    hub.shutdown_async().await;
}

#[tokio::test]
#[ignore = "requires III_ENGINE_BIN; starts an isolated real engine, uses only local mocked HTTP"]
async fn hub_routes_judge_openai_by_provider_and_session_baggage() {
    use opentelemetry::{baggage::BaggageExt, context::FutureExt, Context, KeyValue};
    // What the harness stamps per turn for the calling session's judge.
    let session = |provider: &'static str| {
        Context::current_with_baggage(vec![KeyValue::new(
            judge_contract::PROVIDER_BAGGAGE_KEY,
            provider,
        )])
    };

    let engine = Engine::start("judge_hub_openai").await;
    let (openai, typesafe) = (MockServer::start().await, MockServer::start().await);
    let openai_provider = connect(&engine.url, "judge-openai").await;
    let typesafe_provider = connect(&engine.url, "judge-typesafe").await;
    let hub = Arc::new(connect(&engine.url, "judge").await);
    let owner = connect(&engine.url, "ticket-owner").await;
    judge_openai::register(
        &openai_provider,
        judge_openai::configuration::new_cell(judge_openai::OpenAiConfig::default()),
        judge_openai::DecisionsClient::with_endpoint(
            Some("local-test-key".into()),
            format!("{}/v1/decisions", openai.uri()),
        ),
    );
    judge_typesafe::register(
        &typesafe_provider,
        judge_typesafe::configuration::new_cell(JevConfig::default()),
        JevClient::with_endpoint(
            Some("local-test-key".into()),
            format!("{}/v1/systemone", typesafe.uri()),
        ),
    );
    // The default is not running: only the session or the request can route.
    judge::register(
        &hub,
        judge::configuration::new_cell(judge::JudgeConfig {
            provider: "missing".into(),
            ..Default::default()
        }),
    );
    for id in [
        FUNCTION_ID,
        MODELS_FUNCTION_ID,
        CANCEL_FUNCTION_ID,
        judge_openai::EVALUATE_ID,
        judge_openai::MODELS_ID,
        judge_openai::CANCEL_ID,
        judge_typesafe::EVALUATE_ID,
    ] {
        wait_for(&owner, id).await;
    }

    Mock::given(method("POST"))
        .and(path("/v1/decisions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "model":"gpt-6-luna", "answers":[{"type":"predicate","name":"urgent","probability":0.8}],
            "usage":{"input_tokens":12,"output_tokens":0}
        })))
        .expect(1)
        .mount(&openai)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/systemone"))
        .respond_with(upstream_answer())
        .expect(1)
        .mount(&typesafe)
        .await;
    let evaluation = json!({
        "timeout_ms":3000, "evaluations":[{
            "id":"ticket", "state":{"message":"Sign-in is blocked"}, "questions":{"urgent":{"type":"noul"}}
        }]
    });
    let (via_openai, via_typesafe) = tokio::join!(
        invoke(&owner, FUNCTION_ID, evaluation.clone()).with_context(session("openai")),
        invoke(&owner, FUNCTION_ID, evaluation.clone()).with_context(session("typesafe")),
    );
    let (via_openai, via_typesafe) = (via_openai.unwrap(), via_typesafe.unwrap());
    assert_eq!(via_openai["status"], "ok", "{via_openai}");
    assert_eq!(via_openai["model"], "gpt-6-luna");
    assert_eq!(
        via_openai["results"]["ticket"]["answers"]["urgent"]["noul"],
        0.8
    );
    assert_eq!(via_typesafe["status"], "ok", "{via_typesafe}");
    assert_eq!(via_typesafe["model"], "jev-1.13.0");
    assert_eq!(
        via_typesafe["results"]["ticket"]["answers"]["urgent"]["noul"],
        0.9
    );
    let unrouted = invoke(&owner, FUNCTION_ID, evaluation.clone())
        .await
        .unwrap();
    assert_eq!(unrouted["code"], "provider_unavailable", "{unrouted}");
    openai.verify().await;
    openai.reset().await;

    // From here on an explicit `provider` beats the session's: every call
    // carries typesafe baggage and reaches judge-openai.
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"object":"list", "data":[
            {"id":"gpt-6-luna", "object":"model", "created":1789406102, "owned_by":"system", "shutdown_date":null}
        ]})))
        .expect(1)
        .mount(&openai)
        .await;
    let listed = invoke(
        &owner,
        MODELS_FUNCTION_ID,
        json!({"timeout_ms":3000, "provider":"openai"}),
    )
    .with_context(session("typesafe"))
    .await
    .unwrap();
    assert_eq!(listed["status"], "ok", "{listed}");
    assert_eq!(listed["models"][0]["name"], "gpt-6-luna", "{listed}");

    Mock::given(method("POST"))
        .and(path("/v1/decisions"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"model":"gpt-6-luna", "answers":[]}))
                .set_delay(Duration::from_secs(30)),
        )
        .expect(1)
        .mount(&openai)
        .await;
    let mut identified = evaluation;
    identified["provider"] = json!("openai");
    identified["request_id"] = json!("ticket-run-42");
    let evaluating_owner = owner.clone();
    let pending = tokio::spawn(
        async move {
            invoke(&evaluating_owner, FUNCTION_ID, identified)
                .await
                .unwrap()
        }
        .with_context(session("typesafe")),
    );
    tokio::time::timeout(Duration::from_secs(2), async {
        while openai.received_requests().await.unwrap().len() < 2 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("identified evaluation starts HTTP");
    let cancel = json!({"request_id":"ticket-run-42", "provider":"openai"});
    assert_eq!(
        invoke(&owner, CANCEL_FUNCTION_ID, cancel)
            .with_context(session("typesafe"))
            .await
            .unwrap(),
        json!({"status":"ok","cancelled":true})
    );
    let cancelled = tokio::time::timeout(Duration::from_secs(2), pending)
        .await
        .expect("cancellation interrupts the pending HTTP response")
        .unwrap();
    assert_eq!(cancelled["code"], "cancelled", "{cancelled}");
    serde_json::from_value::<EvaluateResponse>(cancelled).unwrap();
    openai.verify().await;
    // judge-typesafe saw only its own session's call.
    typesafe.verify().await;
    for worker in [openai_provider, typesafe_provider, owner] {
        worker.shutdown_async().await;
    }
    hub.shutdown_async().await;
}

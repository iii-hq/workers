//! DecisionsClient against a loopback Decisions API (wiremock): wire shape,
//! limits, deadlines, retries, permits, cancellation and usage accounting.
use judge_contract::{
    CancelRequest, EvaluateRequest, EvaluateResponse, ModelsRequest, ModelsResponse, Stats,
};
use judge_openai::{DecisionsClient, ExecutionLimits, RetryPolicy, DEFAULT_MODEL, DEFAULT_RETRY};
use serde_json::{json, Value};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::{sleep, timeout},
};
use wiremock::{
    matchers::{header, method, path},
    Mock, MockServer, Request, ResponseTemplate,
};

fn client(server: &MockServer) -> DecisionsClient {
    DecisionsClient::with_endpoint(
        Some("local-test-key".into()),
        format!("{}/v1/decisions", server.uri()),
    )
    .with_retry(fast(0))
}
/// Millisecond backoff so retry paths run fast; `DEFAULT_RETRY` is production.
fn fast(max_retries: u32) -> RetryPolicy {
    RetryPolicy {
        max_retries,
        backoff_initial_ms: 1,
        backoff_max_ms: 4,
        ..DEFAULT_RETRY
    }
}
fn request(value: Value) -> EvaluateRequest {
    serde_json::from_value(value).unwrap()
}
fn ticket() -> EvaluateRequest {
    tickets(1)
}
fn tickets(count: usize) -> EvaluateRequest {
    request(
        json!({"timeout_ms": 3000, "evaluations": (0..count).map(|i| json!({
        "id": format!("ticket-{i}"), "state": format!("ticket {i}: cannot sign in"),
        "questions": {"urgent": {"type": "noul", "instructions": "Is this urgent?"}}
    })).collect::<Vec<_>>()}),
    )
}
/// A predicate answer for every question sent.
fn answer_all(request: &Request) -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(decision(request, "gpt-6-luna"))
}
fn decision(request: &Request, model: &str) -> Value {
    let body: Value = serde_json::from_slice(&request.body).unwrap();
    let answers: Vec<Value> = body["questions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|question| json!({"type": "predicate", "name": question["name"], "probability": 0.75}))
        .collect();
    json!({"model": model, "answers": answers, "usage": {"input_tokens": 11, "output_tokens": 0}})
}
async fn mount(server: &MockServer, template: impl wiremock::Respond + 'static) {
    Mock::given(method("POST"))
        .and(path("/v1/decisions"))
        .respond_with(template)
        .mount(server)
        .await;
}
fn ok(response: EvaluateResponse) -> (String, Value, Stats) {
    match response {
        EvaluateResponse::Ok {
            model,
            results,
            stats,
        } => (model, serde_json::to_value(results).unwrap(), stats),
        other => panic!("expected success, got {other:?}"),
    }
}
fn error(response: EvaluateResponse, code: &str) -> (Value, Stats) {
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(wire["status"], "error", "{wire}");
    assert_eq!(wire["code"], code, "{wire}");
    assert!(wire.get("results").is_none());
    match response {
        EvaluateResponse::Error { stats, .. } => (wire, stats),
        _ => unreachable!(),
    }
}
fn models_error(response: ModelsResponse, code: &str) -> (Value, Stats) {
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(wire["code"], code, "{wire}");
    match response {
        ModelsResponse::Error { stats, .. } => (wire, stats),
        _ => unreachable!(),
    }
}
fn models(timeout_ms: u64) -> ModelsRequest {
    ModelsRequest {
        timeout_ms,
        ..ModelsRequest::default()
    }
}
fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64
}

#[tokio::test]
async fn a_missing_key_sends_no_http_even_for_a_local_only_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let keyless = DecisionsClient::with_endpoint(None, format!("{}/v1/decisions", server.uri()))
        .with_api_key(Some("   "));
    let local = request(
        json!({"timeout_ms": 1000, "evaluations": [{"id": "pick", "state": "s",
        "questions": {"c0": {"type": "choice", "criteria": {"only": null}}}}]}),
    );
    for request in [ticket(), local] {
        let (_, stats) = error(
            keyless.evaluate(request, DEFAULT_MODEL).await,
            "missing_key",
        );
        assert_eq!((stats.attempts, stats.requests), (0, 0));
    }
    let (_, stats) = models_error(keyless.list_models(models(1000)).await, "missing_key");
    assert_eq!(stats.attempts, 0);
}

#[tokio::test]
async fn a_configured_key_takes_precedence_over_the_boot_key() {
    let server = MockServer::start().await;
    for key in ["configured-key", "boot-env-key"] {
        Mock::given(method("POST"))
            .and(header("authorization", format!("Bearer {key}").as_str()))
            .respond_with(answer_all)
            .expect(1)
            .mount(&server)
            .await;
    }
    let boot = DecisionsClient::with_endpoint(
        Some("boot-env-key".into()),
        format!("{}/v1/decisions", server.uri()),
    );
    ok(boot
        .with_api_key(Some(" configured-key "))
        .evaluate(ticket(), DEFAULT_MODEL)
        .await);
    ok(boot
        .with_api_key(None)
        .evaluate(ticket(), DEFAULT_MODEL)
        .await);
}

#[tokio::test]
async fn mixed_local_and_remote_evaluations_make_one_call_per_remote_evaluation() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/decisions"))
        .respond_with(answer_all)
        .expect(1)
        .mount(&server)
        .await;
    let only = json!({"type": "choice", "criteria": {"state::get": {"function_id": "state::get"}}});
    let mixed = request(json!({"timeout_ms": 3000, "evaluations": [
        {"id": "both", "state": {"capability": "read state"}, "questions": {
            "c0": only, "urgent": {"type": "noul"}}},
        {"id": "local", "state": "s", "questions": {"c0": only}}
    ]}));
    let (model, results, stats) = ok(client(&server).evaluate(mixed, DEFAULT_MODEL).await);
    assert_eq!(model, "gpt-6-luna");
    let local = json!({"type": "choice", "choice": "state::get", "probabilities": {"state::get": 1.0}, "confidence": 1.0});
    assert_eq!(results["both"]["answers"]["c0"], local);
    assert_eq!(results["both"]["answers"]["urgent"]["noul"], 0.75);
    assert_eq!(results["local"]["answers"], json!({"c0": local}));
    assert_eq!(
        results["local"]["usage"],
        json!({"input_tokens": 0, "output_tokens": 0})
    );
    assert_eq!(
        (
            stats.attempts,
            stats.requests,
            stats.questions,
            stats.input_tokens
        ),
        (1, 1, 3, 11)
    );
    assert!(stats.usage_complete);
    let sent: Value =
        serde_json::from_slice(&server.received_requests().await.unwrap()[0].body).unwrap();
    assert_eq!(sent["questions"].as_array().unwrap().len(), 1);

    // No HTTP at all: the effective model, zero usage, complete accounting.
    let local_only = request(json!({"timeout_ms": 3000, "evaluations": [
        {"id": "local", "state": "s", "questions": {"c0": only}}]}));
    let (model, _, stats) = ok(client(&server).evaluate(local_only, DEFAULT_MODEL).await);
    assert_eq!(model, "gpt-6-luna");
    assert_eq!((stats.attempts, stats.requests, stats.questions), (0, 0, 1));
    assert!(stats.usage_complete);
}

#[tokio::test]
async fn request_and_response_byte_limits_bound_the_call() {
    let server = MockServer::start().await;
    mount(&server, answer_all).await;
    let limits = ExecutionLimits::default();
    let tight = client(&server).with_limits(ExecutionLimits {
        max_request_bytes: 64,
        ..limits
    });
    let (_, stats) = error(
        tight.evaluate(ticket(), DEFAULT_MODEL).await,
        "payload_too_large",
    );
    assert_eq!(stats.attempts, 0);
    let small = client(&server).with_limits(ExecutionLimits {
        max_response_bytes: 16,
        ..limits
    });
    let (_, stats) = error(
        small.evaluate(ticket(), DEFAULT_MODEL).await,
        "invalid_response",
    );
    assert_eq!((stats.attempts, stats.requests), (1, 0));
}

#[tokio::test]
async fn excessive_timeouts_and_unsupported_models_are_invalid_requests_without_http() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let mut long = ticket();
    long.timeout_ms = 300_001;
    let mut unsupported = ticket();
    unsupported.model = Some("gpt-5.6-luna".into());
    for request in [long, unsupported] {
        let (_, stats) = error(
            client(&server).evaluate(request, DEFAULT_MODEL).await,
            "invalid_request",
        );
        assert_eq!(stats.attempts, 0);
    }
    let (_, stats) = models_error(
        client(&server).list_models(models(300_001)).await,
        "invalid_request",
    );
    assert_eq!(stats.attempts, 0);
}

#[tokio::test]
async fn expired_work_is_a_deadline_without_http() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;
    let mut expired = ticket();
    expired.expires_at_unix_ms = Some(now_ms() - 1);
    let (_, stats) = error(
        client(&server).evaluate(expired, DEFAULT_MODEL).await,
        "deadline",
    );
    assert_eq!(stats.attempts, 0);
    let mut listing = models(1000);
    listing.expires_at_unix_ms = Some(now_ms() - 1);
    let (_, stats) = models_error(client(&server).list_models(listing).await, "deadline");
    assert_eq!(stats.attempts, 0);
}

#[tokio::test]
async fn auth_failures_keep_only_the_error_type_and_code_for_both_operations() {
    for (status, body, detail) in [
        (
            401,
            json!({"error": {"message": "Incorrect API key provided: sk-proj-****abcd", "type": "invalid_request_error", "param": null, "code": "invalid_api_key"}}).to_string(),
            json!({"error": {"type": "invalid_request_error", "code": "invalid_api_key"}}),
        ),
        (
            403,
            "Country, region, or territory not supported for key sk-****wxyz".to_owned(),
            Value::Null,
        ),
    ] {
        let server = MockServer::start().await;
        for verb in ["POST", "GET"] {
            Mock::given(method(verb))
                .respond_with(ResponseTemplate::new(status).set_body_string(body.clone()))
                .expect(1)
                .mount(&server)
                .await;
        }
        let (evaluated, stats) = error(
            client(&server).evaluate(ticket(), DEFAULT_MODEL).await,
            "http",
        );
        assert_eq!(stats.attempts, 1);
        let (listed, _) = models_error(client(&server).list_models(models(1000)).await, "http");
        for wire in [evaluated, listed] {
            assert_eq!(wire["http_status"], status);
            let error = &wire["provider_error"];
            assert_eq!(error["detail"], detail, "{wire}");
            assert!(error.get("message").is_none(), "{wire}");
            assert_eq!(error["truncated"], false);
            assert!(!wire.to_string().contains("sk-"), "{wire}");
        }
    }
}

#[tokio::test]
async fn a_bad_request_is_final_with_its_diagnostics() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error": {
            "message": "Provide between 2 and 255 choices.", "type": "invalid_request_error",
            "param": "questions[0].choices", "code": null}})))
        .expect(1)
        .mount(&server)
        .await;
    let (wire, stats) = error(
        client(&server)
            .with_retry(DEFAULT_RETRY)
            .evaluate(ticket(), DEFAULT_MODEL)
            .await,
        "http",
    );
    assert_eq!(wire["http_status"], 400);
    assert_eq!(
        wire["provider_error"]["detail"]["error"]["param"],
        "questions[0].choices"
    );
    assert_eq!(stats.attempts, 1);
}

#[tokio::test]
async fn rate_limits_honor_retry_after_ms_for_three_attempts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("retry-after-ms", "20")
                .set_body_json(json!({"error": {"type": "rate_limit_error", "code": "slow_down"}})),
        )
        .expect(3)
        .mount(&server)
        .await;
    let (wire, stats) = error(
        client(&server)
            .with_retry(DEFAULT_RETRY)
            .evaluate(ticket(), DEFAULT_MODEL)
            .await,
        "http",
    );
    assert_eq!(wire["http_status"], 429);
    assert_eq!(wire["retry_after_ms"], 20);
    assert_eq!(stats.attempts, 3);
}

#[tokio::test]
async fn billing_rate_limits_are_final() {
    for error_body in [
        json!({"type": "insufficient_quota", "code": "insufficient_quota"}),
        json!({"type": "invalid_request_error", "code": "organization_spend_limit_exceeded"}),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(429)
                    .insert_header("retry-after-ms", "10")
                    .set_body_json(json!({"error": error_body})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let (wire, stats) = error(
            client(&server)
                .with_retry(DEFAULT_RETRY)
                .evaluate(ticket(), DEFAULT_MODEL)
                .await,
            "http",
        );
        assert_eq!(wire["http_status"], 429);
        assert_eq!(stats.attempts, 1);
    }
}

#[tokio::test]
async fn server_errors_are_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503).set_body_json(
            json!({"error": {"type": "service_unavailable_error", "code": "server_is_overloaded"}}),
        ))
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    mount(&server, answer_all).await;
    let (_, _, stats) = ok(client(&server)
        .with_retry(fast(2))
        .evaluate(ticket(), DEFAULT_MODEL)
        .await);
    assert_eq!((stats.attempts, stats.requests), (2, 1));
    // An HTTP error status resolves the attempt: no unknown usage.
    assert!(stats.usage_complete);
}

#[tokio::test]
async fn a_retry_hint_beyond_the_deadline_returns_http_at_once() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after-ms", "5000"))
        .expect(1)
        .mount(&server)
        .await;
    let mut short = ticket();
    short.timeout_ms = 1000;
    let started = Instant::now();
    let (wire, stats) = error(
        client(&server)
            .with_retry(DEFAULT_RETRY)
            .evaluate(short, DEFAULT_MODEL)
            .await,
        "http",
    );
    assert!(started.elapsed() < Duration::from_millis(900));
    assert_eq!(wire["http_status"], 429);
    assert_eq!(wire["retry_after_ms"], 5000);
    assert_eq!(stats.attempts, 1);
}

#[tokio::test]
async fn four_permits_are_shared_and_queueing_counts_against_the_deadline() {
    let server = MockServer::start().await;
    mount(&server, |request: &Request| {
        answer_all(request).set_delay(Duration::from_millis(400))
    })
    .await;
    let shared = client(&server).with_retry(fast(2));
    let mut batch = tickets(8);
    batch.timeout_ms = 10_000;
    let running = tokio::spawn({
        let shared = shared.clone();
        async move { shared.evaluate(batch, DEFAULT_MODEL).await }
    });
    timeout(Duration::from_secs(3), async {
        while server.received_requests().await.unwrap().len() < 4 {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("four requests start");
    // Model listing and another configuration snapshot wait for the same permits.
    let (_, stats) = models_error(shared.list_models(models(100)).await, "deadline");
    assert_eq!(stats.attempts, 0);
    let mut queued = ticket();
    queued.timeout_ms = 100;
    let (_, stats) = error(
        shared
            .with_api_key(Some("snapshot-key"))
            .evaluate(queued, DEFAULT_MODEL)
            .await,
        "deadline",
    );
    assert_eq!(stats.attempts, 0);
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
    let (_, results, stats) = ok(running.await.unwrap());
    assert_eq!(results.as_object().unwrap().len(), 8);
    assert_eq!((stats.attempts, stats.requests), (8, 8));
    assert_eq!(server.received_requests().await.unwrap().len(), 8);
}

/// Headers promise a body that never finishes arriving.
async fn stalled_body() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/decisions", listener.local_addr().unwrap());
    tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut bytes = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            let count = stream.read(&mut buffer).await.unwrap();
            bytes.extend_from_slice(&buffer[..count]);
            if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&bytes[..end]).to_ascii_lowercase();
                let length: usize = headers
                    .lines()
                    .find_map(|line| line.strip_prefix("content-length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if bytes.len() >= end + 4 + length {
                    break;
                }
            }
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 10000\r\n\r\n{")
            .await
            .unwrap();
        let _ = stream.read_to_end(&mut Vec::new()).await;
    });
    endpoint
}

#[tokio::test]
async fn a_stalled_body_hits_the_whole_call_deadline() {
    let endpoint = stalled_body().await;
    let mut short = ticket();
    short.timeout_ms = 300;
    let started = Instant::now();
    let (_, stats) = error(
        DecisionsClient::with_endpoint(Some("local-test-key".into()), endpoint)
            .with_retry(fast(0))
            .evaluate(short, DEFAULT_MODEL)
            .await,
        "deadline",
    );
    assert!(started.elapsed() < Duration::from_secs(2));
    assert_eq!((stats.attempts, stats.requests), (1, 0));
    assert!(!stats.usage_complete);
}

#[tokio::test]
async fn cancellation_is_isolated_between_callers() {
    let server = MockServer::start().await;
    mount(&server, |request: &Request| {
        answer_all(request).set_delay(Duration::from_secs(30))
    })
    .await;
    let owner = client(&server).with_caller_id(Some("owner"));
    let other = owner.with_caller_id(Some("other"));
    let mut identified = ticket();
    identified.request_id = Some("ticket-run-42".into());
    let running = tokio::spawn({
        let owner = owner.clone();
        async move { owner.evaluate(identified, DEFAULT_MODEL).await }
    });
    timeout(Duration::from_secs(3), async {
        while server.received_requests().await.unwrap().is_empty() {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the identified call starts HTTP");
    let cancel = || CancelRequest {
        request_id: "ticket-run-42".into(),
    };
    assert_eq!(
        serde_json::to_value(other.cancel(cancel())).unwrap(),
        json!({"status": "ok", "cancelled": false})
    );
    assert!(!running.is_finished());
    assert_eq!(
        serde_json::to_value(owner.cancel(cancel())).unwrap(),
        json!({"status": "ok", "cancelled": true})
    );
    let response = timeout(Duration::from_secs(2), running)
        .await
        .expect("cancellation interrupts the pending response")
        .unwrap();
    let (_, stats) = error(response, "cancelled");
    assert_eq!((stats.attempts, stats.requests), (1, 0));
    assert!(!stats.usage_complete);
    assert_eq!(
        serde_json::to_value(owner.cancel(cancel())).unwrap(),
        json!({"status": "ok", "cancelled": false})
    );
}

#[tokio::test]
async fn a_refused_evaluation_fails_the_call_but_keeps_validated_usage_only() {
    let server = MockServer::start().await;
    mount(&server, |request: &Request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        if body["input"] == "refuse me" {
            // Answered after the valid evaluation has been accepted.
            ResponseTemplate::new(200)
                .set_body_json(json!({"model": "gpt-6-luna",
                    "answers": [{"type": "refusal", "name": "urgent"}],
                    "usage": {"input_tokens": 900, "output_tokens": 0}}))
                .set_delay(Duration::from_millis(300))
        } else {
            answer_all(request)
        }
    })
    .await;
    let batch = request(json!({"timeout_ms": 3000, "evaluations": [
        {"id": "valid", "state": "fine", "questions": {"urgent": {"type": "noul"}}},
        {"id": "refused", "state": "refuse me", "questions": {"urgent": {"type": "noul"}}}
    ]}));
    let (wire, stats) = error(
        client(&server).evaluate(batch, DEFAULT_MODEL).await,
        "invalid_response",
    );
    assert_eq!(
        wire["provider_error"],
        json!({
            "message": "OpenAI refused 1 question(s) in evaluation refused",
            "detail": {"refused": ["urgent"], "usage": {"input_tokens": 900, "output_tokens": 0}},
            "truncated": false
        })
    );
    assert_eq!(
        (stats.attempts, stats.requests, stats.input_tokens),
        (2, 1, 11)
    );
    assert!(!stats.usage_complete);
}

#[tokio::test]
async fn diverging_model_echoes_are_invalid_and_keep_the_first_usage() {
    let server = MockServer::start().await;
    mount(&server, |request: &Request| {
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        if body["input"] == "later" {
            ResponseTemplate::new(200)
                .set_body_json(decision(request, "gpt-6-luna-2026-10-06"))
                .set_delay(Duration::from_millis(200))
        } else {
            answer_all(request)
        }
    })
    .await;
    let batch = request(json!({"timeout_ms": 3000, "evaluations": [
        {"id": "first", "state": "first", "questions": {"urgent": {"type": "noul"}}},
        {"id": "later", "state": "later", "questions": {"urgent": {"type": "noul"}}}
    ]}));
    let (_, stats) = error(
        client(&server).evaluate(batch, DEFAULT_MODEL).await,
        "invalid_response",
    );
    assert_eq!((stats.requests, stats.input_tokens), (1, 11));
}

#[tokio::test]
async fn model_listing_filters_the_catalog_and_may_be_empty() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .and(header("authorization", "Bearer local-test-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"object": "list", "data": [
            {"id": "gpt-6-luna", "object": "model", "created": 1789406102, "owned_by": "system", "shutdown_date": null},
            {"id": "gpt-5.6-luna", "object": "model", "created": 1780000000, "owned_by": "system"},
            {"id": "gpt-6-astra", "object": "model", "created": 1789000000, "owned_by": "system"}
        ]})))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/models"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [
            {"id": "gpt-5.6-luna", "created": 1780000000}]})))
        .mount(&server)
        .await;
    let response = client(&server).list_models(models(1000)).await;
    let ModelsResponse::Ok {
        models: cards,
        stats,
    } = response
    else {
        panic!("{response:?}");
    };
    assert_eq!(
        serde_json::to_value(cards).unwrap(),
        json!([{"name": "gpt-6-luna", "description": "OpenAI Decisions (beta)",
                "release_date": "2026-09-14", "context_window": 922000}])
    );
    assert_eq!((stats.attempts, stats.requests), (1, 1));
    assert_eq!((stats.input_tokens, stats.output_tokens), (0, 0));
    let empty = serde_json::to_value(client(&server).list_models(models(1000)).await).unwrap();
    assert_eq!(empty["status"], "ok");
    assert_eq!(empty["models"], json!([]));
    assert!(server.received_requests().await.unwrap()[0].body.is_empty());
}

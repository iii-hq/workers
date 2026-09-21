use judge_contract::{
    Answer, EvaluateRequest, EvaluateResponse, Evaluation, ModelsRequest, ModelsResponse, Question,
    Stats,
};
use judge_typesafe::{ExecutionLimits, JevClient, DEFAULT_MODEL};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
    time::{sleep, timeout},
};

struct Reply {
    status: u16,
    body: String,
    delay_ms: u64,
    slow_body: bool,
}
impl Reply {
    fn ok(body: Value) -> Self {
        Self {
            status: 200,
            body: body.to_string(),
            delay_ms: 0,
            slow_body: false,
        }
    }
}
#[derive(Default)]
struct Observed {
    requests: Mutex<Vec<(Value, String)>>,
    active: AtomicUsize,
    peak: AtomicUsize,
}
struct Server {
    endpoint: String,
    observed: Arc<Observed>,
    task: JoinHandle<()>,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Server {
    async fn start(reply: impl Fn(&Value) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}/evaluate", listener.local_addr().unwrap());
        let observed = Arc::new(Observed::default());
        let state = observed.clone();
        let reply = Arc::new(reply);
        let task = tokio::spawn(async move {
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let (mut stream, _) = accepted.unwrap();
                        let state = state.clone(); let reply = reply.clone();
                        connections.spawn(async move {
                            let mut bytes = Vec::new(); let mut buf = [0; 4096];
                            let (headers, body_start, length) = loop {
                                let n = stream.read(&mut buf).await.unwrap(); if n == 0 { return; }
                                bytes.extend_from_slice(&buf[..n]);
                                if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                                    let headers = String::from_utf8(bytes[..end].to_vec()).unwrap().to_lowercase();
                                    let len = headers.lines().find_map(|line| line.strip_prefix("content-length: ")).unwrap_or("0").parse::<usize>().unwrap();
                                    break (headers, end + 4, len);
                                }
                            };
                            while bytes.len() < body_start + length {
                                let n = stream.read(&mut buf).await.unwrap(); if n == 0 { return; }
                                bytes.extend_from_slice(&buf[..n]);
                            }
                            let body: Value = if length == 0 { Value::Null } else { serde_json::from_slice(&bytes[body_start..body_start + length]).unwrap() };
                            state.requests.lock().unwrap().push((body.clone(), headers));
                            let active = state.active.fetch_add(1, Ordering::SeqCst) + 1;
                            state.peak.fetch_max(active, Ordering::SeqCst);
                            let response = reply(&body);
                            if !response.slow_body { sleep(Duration::from_millis(response.delay_ms)).await; }
                            let headers = format!("HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", response.status, response.body.len());
                            let _ = stream.write_all(headers.as_bytes()).await;
                            if response.slow_body { sleep(Duration::from_millis(response.delay_ms)).await; }
                            let _ = stream.write_all(response.body.as_bytes()).await;
                            state.active.fetch_sub(1, Ordering::SeqCst);
                        });
                    }
                    _ = connections.join_next(), if !connections.is_empty() => {}
                }
            }
        });
        Self {
            endpoint,
            observed,
            task,
        }
    }
    fn client(&self) -> JevClient {
        JevClient::with_endpoint(Some("test-credential".into()), self.endpoint.clone())
    }
    fn count(&self) -> usize {
        self.observed.requests.lock().unwrap().len()
    }
    async fn wait_for(&self, count: usize) {
        timeout(Duration::from_secs(3), async {
            while self.count() < count {
                sleep(Duration::from_millis(2)).await;
            }
        })
        .await
        .expect("upstream requests arrive");
    }
}
fn request(count: usize) -> EvaluateRequest {
    EvaluateRequest {
        options: judge_contract::RequestOptions {
            retry: judge_contract::RetryPolicy {
                max_retries: 0,
                ..Default::default()
            },
            ..Default::default()
        },
        request_id: None,
        model: None,
        timeout_ms: 3000,
        expires_at_unix_ms: None,
        evaluations: (0..count)
            .map(|i| Evaluation {
                id: format!("ticket-{i}"),
                state: json!({"ticket": "Customer cannot sign in", "index": i}),
                questions: BTreeMap::from([(
                    "urgent".into(),
                    Question::Noul {
                        instructions: "Does this ticket need urgent support?".into(),
                        criteria: None,
                    },
                )]),
            })
            .collect(),
    }
}
fn answer(body: &Value) -> Value {
    let answers: BTreeMap<_, _> = body["questions"]
        .as_object()
        .unwrap()
        .keys()
        .map(|key| (key, json!({"type":"noul","noul":0.75})))
        .collect();
    json!({"model":body["model"],"answers":answers,"usage":{"input_tokens":11,"output_tokens":3}})
}
fn ok(
    response: EvaluateResponse,
) -> (
    String,
    BTreeMap<String, judge_contract::EvaluationResult>,
    Stats,
) {
    match response {
        EvaluateResponse::Ok {
            model,
            results,
            stats,
        } => (model, results, stats),
        other => panic!("expected success, got {other:?}"),
    }
}
fn error(response: EvaluateResponse, code: &str) -> Stats {
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(wire["status"], "error");
    assert_eq!(wire["code"], code);
    assert!(wire.get("results").is_none());
    match response {
        EvaluateResponse::Error { stats, .. } => stats,
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn mixed_wire_questions_preserve_structured_descriptions_and_typed_answers() {
    let wire = json!({"timeout_ms":60000,"evaluations":[{"id":"ticket","state":{"body":"Invoice question"},"questions":{
        "department":{"type":"choice","criteria":{"billing":null,"support":{"scope":"bugs"}}},
        "severity":{"type":"score","instructions":["Assess impact"],"criteria":["routine",{"impact":"blocking"}]},
        "urgent":{"type":"noul","instructions":null,"criteria":{"true":["urgent"]}}
    }}]});
    let parsed = serde_json::from_value::<EvaluateRequest>(wire.clone());
    assert!(
        parsed.is_ok(),
        "documented mixed request must decode: {parsed:?}"
    );
    let answers = json!({
        "department":{"type":"choice","choice":"billing","probabilities":{"billing":0.667,"support":0.333},"confidence":0.4},
        "severity":{"type":"score","score":0.8,"probabilities":{"0":0.2,"1":0.8},"confidence":0.6,"legend":{"0":"routine","1":{"impact":"blocking"}}},
        "urgent":{"type":"noul","noul":0.75}
    });
    let expected = answers.clone();
    let server = Server::start(move |_| Reply::ok(json!({"model":"jev-1.13.0","answers":answers,"usage":{"input_tokens":11,"output_tokens":3}}))).await;
    let (_, results, stats) = ok(server
        .client()
        .evaluate(parsed.unwrap(), DEFAULT_MODEL)
        .await);
    assert_eq!(
        serde_json::to_value(&results["ticket"].answers).unwrap(),
        expected
    );
    assert_eq!((stats.attempts, stats.requests, stats.questions), (1, 1, 3));
    assert!(stats.usage_complete);
    let seen = server.observed.requests.lock().unwrap();
    assert_eq!(
        seen[0].0["questions"]["severity"],
        wire["evaluations"][0]["questions"]["severity"]
    );
    assert_eq!(
        seen[0].0["questions"]["urgent"]["criteria"],
        json!({"true":["urgent"]})
    );
}

#[tokio::test]
async fn independent_ticket_evaluation_uses_generic_wire_and_counts_known_usage() {
    let server = Server::start(|body| Reply::ok(answer(body))).await;
    let (model, results, stats) = ok(server.client().evaluate(request(1), DEFAULT_MODEL).await);
    assert_eq!(model, "jev-1.13.0");
    assert!(matches!(
        results["ticket-0"].answers["urgent"],
        Answer::Noul { noul: 0.75 }
    ));
    assert_eq!(
        (
            stats.attempts,
            stats.requests,
            stats.questions,
            stats.input_tokens,
            stats.output_tokens
        ),
        (1, 1, 1, 11, 3)
    );
    assert!(stats.usage_complete);
    let observed = server.observed.requests.lock().unwrap();
    let (body, headers) = &observed[0];
    assert_eq!(body.as_object().unwrap().len(), 3);
    assert_eq!(body["state"]["ticket"], "Customer cannot sign in");
    assert!(body["questions"]["urgent"].get("criteria").is_none());
    assert!(headers.contains("authorization: bearer test-credential"));
}
#[tokio::test]
async fn criteria_are_optional_and_preserved_when_supplied() {
    let server = Server::start(|body| Reply::ok(answer(body))).await;
    let mut input = request(1);
    input.evaluations[0].questions.insert(
        "urgent".into(),
        Question::Noul {
            instructions: "Does this ticket need urgent support?".into(),
            criteria: Some(BTreeMap::from([
                ("true".into(), "Account is blocked".into()),
                ("false".into(), "Routine inquiry".into()),
            ])),
        },
    );
    ok(server.client().evaluate(input, DEFAULT_MODEL).await);
    assert_eq!(
        server.observed.requests.lock().unwrap()[0].0["questions"]["urgent"]["criteria"]["true"],
        "Account is blocked"
    );
}
#[tokio::test]
async fn missing_or_blank_key_sends_no_http() {
    let server = Server::start(|b| Reply::ok(answer(b))).await;
    for key in [None, Some("  \n".into())] {
        let stats = error(
            JevClient::with_endpoint(key, server.endpoint.clone())
                .evaluate(request(1), DEFAULT_MODEL)
                .await,
            "missing_key",
        );
        assert_eq!(stats.attempts, 0);
        assert!(!stats.usage_complete);
    }
    assert_eq!(server.count(), 0);
}
#[tokio::test]
async fn invalid_mixed_batch_and_oversized_batch_send_zero_http() {
    let server = Server::start(|b| Reply::ok(answer(b))).await;
    let mut invalid = request(2);
    invalid.evaluations[1].questions.insert(
        "urgent".into(),
        Question::Noul {
            instructions: "Is this urgent?".into(),
            criteria: Some(BTreeMap::from([(
                "other".into(),
                "Not a Noul outcome".into(),
            )])),
        },
    );
    assert_eq!(
        error(
            server.client().evaluate(invalid, DEFAULT_MODEL).await,
            "invalid_request"
        )
        .attempts,
        0
    );
    let mut large = request(2);
    large.evaluations[1].state = json!("x".repeat(17 * 1024));
    assert_eq!(
        error(
            server
                .client()
                .with_limits(ExecutionLimits {
                    max_request_bytes: 16 * 1024,
                    ..ExecutionLimits::default()
                })
                .evaluate(large, DEFAULT_MODEL)
                .await,
            "payload_too_large"
        )
        .attempts,
        0
    );
    error(
        server.client().evaluate(request(513), DEFAULT_MODEL).await,
        "invalid_request",
    );
    assert_eq!(server.count(), 0);
}
#[tokio::test]
async fn invalid_answer_sets_types_ranges_and_models_are_atomic_failures() {
    for replacement in [
        json!({}),
        json!({"other":{"type":"noul","noul":0.5}}),
        json!({"urgent":{"type":"bool","noul":0.5}}),
        json!({"urgent":{"type":"noul","noul":-0.1}}),
        json!({"urgent":{"type":"noul","noul":1.01}}),
        json!({"urgent":{"type":"noul","noul":"NaN"}}),
    ] {
        let server = Server::start(move |b| {
            let mut v = answer(b);
            v["answers"] = replacement.clone();
            Reply::ok(v)
        })
        .await;
        let stats = error(
            server.client().evaluate(request(1), DEFAULT_MODEL).await,
            "invalid_response",
        );
        assert_eq!(
            (stats.attempts, stats.requests, stats.input_tokens),
            (1, 0, 0)
        );
        assert!(!stats.usage_complete);
    }
    let server = Server::start(|b| {
        let mut v = answer(b);
        v["model"] = json!("");
        Reply::ok(v)
    })
    .await;
    error(
        server.client().evaluate(request(1), DEFAULT_MODEL).await,
        "invalid_response",
    );
}
#[tokio::test]
async fn inconsistent_response_models_discard_results_but_preserve_accepted_usage() {
    let server = Server::start(|b| {
        let mut v = answer(b);
        v["model"] = json!(format!("resolved-{}", b["state"]["index"]));
        let mut r = Reply::ok(v);
        r.delay_ms = b["state"]["index"].as_u64().unwrap() * 70;
        r
    })
    .await;
    let stats = error(
        server.client().evaluate(request(2), DEFAULT_MODEL).await,
        "invalid_response",
    );
    assert_eq!(
        (
            stats.attempts,
            stats.requests,
            stats.questions,
            stats.input_tokens,
            stats.output_tokens
        ),
        (2, 1, 1, 11, 3)
    );
    assert!(!stats.usage_complete);
}
#[tokio::test]
async fn http_errors_preserve_diagnostics_with_retries_disabled_and_no_redirects() {
    for status in [301, 401, 429, 500, 529] {
        let server = Server::start(move |_| Reply {
            status,
            body: "untrusted-provider-body".into(),
            delay_ms: 0,
            slow_body: false,
        })
        .await;
        let response = server.client().evaluate(request(1), DEFAULT_MODEL).await;
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(wire["http_status"], status);
        assert_eq!(wire["provider_error"]["message"], "untrusted-provider-body");
        let stats = error(response, "http");
        assert_eq!((stats.attempts, stats.requests), (1, 0));
        assert!(!stats.usage_complete);
        assert_eq!(server.count(), 1);
    }
}
#[tokio::test]
async fn slow_body_hits_whole_call_deadline() {
    let server = Server::start(|b| {
        let mut r = Reply::ok(answer(b));
        r.delay_ms = 350;
        r.slow_body = true;
        r
    })
    .await;
    let mut input = request(1);
    input.timeout_ms = 70;
    let stats = error(
        server.client().evaluate(input, DEFAULT_MODEL).await,
        "deadline",
    );
    assert_eq!((stats.attempts, stats.requests), (1, 0));
    assert!(stats.elapsed_ms >= 60 && stats.elapsed_ms < 300);
}
#[tokio::test]
async fn oversized_response_is_bounded_and_invalid() {
    let server = Server::start(|body| {
        let mut response = answer(body);
        response["metadata"] = json!("x".repeat(1024 * 1024 + 1));
        Reply::ok(response)
    })
    .await;
    error(
        server
            .client()
            .with_limits(ExecutionLimits {
                max_response_bytes: 1024 * 1024,
                ..ExecutionLimits::default()
            })
            .evaluate(request(1), DEFAULT_MODEL)
            .await,
        "invalid_response",
    );
}
#[tokio::test]
async fn expired_work_and_expiry_during_queue_never_send_http() {
    let server = Server::start(|b| {
        let mut r = Reply::ok(answer(b));
        r.delay_ms = 250;
        r
    })
    .await;
    let client = server.client();
    let mut expired = request(1);
    expired.expires_at_unix_ms = Some(1);
    assert_eq!(
        error(client.evaluate(expired, DEFAULT_MODEL).await, "deadline").attempts,
        0
    );
    assert_eq!(server.count(), 0);
    let running_client = client.clone();
    let running =
        tokio::spawn(async move { running_client.evaluate(request(4), DEFAULT_MODEL).await });
    server.wait_for(4).await;
    let mut queued = request(1);
    queued.expires_at_unix_ms = Some(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 50,
    );
    assert_eq!(
        error(client.evaluate(queued, DEFAULT_MODEL).await, "deadline").attempts,
        0
    );
    ok(running.await.unwrap());
    assert_eq!(server.count(), 4);
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_callers_share_four_permits_including_complete_body_read() {
    let server = Server::start(|b| {
        let mut r = Reply::ok(answer(b));
        r.delay_ms = 40;
        r.slow_body = true;
        r
    })
    .await;
    let first = server.client();
    let second = first.with_api_key(Some("rotated-test-credential"));
    let (left, right) = tokio::join!(
        first.evaluate(request(8), DEFAULT_MODEL),
        second.evaluate(request(8), DEFAULT_MODEL)
    );
    assert_eq!(ok(left).2.requests, 8);
    assert_eq!(ok(right).2.requests, 8);
    assert_eq!(server.observed.peak.load(Ordering::SeqCst), 4);
}
#[tokio::test]
async fn known_usage_survives_atomic_failure_and_cancels_unsent_work() {
    let server = Server::start(|b| {
        let i = b["state"]["index"].as_u64().unwrap();
        let mut r = Reply::ok(answer(b));
        if i == 1 {
            r.status = 500;
            r.delay_ms = 70;
        } else if i > 1 {
            r.delay_ms = 500;
        }
        r
    })
    .await;
    let stats = error(
        server.client().evaluate(request(12), DEFAULT_MODEL).await,
        "http",
    );
    assert_eq!(
        (
            stats.requests,
            stats.questions,
            stats.input_tokens,
            stats.output_tokens
        ),
        (1, 1, 11, 3)
    );
    assert!(!stats.usage_complete);
    assert!(stats.attempts <= 5);
    sleep(Duration::from_millis(100)).await;
    assert!(server.count() <= 5);
}
#[tokio::test]
async fn missing_usage_object_and_malformed_counters_are_invalid() {
    let server = Server::start(|b| {
        let mut v = answer(b);
        v.as_object_mut().unwrap().remove("usage");
        Reply::ok(v)
    })
    .await;
    let stats = error(
        server.client().evaluate(request(1), DEFAULT_MODEL).await,
        "invalid_response",
    );
    assert_eq!(
        (stats.requests, stats.input_tokens, stats.output_tokens),
        (0, 0, 0)
    );
    assert!(!stats.usage_complete);
    for usage in [
        json!({"input_tokens":-1,"output_tokens":0}),
        json!({"input_tokens":3,"output_tokens":"unknown"}),
        json!({"input_tokens":1.5,"output_tokens":0}),
    ] {
        let server = Server::start(move |b| {
            let mut v = answer(b);
            v["usage"] = usage.clone();
            Reply::ok(v)
        })
        .await;
        error(
            server.client().evaluate(request(1), DEFAULT_MODEL).await,
            "invalid_response",
        );
    }
}
#[tokio::test]
async fn configuration_snapshots_override_boot_key_without_mutating_existing_calls() {
    let server = Server::start(|b| Reply::ok(answer(b))).await;
    let boot = server.client();
    let first = boot.with_api_key(Some("first-test-credential"));
    let second = boot.with_api_key(Some("second-test-credential"));
    for client in [first.clone(), second, first, boot.with_api_key(None)] {
        ok(client.evaluate(request(1), DEFAULT_MODEL).await);
    }
    let seen = server.observed.requests.lock().unwrap();
    for (i, key) in [
        "first-test-credential",
        "second-test-credential",
        "first-test-credential",
        "test-credential",
    ]
    .iter()
    .enumerate()
    {
        assert!(seen[i].1.contains(&format!("authorization: bearer {key}")));
    }
}
#[tokio::test]
async fn closed_endpoint_is_transport_failure_with_an_attempt() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let stats = error(
        JevClient::with_endpoint(Some("test-credential".into()), endpoint)
            .evaluate(request(1), DEFAULT_MODEL)
            .await,
        "transport",
    );
    assert_eq!((stats.attempts, stats.requests), (1, 0));
    assert!(!stats.usage_complete);
}

#[tokio::test]
async fn request_timeout_validation_remains_distinct_from_expired_work() {
    let server = Server::start(|body| Reply::ok(answer(body))).await;
    for timeout_ms in [0, 300_001, u64::MAX] {
        let mut input = request(1);
        input.timeout_ms = timeout_ms;
        let stats = error(
            server.client().evaluate(input, DEFAULT_MODEL).await,
            "invalid_request",
        );
        assert_eq!(stats.attempts, 0);
    }
    assert_eq!(server.count(), 0);
}
#[tokio::test]
async fn duplicate_answer_ids_are_invalid_instead_of_silently_overwritten() {
    let server = Server::start(|_| Reply { status: 200, delay_ms: 0, slow_body: false, body: r#"{"model":"jev-1.13.0","answers":{"urgent":{"type":"noul","noul":0.1},"urgent":{"type":"noul","noul":0.8}},"usage":{"input_tokens":11,"output_tokens":3}}"#.into() }).await;
    error(
        server.client().evaluate(request(1), DEFAULT_MODEL).await,
        "invalid_response",
    );
}
#[tokio::test]
async fn valid_large_question_set_accepts_response_over_request_byte_budget() {
    let server = Server::start(|body| {
        let mut result = answer(body);
        result["metadata"] = json!("x".repeat(60 * 1024));
        Reply::ok(result)
    })
    .await;
    let mut input = request(1);
    input.evaluations[0].questions = (0..600)
        .map(|i| {
            (
                format!("q{i}"),
                Question::Noul {
                    instructions: "Evaluate.".into(),
                    criteria: None,
                },
            )
        })
        .collect();
    let (_, results, stats) = ok(server.client().evaluate(input, DEFAULT_MODEL).await);
    assert_eq!(results["ticket-0"].answers.len(), 600);
    assert_eq!(stats.questions, 600);
    assert!(stats.usage_complete);
}
#[tokio::test]
async fn token_overflow_cannot_wrap_accepted_usage() {
    let server = Server::start(|body| {
        let mut result = answer(body);
        result["usage"]["input_tokens"] = json!(u64::MAX);
        let mut r = Reply::ok(result);
        r.delay_ms = body["state"]["index"].as_u64().unwrap() * 50;
        r
    })
    .await;
    let stats = error(
        server.client().evaluate(request(2), DEFAULT_MODEL).await,
        "invalid_response",
    );
    assert_eq!(stats.input_tokens, u64::MAX);
    assert_eq!(stats.requests, 1);
    assert!(!stats.usage_complete);
}

#[tokio::test]
async fn partial_or_null_usage_keeps_answers_and_marks_every_missing_counter() {
    for (usage, input, output, complete) in [
        (json!({}), 0, 0, false),
        (json!({"input_tokens":11}), 11, 0, false),
        (json!({"output_tokens":3}), 0, 3, false),
        (json!({"input_tokens":null,"output_tokens":3}), 0, 3, false),
        (
            json!({"input_tokens":11,"output_tokens":null}),
            11,
            0,
            false,
        ),
        (
            json!({"input_tokens":null,"output_tokens":null}),
            0,
            0,
            false,
        ),
        (json!({"input_tokens":0,"output_tokens":0}), 0, 0, true),
    ] {
        let server = Server::start(move |body| {
            let mut response = answer(body);
            response["usage"] = usage.clone();
            Reply::ok(response)
        })
        .await;
        let (_, results, stats) = ok(server.client().evaluate(request(1), DEFAULT_MODEL).await);
        assert_eq!(results.len(), 1);
        assert_eq!(
            (
                stats.requests,
                stats.questions,
                stats.input_tokens,
                stats.output_tokens,
                stats.usage_complete
            ),
            (1, 1, input, output, complete)
        );
    }
}

fn models_request(timeout_ms: u64) -> ModelsRequest {
    serde_json::from_value(json!({"timeout_ms":timeout_ms,"options":{"retry":{"max_retries":0}}}))
        .unwrap()
}
fn model_cards() -> Value {
    json!({"models":[
        {"name":"jev-latest","description":"Current stable alias","release_date":"2026-01-01"},
        {"name":"future-account-model","description":"An account-specific model","release_date":"2026-09-19"}
    ]})
}
fn models_ok(response: ModelsResponse) -> (Vec<judge_contract::ModelCard>, Stats) {
    match response {
        ModelsResponse::Ok { models, stats } => (models, stats),
        other => panic!("expected models, got {other:?}"),
    }
}
fn models_error(response: ModelsResponse, code: &str) -> Stats {
    let wire = serde_json::to_value(&response).unwrap();
    assert_eq!(wire["status"], "error");
    assert_eq!(wire["code"], code);
    assert!(wire.get("models").is_none());
    match response {
        ModelsResponse::Error { stats, .. } => stats,
        _ => unreachable!(),
    }
}

#[tokio::test]
async fn models_get_uses_origin_credentials_and_returns_unfiltered_cards_without_inference() {
    let server = Server::start(|_| Reply::ok(model_cards())).await;
    let endpoint = server
        .endpoint
        .replace("/evaluate", "/v1/systemone?test=ignored");
    let client = JevClient::with_endpoint(Some("boot-test-key".into()), endpoint);
    let request = serde_json::from_value(json!({})).unwrap();
    let (models, stats) = models_ok(
        client
            .with_api_key(Some("snapshot-test-key"))
            .list_models(request)
            .await,
    );
    assert_eq!(
        serde_json::to_value(models).unwrap(),
        model_cards()["models"]
    );
    assert_eq!(
        (
            stats.attempts,
            stats.requests,
            stats.questions,
            stats.input_tokens,
            stats.output_tokens
        ),
        (1, 1, 0, 0, 0)
    );
    assert!(stats.usage_complete);
    let seen = server.observed.requests.lock().unwrap();
    assert!(seen[0].0.is_null());
    assert!(seen[0].1.starts_with("get /v1/models http/1.1\r\n"));
    assert!(seen[0]
        .1
        .contains("authorization: bearer snapshot-test-key"));
}

#[tokio::test]
async fn models_failures_are_typed_sanitized_and_not_retried() {
    for status in [301, 401, 429, 500, 529] {
        let server = Server::start(move |_| Reply {
            status,
            body: "untrusted-provider-body".into(),
            delay_ms: 0,
            slow_body: false,
        })
        .await;
        let response = server.client().list_models(models_request(1000)).await;
        let wire = serde_json::to_value(&response).unwrap();
        assert_eq!(wire["http_status"], status);
        assert_eq!(wire["provider_error"]["message"], "untrusted-provider-body");
        let stats = models_error(response, "http");
        assert_eq!((stats.attempts, stats.requests), (1, 0));
        assert!(!stats.usage_complete);
        assert_eq!(server.count(), 1);
    }
    for body in [
        json!({}),
        json!({"models":null}),
        json!({"models":[{"name":"jev-latest"}]}),
        json!({"models":[{"name":7,"description":"bad","release_date":"today"}]}),
    ] {
        let server = Server::start(move |_| Reply::ok(body.clone())).await;
        models_error(
            server.client().list_models(models_request(1000)).await,
            "invalid_response",
        );
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    let stats = models_error(
        JevClient::with_endpoint(Some("test-key".into()), endpoint)
            .list_models(models_request(1000))
            .await,
        "transport",
    );
    assert_eq!((stats.attempts, stats.requests), (1, 0));
}

#[tokio::test]
async fn models_require_key_valid_timeout_and_unexpired_work_before_sending() {
    let server = Server::start(|_| Reply::ok(model_cards())).await;
    for key in [None, Some("  ".into())] {
        let client = JevClient::with_endpoint(key, server.endpoint.clone());
        assert_eq!(
            models_error(
                client.list_models(models_request(1000)).await,
                "missing_key"
            )
            .attempts,
            0
        );
    }
    for timeout_ms in [0, 300001, u64::MAX] {
        assert_eq!(
            models_error(
                server
                    .client()
                    .list_models(models_request(timeout_ms))
                    .await,
                "invalid_request"
            )
            .attempts,
            0
        );
    }
    let mut expired = models_request(1000);
    expired.expires_at_unix_ms = Some(1);
    assert_eq!(
        models_error(server.client().list_models(expired).await, "deadline").attempts,
        0
    );
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn models_body_read_and_permit_wait_share_the_whole_call_deadline() {
    let server = Server::start(|body| {
        let mut reply = Reply::ok(if body.is_null() {
            model_cards()
        } else {
            answer(body)
        });
        reply.slow_body = true;
        reply.delay_ms = 250;
        reply
    })
    .await;
    let client = server.client();
    let running_client = client.clone();
    let running =
        tokio::spawn(async move { running_client.evaluate(request(4), DEFAULT_MODEL).await });
    server.wait_for(4).await;
    let mut queued = models_request(1000);
    queued.expires_at_unix_ms = Some(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
            + 50,
    );
    assert_eq!(
        models_error(client.list_models(queued).await, "deadline").attempts,
        0
    );
    ok(running.await.unwrap());
    assert_eq!(server.count(), 4);
    let stats = models_error(client.list_models(models_request(50)).await, "deadline");
    assert_eq!((stats.attempts, stats.requests), (1, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn models_and_evaluation_snapshots_share_four_permits_until_bodies_complete() {
    let server = Server::start(|body| {
        let mut reply = Reply::ok(if body.is_null() {
            model_cards()
        } else {
            answer(body)
        });
        reply.delay_ms = 40;
        reply.slow_body = true;
        reply
    })
    .await;
    let client = server.client();
    let changed = client
        .with_limits(ExecutionLimits {
            max_timeout_ms: 90000,
            ..ExecutionLimits::default()
        })
        .with_api_key(Some("new-test-key"));
    let (evaluation, models) = tokio::join!(
        client.evaluate(request(8), DEFAULT_MODEL),
        futures_util::future::join_all((0..8).map(|_| changed.list_models(models_request(60000))))
    );
    assert_eq!(ok(evaluation).2.requests, 8);
    for result in models {
        assert_eq!(models_ok(result).1.requests, 1);
    }
    assert_eq!(server.observed.peak.load(Ordering::SeqCst), 4);
    assert_eq!(server.count(), 16);
}

#[tokio::test]
async fn configured_limits_bound_requests_responses_and_timeouts() {
    let server = Server::start(|body| {
        Reply::ok(if body.is_null() {
            model_cards()
        } else {
            answer(body)
        })
    })
    .await;
    let original = server.client();
    let mut large = request(1);
    large.timeout_ms = 60000;
    large.evaluations[0].state = json!("x".repeat(64 * 1024));
    ok(original.evaluate(large.clone(), DEFAULT_MODEL).await);
    let limited = original.with_limits(ExecutionLimits {
        max_request_bytes: 1024,
        max_response_bytes: 64,
        max_timeout_ms: 70000,
    });
    assert_eq!(
        error(
            limited.evaluate(large, DEFAULT_MODEL).await,
            "payload_too_large"
        )
        .attempts,
        0
    );
    error(
        limited.evaluate(request(1), DEFAULT_MODEL).await,
        "invalid_response",
    );
    models_error(
        limited.list_models(models_request(60000)).await,
        "invalid_response",
    );
    assert_eq!(
        models_error(
            limited.list_models(models_request(70001)).await,
            "invalid_request"
        )
        .attempts,
        0
    );
    ok(original.evaluate(request(1), DEFAULT_MODEL).await);
    models_ok(original.list_models(models_request(60000)).await);
}

#[tokio::test]
async fn invalid_execution_limits_fail_without_http_or_panics() {
    let server = Server::start(|body| Reply::ok(answer(body))).await;
    for limits in [
        ExecutionLimits {
            max_request_bytes: 0,
            ..ExecutionLimits::default()
        },
        ExecutionLimits {
            max_response_bytes: 0,
            ..ExecutionLimits::default()
        },
        ExecutionLimits {
            max_timeout_ms: 0,
            ..ExecutionLimits::default()
        },
        ExecutionLimits {
            max_request_bytes: usize::MAX,
            ..ExecutionLimits::default()
        },
        ExecutionLimits {
            max_response_bytes: usize::MAX,
            ..ExecutionLimits::default()
        },
        ExecutionLimits {
            max_timeout_ms: u64::MAX,
            ..ExecutionLimits::default()
        },
    ] {
        let client = server.client().with_limits(limits);
        assert_eq!(
            error(
                client.evaluate(request(1), DEFAULT_MODEL).await,
                "invalid_request"
            )
            .attempts,
            0
        );
        assert_eq!(
            models_error(
                client.list_models(models_request(1000)).await,
                "invalid_request"
            )
            .attempts,
            0
        );
    }
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn one_unknown_usage_counter_keeps_batch_completeness_false_after_known_responses() {
    let server = Server::start(|body| {
        let mut response = answer(body);
        if body["state"]["index"] == 0 {
            response["usage"]["input_tokens"] = Value::Null;
        }
        let mut reply = Reply::ok(response);
        reply.delay_ms = body["state"]["index"].as_u64().unwrap() * 30;
        reply
    })
    .await;
    let (_, results, stats) = ok(server.client().evaluate(request(2), DEFAULT_MODEL).await);
    assert_eq!(results.len(), 2);
    assert_eq!(
        (
            stats.requests,
            stats.questions,
            stats.input_tokens,
            stats.output_tokens
        ),
        (2, 2, 11, 6)
    );
    assert!(!stats.usage_complete);
}

fn mixed_request() -> EvaluateRequest {
    serde_json::from_value(json!({"timeout_ms":1000,"evaluations":[{"id":"ticket","state":["Invoice question"],"questions":{
        "department":{"type":"choice","criteria":{"billing":null,"support":{"scope":"bugs"},"other":["Anything else"]}},
        "severity":{"type":"score","instructions":"Assess impact","criteria":[["negligible"],"routine",{"impact":"blocking"}]},
        "urgent":{"type":"noul"}
    }}]})).unwrap()
}
fn mixed_answers() -> Value {
    json!({
        "department":{"type":"choice","choice":"billing","probabilities":{"billing":0.333,"support":0.333,"other":0.333},"confidence":0.123},
        "severity":{"type":"score","score":1.1,"probabilities":{"0":0.333,"1":0.333,"2":0.333},"confidence":0.432,"legend":{"0":["negligible"],"1":"routine","2":{"impact":"blocking"}}},
        "urgent":{"type":"noul","noul":0.75}
    })
}

#[tokio::test]
async fn rounded_distributions_and_structured_legends_preserve_provider_score_and_confidence() {
    let server = Server::start(|_| {
        Reply::ok(json!({"model":"jev-1.13.0","answers":mixed_answers(),"usage":{}}))
    })
    .await;
    let (_, results, stats) = ok(server
        .client()
        .evaluate(mixed_request(), DEFAULT_MODEL)
        .await);
    assert_eq!(
        serde_json::to_value(&results["ticket"].answers).unwrap(),
        mixed_answers()
    );
    assert!(!stats.usage_complete);
    assert_eq!((stats.requests, stats.questions), (1, 3));
}

#[tokio::test]
async fn mixed_answer_type_membership_ids_confidence_scores_and_distributions_fail_atomically() {
    for (path, replacement) in [
        ("/department", json!({"type":"noul","noul":0.5})),
        ("/severity", mixed_answers()["department"].clone()),
        ("/urgent", mixed_answers()["severity"].clone()),
        ("/department/choice", json!("missing")),
        ("/department/confidence", json!(-0.01)),
        ("/department/confidence", json!(1.01)),
        ("/department/confidence", json!("NaN")),
        (
            "/department/probabilities",
            json!({"billing":0.5,"support":0.5}),
        ),
        (
            "/department/probabilities",
            json!({"billing":0.5,"support":0.25,"other":0.25,"extra":0}),
        ),
        (
            "/department/probabilities",
            json!({"billing":0.6,"support":0.6,"other":0.6}),
        ),
        (
            "/department/probabilities",
            json!({"billing":1.1,"support":-0.1,"other":0}),
        ),
        ("/severity/score", json!(-0.01)),
        ("/severity/score", json!(2.01)),
        ("/severity/score", json!("Infinity")),
        ("/severity/confidence", json!(-0.01)),
        ("/severity/confidence", json!(1.01)),
        ("/severity/probabilities", json!({"0":0.5,"1":0.5})),
        (
            "/severity/probabilities",
            json!({"0":0.5,"1":0.25,"wrong":0.25}),
        ),
        (
            "/severity/probabilities",
            json!({"0":0.5,"1":0.25,"2":0.25,"3":0}),
        ),
        ("/severity/probabilities", json!({"0":0.1,"1":0.1,"2":0.1})),
        ("/severity/probabilities", json!({"0":1.1,"1":-0.1,"2":0})),
        (
            "/severity/legend",
            json!({"0":["negligible"],"1":"routine"}),
        ),
        (
            "/severity/legend",
            json!({"0":["negligible"],"1":"routine","wrong":{"impact":"blocking"}}),
        ),
        (
            "/severity/legend",
            json!({"0":["negligible"],"1":"routine","2":{"impact":"blocking"},"3":"extra"}),
        ),
        ("/severity/legend/0", Value::Null),
        (
            "/severity/legend",
            json!({"0":"routine","1":["negligible"],"2":{"impact":"blocking"}}),
        ),
    ] {
        let server = Server::start(move |_| {
            let mut answers = mixed_answers();
            *answers.pointer_mut(path).unwrap() = replacement.clone();
            Reply::ok(json!({"model":"jev-1.13.0","answers":answers,"usage":{"input_tokens":11,"output_tokens":3}}))
        }).await;
        let stats = error(
            server
                .client()
                .evaluate(mixed_request(), DEFAULT_MODEL)
                .await,
            "invalid_response",
        );
        assert_eq!(
            (
                stats.attempts,
                stats.requests,
                stats.questions,
                stats.input_tokens,
                stats.output_tokens
            ),
            (1, 0, 0, 0, 0),
            "invalid answer at {path}"
        );
        assert!(!stats.usage_complete);
    }
}

#[tokio::test]
async fn content_variants_and_optional_noul_descriptions_reach_the_provider() {
    let server = Server::start(|body| Reply::ok(answer(body))).await;
    for (instructions, criteria) in [
        (json!(null), json!(null)),
        (json!(""), json!({})),
        (json!(" "), json!({"true":null})),
        (json!("Evaluate impact"), json!({"false":"Routine inquiry"})),
        (
            json!({"scope":"support","flags":[true,3,null]}),
            json!({"true":["urgent"],"false":{"kind":"routine"}}),
        ),
        (
            json!(["Assess urgency",{"priority":1}]),
            json!({"true":"blocked"}),
        ),
    ] {
        let wire = json!({"timeout_ms":1000,"evaluations":[{"id":"ticket","state":"Account is blocked","questions":{"urgent":{"type":"noul","instructions":instructions,"criteria":criteria}}}]});
        let input = serde_json::from_value(wire).unwrap();
        ok(server.client().evaluate(input, DEFAULT_MODEL).await);
        let observed = server.observed.requests.lock().unwrap();
        let sent = &observed.last().unwrap().0["questions"]["urgent"];
        assert_eq!(sent["instructions"], instructions);
        if !criteria.is_null() {
            assert_eq!(sent["criteria"], criteria);
        }
    }
}

#[tokio::test]
async fn default_response_budget_accepts_valid_data_above_the_previous_megabyte_limit() {
    let server = Server::start(|body| {
        let mut response = answer(body);
        response["metadata"] = json!("x".repeat(1024 * 1024 + 1));
        Reply::ok(response)
    })
    .await;
    assert_eq!(
        ok(server.client().evaluate(request(1), DEFAULT_MODEL).await)
            .2
            .requests,
        1
    );
}

#[tokio::test]
async fn operator_timeout_above_default_is_used_by_both_methods() {
    let server = Server::start(|body| {
        Reply::ok(if body.is_null() {
            model_cards()
        } else {
            answer(body)
        })
    })
    .await;
    let client = server.client().with_limits(ExecutionLimits {
        max_timeout_ms: 600000,
        ..ExecutionLimits::default()
    });
    let mut input = request(1);
    input.timeout_ms = 500000;
    ok(client.evaluate(input, DEFAULT_MODEL).await);
    models_ok(client.list_models(models_request(500000)).await);
}

#[tokio::test]
async fn streaming_bodies_cannot_bypass_configured_response_limits() {
    // No Content-Length: a declared-length-only guard would accept both valid bodies.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}/v1/systemone", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        for models in [false, true] {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0; 4096];
            let count = stream.read(&mut buf).await.unwrap();
            assert!(count > 0);
            let mut body = if models {
                model_cards()
            } else {
                json!({"model":"jev-1.13.0","answers":{"urgent":{"type":"noul","noul":0.5}},"usage":{}})
            };
            body["metadata"] = json!("x".repeat(1024));
            let body = body.to_string();
            let wire = format!("HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n{:x}\r\n{}\r\n0\r\n\r\n",body.len(),body);
            stream.write_all(wire.as_bytes()).await.unwrap();
        }
    });
    let client =
        JevClient::with_endpoint(Some("test-key".into()), endpoint).with_limits(ExecutionLimits {
            max_response_bytes: 128,
            ..ExecutionLimits::default()
        });
    error(
        client.evaluate(request(1), DEFAULT_MODEL).await,
        "invalid_response",
    );
    models_error(
        client.list_models(models_request(1000)).await,
        "invalid_response",
    );
    server.await.unwrap();
}

#[tokio::test]
async fn accepted_results_preserve_each_usage_counter() {
    let server = Server::start(|body| {
        let mut response = answer(body);
        response["usage"] = if body["state"]["index"] == 0 {
            json!({"input_tokens":0,"output_tokens":null})
        } else {
            json!({"input_tokens":11,"output_tokens":3})
        };
        Reply::ok(response)
    })
    .await;
    let (_, results, stats) = ok(server.client().evaluate(request(2), DEFAULT_MODEL).await);
    let wire = serde_json::to_value(results).unwrap();
    assert_eq!(
        wire["ticket-0"]["usage"],
        json!({"input_tokens":0,"output_tokens":null})
    );
    assert_eq!(
        wire["ticket-1"]["usage"],
        json!({"input_tokens":11,"output_tokens":3})
    );
    assert_eq!((stats.input_tokens, stats.output_tokens), (11, 3));
    assert!(!stats.usage_complete);
}

#[tokio::test]
async fn cancelled_call_aborts_http_and_releases_its_id() {
    let server = Server::start(|body| {
        let mut reply = Reply::ok(answer(body));
        reply.delay_ms = 500;
        reply.slow_body = true;
        reply
    })
    .await;
    let client = server.client();
    let mut input = request(6);
    input.request_id = Some("job-42".into());
    let call = tokio::spawn({
        let client = client.clone();
        let input = input.clone();
        async move { client.evaluate(input, DEFAULT_MODEL).await }
    });
    server.wait_for(4).await;
    let cancel = || judge_contract::CancelRequest {
        request_id: "job-42".into(),
    };
    let wrong = client
        .with_caller_id(Some("another-worker"))
        .cancel(cancel());
    assert!(matches!(
        wrong,
        judge_contract::CancelResponse::Ok { cancelled: false }
    ));
    let duplicate = error(
        client.evaluate(input, DEFAULT_MODEL).await,
        "invalid_request",
    );
    assert_eq!(duplicate.attempts, 0);
    assert!(matches!(
        client.cancel(cancel()),
        judge_contract::CancelResponse::Ok { cancelled: true }
    ));
    let stats = error(
        timeout(Duration::from_millis(200), call)
            .await
            .unwrap()
            .unwrap(),
        "cancelled",
    );
    assert_eq!(stats.attempts, 4);
    assert!(!stats.usage_complete);
    assert_eq!(server.count(), 4);
    assert!(matches!(
        client.cancel(cancel()),
        judge_contract::CancelResponse::Ok { cancelled: false }
    ));
    let mut reuse = request(1);
    reuse.request_id = Some("job-42".into());
    ok(client.evaluate(reuse, DEFAULT_MODEL).await);
}

#[tokio::test]
async fn cancel_interrupts_backoff_without_a_new_attempt() {
    let server = Server::start(|_| Reply {
        status: 429,
        body: "Rate limited".into(),
        delay_ms: 0,
        slow_body: false,
    })
    .await;
    let client = server.client();
    let mut input = request(1);
    input.request_id = Some("backoff".into());
    input.options.retry = judge_contract::RetryPolicy {
        backoff_initial_ms: 1000,
        backoff_max_ms: 1000,
        backoff_jitter: 0.0,
        ..Default::default()
    };
    let call = tokio::spawn({
        let client = client.clone();
        async move { client.evaluate(input, DEFAULT_MODEL).await }
    });
    server.wait_for(1).await;
    sleep(Duration::from_millis(30)).await;
    assert!(matches!(
        client.cancel(judge_contract::CancelRequest {
            request_id: "backoff".into()
        }),
        judge_contract::CancelResponse::Ok { cancelled: true }
    ));
    let stats = error(
        timeout(Duration::from_millis(200), call)
            .await
            .unwrap()
            .unwrap(),
        "cancelled",
    );
    assert_eq!(stats.attempts, 1);
    assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn cancelled_model_listing_and_queued_evaluation_stop_before_dispatch() {
    let server = Server::start(|body| {
        let mut reply = Reply::ok(if body.is_null() {
            model_cards()
        } else {
            answer(body)
        });
        reply.delay_ms = 500;
        reply
    })
    .await;
    let client = server.client();
    let holder = tokio::spawn({
        let client = client.clone();
        async move { client.evaluate(request(4), DEFAULT_MODEL).await }
    });
    server.wait_for(4).await;
    let mut input = request(1);
    input.request_id = Some("queued".into());
    let queued = tokio::spawn({
        let client = client.clone();
        async move { client.evaluate(input, DEFAULT_MODEL).await }
    });
    let mut models = models_request(3000);
    models.request_id = Some("catalog".into());
    let listing = tokio::spawn({
        let client = client.clone();
        async move { client.list_models(models).await }
    });
    for id in ["queued", "catalog"] {
        timeout(Duration::from_millis(200), async {
            loop {
                if matches!(
                    client.cancel(judge_contract::CancelRequest {
                        request_id: id.into()
                    }),
                    judge_contract::CancelResponse::Ok { cancelled: true }
                ) {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    assert_eq!(error(queued.await.unwrap(), "cancelled").attempts, 0);
    assert_eq!(
        models_error(listing.await.unwrap(), "cancelled").attempts,
        0
    );
    assert_eq!(server.count(), 4);
    ok(holder.await.unwrap());
}

#[tokio::test]
async fn retry_after_attempt_timeout_keeps_known_usage_but_marks_total_incomplete() {
    let seen = Arc::new(AtomicUsize::new(0));
    let server = Server::start(move |body| {
        let mut reply = Reply::ok(answer(body));
        if seen.fetch_add(1, Ordering::SeqCst) == 0 {
            reply.delay_ms = 350;
            reply.slow_body = true;
        }
        reply
    })
    .await;
    let mut input = request(1);
    input.options.attempt_timeout_ms = Some(60);
    input.options.retry = judge_contract::RetryPolicy {
        max_retries: 1,
        backoff_initial_ms: 1,
        backoff_jitter: 0.0,
        ..Default::default()
    };
    let (_, results, stats) = ok(server.client().evaluate(input, DEFAULT_MODEL).await);
    assert_eq!(stats.attempts, 2);
    assert_eq!(
        (stats.requests, stats.input_tokens, stats.output_tokens),
        (1, 11, 3)
    );
    assert!(!stats.usage_complete);
    assert_eq!(
        results["ticket-0"].usage.as_ref().unwrap().input_tokens,
        Some(11)
    );
}

#[tokio::test]
async fn cancellation_preserves_previously_accepted_usage_without_partial_answers() {
    let server = Server::start(|body| {
        let mut reply = Reply::ok(answer(body));
        if body["state"]["index"] == 1 {
            reply.delay_ms = 500;
            reply.slow_body = true;
        }
        reply
    })
    .await;
    let client = server.client();
    let mut input = request(2);
    input.request_id = Some("partial-completion".into());
    let call = tokio::spawn({
        let client = client.clone();
        async move { client.evaluate(input, DEFAULT_MODEL).await }
    });
    server.wait_for(2).await;
    sleep(Duration::from_millis(60)).await;
    assert!(matches!(
        client.cancel(judge_contract::CancelRequest {
            request_id: "partial-completion".into()
        }),
        judge_contract::CancelResponse::Ok { cancelled: true }
    ));
    let stats = error(call.await.unwrap(), "cancelled");
    assert_eq!(
        (
            stats.attempts,
            stats.requests,
            stats.input_tokens,
            stats.output_tokens
        ),
        (2, 1, 11, 3)
    );
    assert!(!stats.usage_complete);
}

#[tokio::test]
async fn invalid_options_fail_the_entire_batch_before_http() {
    let server = Server::start(|body| Reply::ok(answer(body))).await;
    let client = server.client();
    for invalid in [
        json!({"headers":{"Authorization":"caller-key"}}),
        json!({"retry":{"max_retries":11}}),
        json!({"attempt_timeout_ms":0}),
        json!({"headers":{"x-label":"line\r\nbreak"}}),
    ] {
        let mut input = request(5);
        input.options = serde_json::from_value(invalid).unwrap();
        let stats = error(
            client.evaluate(input, DEFAULT_MODEL).await,
            "invalid_request",
        );
        assert_eq!(stats.attempts, 0);
    }
    assert_eq!(server.count(), 0);
}

#[tokio::test]
async fn default_retry_policy_recovers_rate_limits_for_evaluation_and_models() {
    for listing in [false, true] {
        let seen = Arc::new(AtomicUsize::new(0));
        let server = Server::start(move |body| {
            let index = seen.fetch_add(1, Ordering::SeqCst);
            if index < 2 {
                Reply {
                    status: if index == 0 { 429 } else { 529 },
                    body: "Temporarily unavailable".into(),
                    delay_ms: 0,
                    slow_body: false,
                }
            } else {
                Reply::ok(if listing { model_cards() } else { answer(body) })
            }
        })
        .await;
        let client = server.client();
        let mut options = judge_contract::RequestOptions::default();
        options.retry.backoff_initial_ms = 1;
        options.retry.backoff_jitter = 0.0;
        let stats = if listing {
            let mut input = models_request(3000);
            input.options = options;
            models_ok(client.list_models(input).await).1
        } else {
            let mut input = request(1);
            input.options = options;
            ok(client.evaluate(input, DEFAULT_MODEL).await).2
        };
        assert_eq!((stats.attempts, stats.requests), (3, 1));
        assert!(stats.usage_complete);
        assert_eq!(stats.input_tokens, if listing { 0 } else { 11 });
        assert_eq!(server.count(), 3);
    }
}

#[tokio::test]
async fn later_batch_entries_use_http_slots_while_earlier_entries_back_off() {
    let seen = Arc::new(std::array::from_fn::<_, 8, _>(|_| AtomicUsize::new(0)));
    let server = Server::start(move |body| {
        let index = body["state"]["index"].as_u64().unwrap() as usize;
        let attempt = seen[index].fetch_add(1, Ordering::SeqCst);
        if index < 4 && attempt == 0 {
            Reply {
                status: 429,
                body: "Rate limited".into(),
                delay_ms: 0,
                slow_body: false,
            }
        } else {
            let mut reply = Reply::ok(answer(body));
            if index >= 4 {
                reply.delay_ms = 400;
                reply.slow_body = true;
            }
            reply
        }
    })
    .await;
    let mut input = request(8);
    input.timeout_ms = 650;
    input.options.retry = judge_contract::RetryPolicy {
        max_retries: 1,
        backoff_initial_ms: 400,
        backoff_max_ms: 400,
        backoff_jitter: 0.0,
        ..Default::default()
    };
    let (_, results, stats) = ok(server.client().evaluate(input, DEFAULT_MODEL).await);
    assert_eq!(results.len(), 8);
    assert_eq!(
        (stats.attempts, stats.requests, stats.input_tokens),
        (12, 8, 88)
    );
    assert!(stats.usage_complete);
    assert!(server.observed.peak.load(Ordering::SeqCst) <= 4);
    let observed = server.observed.requests.lock().unwrap();
    let mut first_wave = observed[..8]
        .iter()
        .map(|(body, _)| body["state"]["index"].as_u64().unwrap())
        .collect::<Vec<_>>();
    first_wave.sort_unstable();
    assert_eq!(first_wave, (0..8).collect::<Vec<_>>());
}

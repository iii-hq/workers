//! Loopback coverage of `http::on-request`: the fake engine plays the observer
//! lifecycle (`observeradded` / `observerack` / `observerremoved`), the server
//! calls `http::on-request` for every request, and its deliver hook answers
//! the call or reaches the route's target through the observer. Never an
//! external engine.
mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::fake_engine::FakeEngine;
use iii_http::boot::{self, BootHandle};
use iii_http::config::{MiddlewareConfig, RestApiConfig, WebhookListenerConfig};
use iii_http::on_request::ON_REQUEST_FN;
use iii_http::types::HttpRequest;
use iii_sdk::channel::ChannelWriter;
use iii_sdk::errors::Error;
use iii_sdk::RegisterFunction;
use serde_json::{json, Value};

async fn start() -> (FakeEngine, BootHandle) {
    start_with_middleware(Vec::new()).await
}

/// [`start`], with `middleware` as the global middleware.
async fn start_with_middleware(middleware: Vec<MiddlewareConfig>) -> (FakeEngine, BootHandle) {
    let engine = FakeEngine::start().await;
    let config = RestApiConfig {
        host: "127.0.0.1".into(),
        port: 0,
        webhook_listener: Some(WebhookListenerConfig {
            port: 0,
            ..Default::default()
        }),
        middleware,
        ..Default::default()
    };
    let boot = boot::start(engine.iii.clone(), config).await.unwrap();
    (engine, boot)
}

fn global(function_id: &str) -> MiddlewareConfig {
    MiddlewareConfig {
        function_id: function_id.into(),
        phase: "preHandler".into(),
        priority: 0,
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

/// A target that echoes the request it received, with a fixed status and
/// header, and counts its calls.
fn register_echo(engine: &FakeEngine, id: &str) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    engine.iii.register_function(
        id.to_string(),
        RegisterFunction::new_async(move |req: HttpRequest| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<Value, Error>(json!({
                    "status_code": 201,
                    "headers": { "x-served-by": "target" },
                    "body": {
                        "method": req.method,
                        "path": req.path,
                        "path_params": req.path_params,
                        "query_params": req.query_params,
                        "body": req.body,
                    }
                }))
            }
        }),
    );
    calls
}

/// Plays `iii.observe({ function_id: 'http::on-request', args, filters }, target)`
/// and returns the host's rejection, if any.
async fn observe(
    engine: &FakeEngine,
    id: &str,
    target: &str,
    args: Value,
    filters: Value,
) -> Option<Value> {
    engine
        .add_observer(
            ON_REQUEST_FN,
            json!({ "id": id, "target": target, "args": args, "filters": filters }),
        )
        .await
}

#[tokio::test]
async fn an_observer_route_serves_its_targets_result() {
    let (engine, boot) = start().await;
    let calls = register_echo(&engine, "test::new-ping");
    assert!(observe(
        &engine,
        "obs-ping",
        "test::new-ping",
        json!({"method": "GET", "path": "/new/ping"}),
        json!({}),
    )
    .await
    .is_none());
    register_echo(&engine, "test::new-user");
    assert!(observe(
        &engine,
        "obs-user",
        "test::new-user",
        json!({"method": "post", "path": "/new/users/{id}"}),
        json!({}),
    )
    .await
    .is_none());
    assert!(boot.on_request.is_observed());
    let http = client();
    let addr = boot.local_addr;

    let resp = http
        .get(format!("http://{addr}/new/ping?q=1"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    assert_eq!(resp.headers()["x-served-by"], "target");
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["method"], "GET");
    assert_eq!(body["path"], "/new/ping");
    assert_eq!(body["query_params"]["q"], "1");
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let resp = http
        .post(format!("http://{addr}/new/users/42"))
        .json(&json!({"name": "ada"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["path"], "/new/users/:id");
    assert_eq!(body["path_params"]["id"], "42");
    assert_eq!(body["body"], json!({"name": "ada"}));

    // A known path with another method is a 405 with Allow; an unknown one a 404.
    let resp = http
        .get(format!("http://{addr}/new/users/42"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 405);
    assert_eq!(resp.headers()["allow"], "POST");
    let resp = http
        .get(format!("http://{addr}/new/nothing"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 404);

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn a_targets_response_keeps_its_status_headers_and_content_type() {
    let (engine, boot) = start().await;
    engine.iii.register_function(
        "test::text".to_string(),
        RegisterFunction::new_async(|_req: HttpRequest| async move {
            Ok::<Value, Error>(json!({
                "status_code": 202,
                "headers": ["Content-Type: text/plain", "X-Kind: same"],
                "body": "plain text",
            }))
        }),
    );
    assert!(observe(
        &engine,
        "obs-text",
        "test::text",
        json!({"method": "GET", "path": "/new/text"}),
        json!({}),
    )
    .await
    .is_none());

    let resp = client()
        .get(format!("http://{}/new/text", boot.local_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 202);
    assert_eq!(resp.headers()["content-type"], "text/plain");
    assert_eq!(resp.headers()["x-kind"], "same");
    assert_eq!(resp.text().await.unwrap(), "plain text");

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn a_targets_error_is_a_500_with_its_code_and_message() {
    let (engine, boot) = start().await;
    engine.iii.register_function(
        "test::boom".to_string(),
        RegisterFunction::new_async(|_req: HttpRequest| async move {
            Err::<Value, Error>(Error::Remote {
                code: "CUSTOM_BOOM".to_string(),
                message: "backend exploded".to_string(),
                stacktrace: None,
            })
        }),
    );
    assert!(observe(
        &engine,
        "obs-boom",
        "test::boom",
        json!({"method": "GET", "path": "/new/boom"}),
        json!({}),
    )
    .await
    .is_none());

    let resp = client()
        .get(format!("http://{}/new/boom", boot.local_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 500);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "CUSTOM_BOOM");
    assert_eq!(body["error"]["message"], "backend exploded");

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn an_observer_target_can_stream_through_the_response_channel() {
    let (engine, boot) = start().await;
    let url = engine.url.clone();
    engine.iii.register_function(
        "test::stream".to_string(),
        RegisterFunction::new_async(move |req: HttpRequest| {
            let url = url.clone();
            async move {
                let writer = ChannelWriter::new(&url, &req.response);
                writer
                    .send_message(&json!({"type": "set_status", "status_code": 206}).to_string())
                    .await?;
                writer.write(b"chunk-1").await?;
                writer.write(b"chunk-2").await?;
                writer.close().await?;
                Ok::<Value, Error>(Value::Null)
            }
        }),
    );
    assert!(observe(
        &engine,
        "obs-stream",
        "test::stream",
        json!({"method": "GET", "path": "/new/stream"}),
        json!({}),
    )
    .await
    .is_none());

    let resp = client()
        .get(format!("http://{}/new/stream", boot.local_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 206);
    assert_eq!(resp.text().await.unwrap(), "chunk-1chunk-2");

    boot.shutdown().await;
    engine.shutdown().await;
}

fn register_middleware(engine: &FakeEngine, id: &str, result: Value) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    engine.iii.register_function(
        id.to_string(),
        RegisterFunction::new_async(move |_input: Value| {
            let counter = counter.clone();
            let result = result.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<Value, Error>(result)
            }
        }),
    );
    calls
}

#[tokio::test]
async fn route_middleware_runs_on_an_observer_route() {
    let (engine, boot) = start().await;
    let passed = register_middleware(&engine, "test::mw-continue", json!({"action": "continue"}));
    let blocked = register_middleware(
        &engine,
        "test::mw-respond",
        json!({"action": "respond", "response": {"status_code": 403, "body": {"denied": true}}}),
    );
    let open = register_echo(&engine, "test::mw-open");
    let closed = register_echo(&engine, "test::mw-closed");
    assert!(observe(
        &engine,
        "obs-mw-open",
        "test::mw-open",
        json!({"method": "GET", "path": "/new/open", "middleware_function_ids": ["test::mw-continue"]}),
        json!({}),
    )
    .await
    .is_none());
    assert!(observe(
        &engine,
        "obs-mw-closed",
        "test::mw-closed",
        json!({"method": "GET", "path": "/new/closed", "middleware_function_ids": ["test::mw-continue", "test::mw-respond"]}),
        json!({}),
    )
    .await
    .is_none());

    let http = client();
    let resp = http
        .get(format!("http://{}/new/open", boot.local_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    assert_eq!(passed.load(Ordering::SeqCst), 1);
    assert_eq!(open.load(Ordering::SeqCst), 1);

    let resp = http
        .get(format!("http://{}/new/closed", boot.local_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 403);
    assert_eq!(resp.json::<Value>().await.unwrap(), json!({"denied": true}));
    assert_eq!(passed.load(Ordering::SeqCst), 2);
    assert_eq!(blocked.load(Ordering::SeqCst), 1);
    assert_eq!(
        closed.load(Ordering::SeqCst),
        0,
        "middleware short-circuits"
    );

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn an_observers_filters_answer_422_without_calling_the_target() {
    let (engine, boot) = start().await;
    let calls = register_echo(&engine, "test::signed");
    // Normal form of { 'input.headers.x-signature': { $exists: true } }.
    let filters = json!({"$and": [{"input.headers.x-signature": {"$exists": true}}]});
    assert!(observe(
        &engine,
        "obs-signed",
        "test::signed",
        json!({"method": "POST", "path": "/new/hook"}),
        filters,
    )
    .await
    .is_none());

    let url = format!("http://{}/new/hook", boot.local_addr);
    let resp = client().post(&url).send().await.unwrap();
    assert_eq!(resp.status(), 422);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], "CONDITION_NOT_MET");
    assert_eq!(body["skipped"], true);
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    let resp = client()
        .post(&url)
        .header("x-signature", "sha256=1")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn invalid_args_are_rejected_and_add_no_route() {
    let (engine, boot) = start().await;
    register_echo(&engine, "test::invalid");
    for (id, args) in [
        ("obs-no-method", json!({"path": "/new/invalid"})),
        ("obs-no-path", json!({"method": "GET"})),
        (
            "obs-typo",
            json!({"method": "GET", "path": "/new/invalid", "api_path": "/x"}),
        ),
        ("obs-no-args", json!({})),
    ] {
        let rejection = observe(&engine, id, "test::invalid", args, json!({}))
            .await
            .unwrap_or_else(|| panic!("{id} accepted"));
        assert_eq!(rejection["code"], "INVALID_ARGS", "{id}: {rejection}");
    }
    assert!(boot
        .routes
        .read()
        .await
        .match_route("GET", "/new/invalid")
        .is_none());
    assert!(!boot.on_request.is_observed());

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn a_route_shape_belongs_to_the_observer_that_claimed_it_first() {
    let (engine, boot) = start().await;
    let first = register_echo(&engine, "test::dup-first");
    let second = register_echo(&engine, "test::dup-second");

    assert!(observe(
        &engine,
        "obs-dup-first",
        "test::dup-first",
        json!({"method": "GET", "path": "/dup/:a"}),
        json!({}),
    )
    .await
    .is_none());
    let rejection = observe(
        &engine,
        "obs-dup-second",
        "test::dup-second",
        json!({"method": "GET", "path": "/dup/{b}"}),
        json!({}),
    )
    .await
    .expect("conflicting observer rejected");
    assert_eq!(rejection["code"], "HTTP_ROUTE_CONFLICT");
    assert!(rejection["message"]
        .as_str()
        .unwrap()
        .contains("conflicts with already-registered route 'GET /dup/:a'"));
    assert_eq!(
        boot.routes
            .read()
            .await
            .match_route("GET", "/dup/1")
            .unwrap()
            .0
            .id(),
        "obs-dup-first"
    );

    let resp = client()
        .get(format!("http://{}/dup/1", boot.local_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["path_params"]["a"], "1");
    assert_eq!(first.load(Ordering::SeqCst), 1);
    assert_eq!(second.load(Ordering::SeqCst), 0);

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn removing_an_observer_removes_its_route() {
    let (engine, boot) = start().await;
    register_echo(&engine, "test::gone");
    assert!(observe(
        &engine,
        "obs-gone",
        "test::gone",
        json!({"method": "GET", "path": "/new/gone"}),
        json!({}),
    )
    .await
    .is_none());
    let url = format!("http://{}/new/gone", boot.local_addr);
    assert_eq!(client().get(&url).send().await.unwrap().status(), 201);

    engine.remove_observer(ON_REQUEST_FN, "obs-gone");
    common::wait_for_no_route(&boot.routes, "GET", "/new/gone").await;
    assert_eq!(client().get(&url).send().await.unwrap().status(), 404);
    assert!(!boot.on_request.is_observed());

    // The path is free again.
    assert!(observe(
        &engine,
        "obs-back",
        "test::gone",
        json!({"method": "GET", "path": "/new/gone"}),
        json!({}),
    )
    .await
    .is_none());

    boot.shutdown().await;
    engine.shutdown().await;
}

/// The engine withdraws an observer while its worker is away and announces
/// it again when the worker reattaches, or announces the observer a restarted
/// worker creates under the same id: the route comes back under that id.
#[tokio::test]
async fn an_observer_announced_again_under_its_id_gets_its_route_back() {
    let (engine, boot) = start().await;
    let before = register_echo(&engine, "test::stable");
    let route = json!({"method": "GET", "path": "/new/stable/{id}"});
    assert!(observe(
        &engine,
        "obs-stable",
        "test::stable",
        route.clone(),
        json!({})
    )
    .await
    .is_none());
    let url = format!("http://{}/new/stable/7", boot.local_addr);
    assert_eq!(client().get(&url).send().await.unwrap().status(), 201);

    engine.remove_observer(ON_REQUEST_FN, "obs-stable");
    common::wait_for_no_route(&boot.routes, "GET", "/new/stable/:id").await;
    assert_eq!(client().get(&url).send().await.unwrap().status(), 404);

    let after = register_echo(&engine, "test::stable-again");
    assert_eq!(
        observe(
            &engine,
            "obs-stable",
            "test::stable-again",
            route,
            json!({})
        )
        .await,
        None,
        "the same id gets the route back without HTTP_ROUTE_CONFLICT"
    );
    let resp = client().get(&url).send().await.unwrap();
    assert_eq!(resp.status(), 201);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["path_params"]["id"], "7");
    assert_eq!(before.load(Ordering::SeqCst), 1);
    assert_eq!(after.load(Ordering::SeqCst), 1, "the new target serves");

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn the_restricted_listener_serves_only_public_webhook_observers() {
    let (engine, boot) = start().await;
    register_echo(&engine, "test::public");
    register_echo(&engine, "test::private");
    assert!(observe(
        &engine,
        "obs-public",
        "test::public",
        json!({"method": "POST", "path": "/new/public", "public_webhook": true}),
        json!({}),
    )
    .await
    .is_none());
    assert!(observe(
        &engine,
        "obs-private",
        "test::private",
        json!({"method": "POST", "path": "/new/private"}),
        json!({}),
    )
    .await
    .is_none());

    let restricted = boot.current_webhook_addr().await.unwrap();
    let http = client();
    let status = |path: &'static str, addr| {
        let http = http.clone();
        async move {
            http.post(format!("http://{addr}{path}"))
                .send()
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(status("/new/public", restricted).await, 201);
    let calls = engine.calls_of(ON_REQUEST_FN).len();
    assert_eq!(status("/new/private", restricted).await, 404);
    // Refused before its body is read: no call of http::on-request.
    let large = http
        .post(format!("http://{restricted}/new/nowhere"))
        .body(vec![b'['; 8 * 1024 * 1024])
        .send()
        .await
        .unwrap();
    assert_eq!(large.status(), 404);
    assert_eq!(engine.calls_of(ON_REQUEST_FN).len(), calls);
    assert_eq!(status("/new/private", boot.local_addr).await, 201);

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn on_request_registers_as_a_private_event_source_with_route_args() {
    let (engine, boot) = start().await;
    // Registered once the engine advertised observe.v2.
    tokio::time::timeout(Duration::from_secs(5), async {
        while engine.registration(ON_REQUEST_FN).is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("registerfunction for http::on-request");
    let frame = engine.registration(ON_REQUEST_FN).unwrap();
    assert_eq!(frame["private"], true);
    let mut required: Vec<&str> = frame["args_format"]["required"]
        .as_array()
        .expect("args_format")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    required.sort_unstable();
    assert_eq!(required, ["method", "path"]);
    // The call's input is the request as the server received it, and the
    // handler returns its input.
    assert_eq!(frame["request_format"], frame["response_format"]);
    let mut fields: Vec<&String> = frame["request_format"]["properties"]
        .as_object()
        .expect("request_format properties")
        .keys()
        .collect();
    fields.sort();
    assert_eq!(
        fields,
        ["body", "headers", "method", "path", "query_params"]
    );
    assert_eq!(engine.on_request_registrations(), 1);

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn the_worker_boots_again_on_the_same_client_after_shutdown() {
    let (engine, boot) = start().await;
    register_echo(&engine, "test::again");
    assert!(observe(
        &engine,
        "obs-again-1",
        "test::again",
        json!({"method": "GET", "path": "/new/again"}),
        json!({}),
    )
    .await
    .is_none());
    boot.shutdown().await;

    // A second boot on the same client registers `http::on-request` again
    // (the e2e suites boot one worker per test on a shared client).
    let boot = boot::start(
        engine.iii.clone(),
        RestApiConfig {
            host: "127.0.0.1".into(),
            port: 0,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(observe(
        &engine,
        "obs-again-2",
        "test::again",
        json!({"method": "GET", "path": "/new/again"}),
        json!({}),
    )
    .await
    .is_none());
    let resp = client()
        .get(format!("http://{}/new/again", boot.local_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    boot.shutdown().await;
    engine.shutdown().await;
}

/// A middleware that answers 403 for a request with an `x-blocked` header and
/// lets every other request through.
fn register_gate(engine: &FakeEngine, id: &str) {
    engine.iii.register_function(
        id.to_string(),
        RegisterFunction::new_async(|input: Value| async move {
            let blocked = input["request"]["headers"].get("x-blocked").is_some();
            Ok::<Value, Error>(if blocked {
                json!({"action": "respond", "response": {"status_code": 403, "body": {"denied": true}}})
            } else {
                json!({"action": "continue"})
            })
        }),
    );
}

#[tokio::test]
async fn every_request_is_one_call_of_on_request() {
    let (engine, boot) = start_with_middleware(vec![global("test::gate")]).await;
    register_gate(&engine, "test::gate");
    let target = register_echo(&engine, "test::counted");
    assert!(observe(
        &engine,
        "obs-counted",
        "test::counted",
        json!({"method": "POST", "path": "/new/counted/:id"}),
        json!({"$and": [{"input.headers.x-signature": {"$exists": true}}]}),
    )
    .await
    .is_none());

    let http = client();
    let url = |path: &str| format!("http://{}{path}", boot.local_addr);
    let signed = |path: &str| http.post(url(path)).header("x-signature", "sha256=1");
    let cases = [
        ("served", signed("/new/counted/1"), 201),
        ("no route", http.get(url("/new/nowhere")), 404),
        ("another method", http.get(url("/new/counted/1")), 405),
        ("filters", http.post(url("/new/counted/1")), 422),
        (
            "body over the limit",
            signed("/new/counted/1").body(vec![b'x'; 16 * 1024 * 1024 + 1]),
            413,
        ),
        (
            "global middleware",
            signed("/new/counted/1").header("x-blocked", "1"),
            403,
        ),
    ];
    for (n, (case, request, status)) in cases.into_iter().enumerate() {
        let resp = request.send().await.unwrap();
        assert_eq!(resp.status(), status, "{case}");
        if status == 405 {
            assert_eq!(resp.headers()["allow"], "POST");
        }
        assert_eq!(
            engine.calls_of(ON_REQUEST_FN).len(),
            n + 1,
            "{case}: one call of http::on-request per request"
        );
    }
    assert_eq!(
        target.load(Ordering::SeqCst),
        1,
        "only the first reached the target"
    );
    assert_eq!(
        engine.calls_of("engine::channels::create").len(),
        2,
        "only a request whose target runs opens the two channels"
    );
    // The call's input is the request as the server received it.
    let first = &engine.calls_of(ON_REQUEST_FN)[0];
    assert_eq!(first["data"]["path"], "/new/counted/1");
    assert_eq!(first["data"]["method"], "POST");
    assert!(first["data"].get("path_params").is_none());
    assert!(first["data"].get("request_body").is_none());

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn the_deliver_hooks_calls_are_made_from_the_functions_invocation() {
    let (engine, boot) = start_with_middleware(vec![global("test::global-mw")]).await;
    register_middleware(&engine, "test::global-mw", json!({"action": "continue"}));
    register_middleware(&engine, "test::route-mw", json!({"action": "continue"}));
    register_echo(&engine, "test::caller");
    assert!(observe(
        &engine,
        "obs-caller",
        "test::caller",
        json!({"method": "GET", "path": "/new/caller", "middleware_function_ids": ["test::route-mw"]}),
        json!({}),
    )
    .await
    .is_none());

    let resp = client()
        .get(format!("http://{}/new/caller", boot.local_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);

    let call = engine.calls_of(ON_REQUEST_FN).remove(0);
    let watched = [
        ON_REQUEST_FN,
        "test::global-mw",
        "test::route-mw",
        "engine::channels::create",
        "test::caller",
    ];
    let order: Vec<String> = engine
        .called()
        .into_iter()
        .filter(|id| watched.contains(&id.as_str()))
        .collect();
    assert_eq!(
        order,
        [
            ON_REQUEST_FN,
            "test::global-mw",
            "test::route-mw",
            "engine::channels::create",
            "engine::channels::create",
            "test::caller",
        ],
        "global middleware, route middleware, the channels, then the target"
    );
    for id in &watched[1..] {
        for frame in engine.calls_of(id) {
            assert_eq!(
                frame["caller"]["function_id"], ON_REQUEST_FN,
                "{id}: {frame}"
            );
            assert_eq!(
                frame["caller"]["invocation_id"], call["invocation_id"],
                "{id}: {frame}"
            );
        }
    }
    let through = engine.calls_of("test::caller").remove(0);
    assert_eq!(through["activation"]["observer_id"], "obs-caller");

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn filters_read_the_request_as_the_server_received_it() {
    let (engine, boot) = start().await;
    let calls = register_echo(&engine, "test::seven");
    assert!(observe(
        &engine,
        "obs-seven",
        "test::seven",
        json!({"method": "GET", "path": "/new/items/:id"}),
        json!({"$and": [{"input.path": {"$eq": "/new/items/7"}}]}),
    )
    .await
    .is_none());

    let http = client();
    let resp = http
        .get(format!("http://{}/new/items/7", boot.local_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 201);
    let body: Value = resp.json().await.unwrap();
    // The target receives the route's template and its params.
    assert_eq!(body["path"], "/new/items/:id");
    assert_eq!(body["path_params"]["id"], "7");
    let resp = http
        .get(format!("http://{}/new/items/8", boot.local_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 422);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    boot.shutdown().await;
    engine.shutdown().await;
}

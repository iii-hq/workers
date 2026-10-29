//! Loopback coverage of `http::on-request`: the fake engine plays the observer
//! lifecycle (`observeradded` / `observerack` / `observerremoved`), routes are
//! served next to `http` trigger bindings from the same table, and each
//! observer's target is reached with `obs.call`. Never an external engine.
mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use common::fake_engine::FakeEngine;
use iii_http::boot::{self, BootHandle};
use iii_http::config::{RestApiConfig, WebhookListenerConfig};
use iii_http::on_request::ON_REQUEST_FN;
use iii_http::types::HttpRequest;
use iii_sdk::channel::ChannelWriter;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::RegisterFunction;
use serde_json::{json, Value};

async fn start() -> (FakeEngine, BootHandle) {
    let engine = FakeEngine::start_with_observe().await;
    let config = RestApiConfig {
        host: "127.0.0.1".into(),
        port: 0,
        webhook_listener: Some(WebhookListenerConfig {
            port: 0,
            ..Default::default()
        }),
        ..Default::default()
    };
    let boot = boot::start(engine.iii.clone(), config).await.unwrap();
    (engine, boot)
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
                        "trigger": req.trigger,
                    }
                }))
            }
        }),
    );
    calls
}

/// Plays `iii.observe({ function_id: 'http::on-request', args, filters }, target)`
/// and returns the owner's rejection, if any.
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

/// An old consumer's binding: `registerTrigger({ type: 'http', ... })`.
#[allow(deprecated)] // an old consumer: binds the `http` trigger type
fn bind_trigger(engine: &FakeEngine, function_id: &str, config: Value) {
    engine
        .iii
        .register_trigger(RegisterTriggerInput::new("http", function_id, config))
        .unwrap();
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
    // Same payload as an `http` trigger's function receives.
    assert_eq!(
        body["trigger"],
        json!({"type": "http", "path": "/new/ping", "method": "GET"})
    );
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

    // Same matching rules as trigger routes: 405 with Allow, then 404.
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
async fn trigger_and_observer_responses_are_mapped_the_same_way() {
    let (engine, boot) = start().await;
    for id in ["test::text-legacy", "test::text-new"] {
        engine.iii.register_function(
            id.to_string(),
            RegisterFunction::new_async(|_req: HttpRequest| async move {
                Ok::<Value, Error>(json!({
                    "status_code": 202,
                    "headers": ["Content-Type: text/plain", "X-Kind: same"],
                    "body": "plain text",
                }))
            }),
        );
    }
    bind_trigger(
        &engine,
        "test::text-legacy",
        json!({"api_path": "/legacy/text", "http_method": "GET"}),
    );
    common::wait_for_route(&boot.routes, "GET", "/legacy/text").await;
    assert!(observe(
        &engine,
        "obs-text",
        "test::text-new",
        json!({"method": "GET", "path": "/new/text"}),
        json!({}),
    )
    .await
    .is_none());

    let http = client();
    let mut seen = Vec::new();
    for path in ["/legacy/text", "/new/text"] {
        let resp = http
            .get(format!("http://{}{path}", boot.local_addr))
            .send()
            .await
            .unwrap();
        let status = resp.status();
        let content_type = resp.headers()["content-type"].clone();
        let kind = resp.headers()["x-kind"].clone();
        seen.push((status, content_type, kind, resp.text().await.unwrap()));
    }
    assert_eq!(seen[0].0, 202);
    assert_eq!(seen[0].3, "plain text");
    assert_eq!(seen[0], seen[1]);

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn an_observer_targets_error_maps_like_a_trigger_targets_error() {
    let (engine, boot) = start().await;
    for id in ["test::boom-legacy", "test::boom-new"] {
        engine.iii.register_function(
            id.to_string(),
            RegisterFunction::new_async(|_req: HttpRequest| async move {
                Err::<Value, Error>(Error::Remote {
                    code: "CUSTOM_BOOM".to_string(),
                    message: "backend exploded".to_string(),
                    stacktrace: None,
                })
            }),
        );
    }
    bind_trigger(
        &engine,
        "test::boom-legacy",
        json!({"api_path": "/legacy/boom", "http_method": "GET"}),
    );
    common::wait_for_route(&boot.routes, "GET", "/legacy/boom").await;
    assert!(observe(
        &engine,
        "obs-boom",
        "test::boom-new",
        json!({"method": "GET", "path": "/new/boom"}),
        json!({}),
    )
    .await
    .is_none());

    for path in ["/legacy/boom", "/new/boom"] {
        let resp = client()
            .get(format!("http://{}{path}", boot.local_addr))
            .send()
            .await
            .unwrap();
        assert_eq!(resp.status(), 500, "{path}");
        let body: Value = resp.json().await.unwrap();
        assert_eq!(body["error"]["code"], "CUSTOM_BOOM", "{path}");
        assert_eq!(body["error"]["message"], "backend exploded", "{path}");
    }

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
async fn a_route_shape_belongs_to_whichever_binding_claimed_it_first() {
    let (engine, boot) = start().await;
    let legacy = register_echo(&engine, "test::dup-legacy");
    let new = register_echo(&engine, "test::dup-new");

    // A trigger binding first: a later observer of the same shape is rejected.
    bind_trigger(
        &engine,
        "test::dup-legacy",
        json!({"api_path": "/dup/:a", "http_method": "GET"}),
    );
    common::wait_for_route(&boot.routes, "GET", "/dup/1").await;
    let rejection = observe(
        &engine,
        "obs-dup",
        "test::dup-new",
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

    // An observer first: a later trigger binding of the same shape fails.
    assert!(observe(
        &engine,
        "obs-first",
        "test::dup-new",
        json!({"method": "POST", "path": "/dup2"}),
        json!({}),
    )
    .await
    .is_none());
    bind_trigger(
        &engine,
        "test::dup-legacy",
        json!({"api_path": "/dup2", "http_method": "POST"}),
    );
    common::settle().await;
    assert_eq!(
        boot.routes
            .read()
            .await
            .match_route("POST", "/dup2")
            .unwrap()
            .0
            .id(),
        "obs-first"
    );

    let http = client();
    http.get(format!("http://{}/dup/1", boot.local_addr))
        .send()
        .await
        .unwrap();
    http.post(format!("http://{}/dup2", boot.local_addr))
        .send()
        .await
        .unwrap();
    assert_eq!(legacy.load(Ordering::SeqCst), 1);
    assert_eq!(new.load(Ordering::SeqCst), 1);

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

    // The path is free again, for either kind of binding.
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
    assert_eq!(status("/new/private", restricted).await, 404);
    assert_eq!(status("/new/private", boot.local_addr).await, 201);

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn invoking_on_request_directly_is_forbidden() {
    let (engine, boot) = start().await;
    let result = engine
        .iii
        .trigger(TriggerRequest {
            function_id: ON_REQUEST_FN.to_string(),
            payload: json!({"method": "GET", "path": "/new/ping"}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await;
    match result {
        Err(Error::Remote { code, .. }) => assert_eq!(code, "FORBIDDEN"),
        other => panic!("expected FORBIDDEN, got {other:?}"),
    }

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

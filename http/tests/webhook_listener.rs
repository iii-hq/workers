//! Loopback integration coverage: real SDK registration and binary channels,
//! fake engine from the shared harness, never an external engine or tunnel.
mod common;

use std::sync::Arc;
use std::time::Duration;

use common::fake_engine::FakeEngine;
use iii_http::boot::{self, BootHandle};
use iii_http::config::{CorsConfig, MiddlewareConfig, RestApiConfig, WebhookListenerConfig};
use iii_http::configuration;
use iii_http::types::HttpRequest;
use iii_sdk::channel::ChannelReader;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::trigger::Trigger;
use iii_sdk::RegisterFunction;
use serde_json::{json, Value};
use tokio::net::TcpListener;

fn config(enabled: bool) -> RestApiConfig {
    RestApiConfig {
        host: "127.0.0.1".into(),
        port: 0,
        webhook_listener: enabled.then(|| WebhookListenerConfig {
            port: 0,
            ..Default::default()
        }),
        ..Default::default()
    }
}

async fn start(enabled: bool) -> (FakeEngine, BootHandle) {
    let engine = FakeEngine::start().await;
    let boot = boot::start(engine.iii.clone(), config(enabled))
        .await
        .unwrap();
    configuration::register_config_trigger(
        &engine.iii,
        boot.config.clone(),
        boot.hot_router.clone(),
        boot.control.clone(),
        boot.apply_lock.clone(),
    )
    .unwrap();
    (engine, boot)
}

async fn bind(
    engine: &FakeEngine,
    boot: &BootHandle,
    path: &str,
    method: &str,
    public: Option<bool>,
) -> Trigger {
    let id = format!("test::{method}:{path}");
    let url = engine.url.clone();
    engine.iii.register_function(id.clone(), RegisterFunction::new_async(move |req: HttpRequest| {
        let url = url.clone();
        async move {
            // An empty body has no channel frames in the existing HTTP path.
            let raw = if req.headers.get("content-length").is_some_and(|s| s != "0") {
                tokio::time::timeout(Duration::from_secs(3), ChannelReader::new(&url, &req.request_body).read_all())
                    .await.map_err(|e| Error::Handler(e.to_string()))??
            } else { Vec::new() };
            Ok::<Value, Error>(json!({"body": {"raw": raw, "headers": req.headers, "body": req.body, "method": req.method}}))
        }
    }).description("Local byte-preserving HTTP test backend").response_format(json!({"type": "object"})));
    let mut config = json!({"api_path": path, "http_method": method});
    if let Some(public) = public {
        config["public_webhook"] = json!(public);
    }
    let trigger = engine
        .iii
        .register_trigger(RegisterTriggerInput::new("http", id, config))
        .unwrap();
    common::wait_for_route(&boot.routes, method, path).await;
    trigger
}

async fn reload(engine: &FakeEngine, value: &RestApiConfig) {
    engine.set_config(value.to_json()).await;
    engine
        .iii
        .trigger(TriggerRequest {
            function_id: "http::on-config-change".into(),
            payload: json!({"untrusted": "ignored"}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .unwrap();
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

async fn assert_closed(addr: std::net::SocketAddr) {
    assert!(
        tokio::net::TcpStream::connect(addr).await.is_err(),
        "listener {addr} still accepts connections"
    );
    // Also prove that shutdown released the actual binding, not just its routes.
    let _rebound = TcpListener::bind(addr).await.unwrap();
}

#[tokio::test]
async fn restricted_listener_exposes_only_explicit_routes_and_preserves_raw_bytes() {
    let (engine, boot) = start(true).await;
    let public = bind(&engine, &boot, "/hook", "POST", Some(true)).await;
    bind(&engine, &boot, "/hook", "DELETE", None).await;
    for path in ["/private", "/admin", "/console", "/engine"] {
        bind(&engine, &boot, path, "GET", None).await;
    }
    bind(&engine, &boot, "/false", "POST", Some(false)).await;
    let restricted = boot.current_webhook_addr().await.unwrap();
    let normal = boot.local_addr;
    let http = client();

    // HMAC inputs include UTF-8, whitespace, CRLF and a final newline. The JSON
    // projection differs, but request_body MUST NOT be reserialized from it.
    for raw in [
        "{ \"ação\" : \"☃ café\",\r\n \"n\": 1 }  \n"
            .as_bytes()
            .to_vec(),
        vec![0, 255, 128, 32, 13, 10],
    ] {
        let response = http
            .post(format!("http://{restricted}/hook"))
            .header("content-type", "application/json")
            .header("x-hub-signature-256", "sha256=000abcDEF")
            .header("x-github-delivery", "delivery-82238077")
            .header("x-custom", "keep  inner   spaces")
            .body(raw.clone())
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        let echoed: Value = response.json().await.unwrap();
        assert_eq!(echoed["raw"], json!(raw));
        assert_eq!(echoed["headers"]["x-hub-signature-256"], "sha256=000abcDEF");
        assert_eq!(echoed["headers"]["x-github-delivery"], "delivery-82238077");
        assert_eq!(echoed["headers"]["x-custom"], "keep  inner   spaces");
    }
    assert_eq!(
        http.post(format!("http://{normal}/hook"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        http.get(format!("http://{normal}/private"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );

    for path in [
        "/private",
        "/private/",
        "//private",
        "/%70rivate",
        "/private%2f",
        "/%2570rivate",
        "/admin",
        "/console",
        "/engine",
        "/false",
    ] {
        for method in [
            "GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS", "TRACE", "CONNECT", "post",
        ] {
            let response = http
                .request(
                    reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
                    format!("http://{restricted}{path}"),
                )
                .header("origin", "https://example.invalid")
                .header("access-control-request-method", "GET")
                .header("x-http-method-override", "GET")
                .send()
                .await
                .unwrap();
            assert_eq!(response.status(), 404, "{method} {path}");
            assert!(response.headers().get("allow").is_none());
        }
    }
    for method in ["GET", "HEAD", "DELETE", "OPTIONS", "post"] {
        let response = http
            .request(
                reqwest::Method::from_bytes(method.as_bytes()).unwrap(),
                format!("http://{restricted}/hook"),
            )
            .header("x-http-method-override", "POST")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 405, "{method}");
        assert_eq!(response.headers()["allow"], "POST");
    }
    for path in ["/hook/", "//hook", "/%68ook", "/hook%2f"] {
        assert_eq!(
            http.post(format!("http://{restricted}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            404
        );
    }
    public.unregister();
    common::wait_for_no_route(&boot.routes, "POST", "/hook").await;
    assert_eq!(
        http.post(format!("http://{restricted}/hook"))
            .send()
            .await
            .unwrap()
            .status(),
        404
    );
    assert_eq!(engine.http_provider_count(), 1);
    boot.shutdown().await;
    assert_closed(normal).await;
    assert_closed(restricted).await;
    engine.shutdown().await;
}

#[tokio::test]
async fn reload_enables_rebinds_disables_and_rolls_back_both_listeners() {
    let (engine, boot) = start(false).await;
    bind(&engine, &boot, "/hook", "POST", Some(true)).await;
    assert!(boot.current_webhook_addr().await.is_none());
    let original_normal = boot.local_addr;
    let mut next = config(true);
    reload(&engine, &next).await;
    let first = boot.current_webhook_addr().await.unwrap();
    assert_ne!(first, original_normal);
    assert_eq!(boot.current_addr().await, Some(original_normal));

    // Same-address updates rebuild the shared layers, without moving either listener.
    next.cors = Some(CorsConfig {
        allowed_origins: vec!["https://allowed.invalid".into()],
        allowed_methods: vec!["POST".into()],
    });
    reload(&engine, &next).await;
    assert_eq!(boot.current_webhook_addr().await, Some(first));
    for addr in [first, original_normal] {
        let response = client()
            .post(format!("http://{addr}/hook"))
            .header("origin", "https://allowed.invalid")
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 200);
        assert_eq!(
            response.headers()["access-control-allow-origin"],
            "https://allowed.invalid"
        );
    }

    // If the second bind fails, the prepared normal bind is dropped too and
    // neither running listener nor the live snapshot changes.
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let normal_candidate = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let candidate_addr = normal_candidate.local_addr().unwrap();
    drop(normal_candidate);
    let mut rejected = next.clone();
    rejected.port = candidate_addr.port();
    rejected.webhook_listener.as_mut().unwrap().port = occupied.local_addr().unwrap().port();
    reload(&engine, &rejected).await;
    assert_eq!(boot.current_addr().await, Some(original_normal));
    assert_eq!(boot.current_webhook_addr().await, Some(first));
    assert_eq!(boot.config.read().await.to_json(), next.to_json());
    let _released_candidate = TcpListener::bind(candidate_addr).await.unwrap();

    let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
    next.webhook_listener.as_mut().unwrap().port = reserved.local_addr().unwrap().port();
    drop(reserved);
    reload(&engine, &next).await;
    let second = boot.current_webhook_addr().await.unwrap();
    assert_ne!(first, second);
    assert_closed(first).await;
    assert_eq!(
        client()
            .post(format!("http://{second}/hook"))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );

    // Move the normal listener without touching the restricted listener.
    let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
    next.port = reserved.local_addr().unwrap().port();
    drop(reserved);
    reload(&engine, &next).await;
    let final_normal = boot.current_addr().await.unwrap();
    assert_closed(original_normal).await;
    assert_eq!(boot.current_webhook_addr().await, Some(second));
    next.webhook_listener = None;
    reload(&engine, &next).await;
    assert!(boot.current_webhook_addr().await.is_none());
    assert_closed(second).await;
    next.webhook_listener = config(true).webhook_listener;
    reload(&engine, &next).await;
    let final_public = boot.current_webhook_addr().await.unwrap();
    let control = boot.control.clone();
    let public_control = boot.hot_router.webhook.as_ref().unwrap().control.clone();
    boot.shutdown().await;
    assert_closed(final_normal).await;
    assert_closed(final_public).await;
    reload(&engine, &next).await; // Late bus callback cannot resurrect listeners.
    assert!(control.lock().await.is_none());
    assert!(public_control.lock().await.is_none());
    assert_eq!(engine.http_provider_count(), 1);
    engine.shutdown().await;
}

#[tokio::test]
async fn restricted_listener_preserves_middleware_timeout_and_body_limit() {
    let (engine, boot) = start(true).await;
    bind(&engine, &boot, "/hook", "POST", Some(true)).await;
    let addr = boot.current_webhook_addr().await.unwrap();
    let url = format!("http://{addr}/hook");
    assert_eq!(
        client()
            .post(&url)
            .body(vec![b'x'; 16 * 1024 * 1024 + 1])
            .send()
            .await
            .unwrap()
            .status(),
        413
    );

    let calls = common::backend::register_respond_middleware(&engine.iii, "test::block");
    let mut next = config(true);
    next.middleware = vec![MiddlewareConfig {
        function_id: "test::block".into(),
        phase: "preHandler".into(),
        priority: 0,
    }];
    reload(&engine, &next).await;
    assert_eq!(client().post(&url).send().await.unwrap().status(), 403);
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

    engine.iii.register_function(
        "test::slow",
        RegisterFunction::new_async(|_: Value| async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            Ok::<Value, Error>(json!({"action": "continue"}))
        })
        .description("Slow local middleware")
        .request_format(json!({"type": "object"}))
        .response_format(json!({"type": "object"})),
    );
    next.middleware[0].function_id = "test::slow".into();
    next.default_timeout = 25;
    reload(&engine, &next).await;
    let response = client().post(&url).send().await.unwrap();
    assert!(
        response.status() == 504 || response.status() == 500,
        "timeout layer/invocation race must reject request"
    );
    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn failed_webhook_boot_releases_prepared_normal_listener() {
    let engine = FakeEngine::start().await;
    let occupied = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let reserved = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let normal = reserved.local_addr().unwrap();
    drop(reserved);
    let mut cfg = config(true);
    cfg.port = normal.port();
    cfg.webhook_listener.as_mut().unwrap().port = occupied.local_addr().unwrap().port();
    assert!(boot::start(Arc::clone(&engine.iii), cfg).await.is_err());
    assert_closed(normal).await;
    engine.shutdown().await;
}

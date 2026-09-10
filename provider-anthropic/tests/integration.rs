//! Engine-backed integration suite — real engine, real router, real provider,
//! stubbed upstream. Self-skips when no engine is available.
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient};
use llm_router::register::register_router;
use provider_anthropic::register::register_provider;
use serde_json::{json, Value};

// ── engine bootstrap ── shared with llm-router and every provider suite; see
// llm-router/tests/support/engine_fixture.rs for what it spawns (bare engine +
// standalone state worker) and the skip-vs-fail policy.
#[path = "../../llm-router/tests/support/engine_fixture.rs"]
mod engine_fixture;
use engine_fixture::*;

async fn call(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, iii_sdk::errors::Error> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
}

/// Consumer-side channel: collect frames + a pump that drives dispatch.
async fn consumer_channel(
    iii: &IIIClient,
) -> (
    iii_sdk::channel::StreamChannelRef,
    Arc<std::sync::Mutex<Vec<String>>>,
    tokio::task::JoinHandle<()>,
) {
    let channel = iii_sdk::helpers::create_channel(iii, None)
        .await
        .expect("channel");
    let frames = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let f2 = frames.clone();
    channel
        .reader
        .on_message(move |m| {
            f2.lock().unwrap().push(m);
        })
        .await;
    let writer_ref = channel.writer_ref.clone();
    let pump = tokio::spawn(async move {
        let _ = channel.reader.read_all().await;
    });
    (writer_ref, frames, pump)
}

// ── stub upstream ───────────────────────────────────────────────────────────

/// Routes by request line; loops over connections until dropped.
struct StubUpstream {
    url: String, // http://addr/v1/messages — what goes in the config slice
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for StubUpstream {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

const STUB_SSE: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\nevent: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

const STUB_401: &str = "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"message\":\"invalid x-api-key\"}}";

const STUB_MODELS: &str = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"data\":[{\"id\":\"claude-sonnet-4-6\",\"display_name\":\"Claude Sonnet 4.6\",\"max_input_tokens\":200000,\"max_tokens\":64000,\"capabilities\":{\"thinking\":{\"supported\":true},\"effort\":{\"xhigh\":{\"supported\":false}},\"image_input\":{\"supported\":true}}},{\"id\":\"claude-sonnet-4-6-20260115\",\"display_name\":\"Claude Sonnet 4.6 (dated)\",\"max_input_tokens\":777000,\"max_tokens\":32000},{\"id\":\"claude-new-1\"},{\"id\":\"claude-legacy-1\",\"capabilities\":{\"thinking\":{\"supported\":true,\"types\":{\"adaptive\":{\"supported\":false}}}}}]}";

async fn stub_upstream(messages_response: &'static str) -> StubUpstream {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]);
                let response = if head.starts_with("GET /v1/models") {
                    STUB_MODELS
                } else {
                    messages_response
                };
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    StubUpstream {
        url: format!("http://{addr}/v1/messages"),
        handle,
    }
}

// ── boot + config ───────────────────────────────────────────────────────────

/// Boot router + provider on one engine; wait until the provider is listed.
async fn boot_stack(engine_url: &str) -> (IIIClient, IIIClient) {
    let router_iii = register_worker(engine_url, test_init_options());
    register_router(router_iii.clone())
        .await
        .expect("router boots");
    let provider_iii = register_worker(engine_url, test_init_options());
    register_provider(provider_iii.clone())
        .await
        .expect("provider boots");

    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let list = call(&router_iii, "router::provider::list", json!({}))
            .await
            .unwrap();
        let registered = list["providers"]
            .as_array()
            .is_some_and(|p| p.iter().any(|x| x["id"] == "anthropic"));
        if registered {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "provider never registered: {list}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    (router_iii, provider_iii)
}

/// Paste-a-key: point the anthropic slice at the stub.
async fn configure_stub_key(router_iii: &IIIClient, stub_url: &str) {
    call(
        router_iii,
        "configuration::set",
        json!({ "id": "llm-router", "value": { "providers": {
            "anthropic": { "api_key": "sk-test", "api_url": stub_url }
        } } }),
    )
    .await
    .expect("config set");
}

/// Pull the live (stubbed) list into the catalog and wait until routing can
/// see it — the declaration carries no models, so tests that route by
/// catalog ownership must refresh first.
async fn refresh_and_wait(router_iii: &IIIClient, provider_iii: &IIIClient, expect_id: &str) {
    let res = call(
        provider_iii,
        "provider::anthropic::refresh_models",
        json!({}),
    )
    .await
    .expect("refresh succeeds");
    assert_eq!(res["ok"], true, "refresh response: {res}");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let list = call(
            router_iii,
            "router::models::list",
            json!({ "provider": "anthropic" }),
        )
        .await
        .unwrap();
        let present = list["models"]
            .as_array()
            .is_some_and(|a| a.iter().any(|m| m["id"] == expect_id));
        if present {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "catalog never gained {expect_id}: {list}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

// ── scenarios ───────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn provider_registers_with_persisted_token_and_live_only_catalog() {
    let engine = engine_or_skip!();
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;

    // No static slice: with no key configured the catalog stays empty
    // until live discovery can run (models come from GET /v1/models only).
    let list = call(
        &router_iii,
        "router::models::list",
        json!({ "provider": "anthropic" }),
    )
    .await
    .unwrap();
    let ids: Vec<&str> = list["models"]
        .as_array()
        .map(|a| a.iter().filter_map(|m| m["id"].as_str()).collect())
        .unwrap_or_default();
    assert!(
        ids.is_empty(),
        "catalog empty before discovery, got {ids:?}"
    );

    // the registration token was persisted to the provider's state scope
    let token = call(
        &provider_iii,
        "state::get",
        json!({ "scope": "provider-anthropic", "key": "registration_token" }),
    )
    .await
    .unwrap();
    assert!(
        token.as_str().is_some_and(|t| !t.is_empty()),
        "token persisted, got {token}"
    );

    router_iii.shutdown();
    provider_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_streams_end_to_end_with_cost_fill() {
    let engine = engine_or_skip!();
    let stub = stub_upstream(STUB_SSE).await;
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;
    configure_stub_key(&router_iii, &stub.url).await;
    // catalog-ownership routing needs the live slice in place
    refresh_and_wait(&router_iii, &provider_iii, "claude-sonnet-4-6").await;

    let consumer = register_worker(&engine.url, test_init_options());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let res = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "model": "claude-sonnet-4-6",
                "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 }],
            }),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await
        .expect("chat succeeds");
    assert_eq!(res["ok"], true, "chat response: {res}");
    assert_eq!(res["provider"], "anthropic");
    assert_eq!(res["stop_reason"], "end");
    // the router filled cost_usd from the curated pricing (12 in + 2 out)
    assert!(
        res["usage"]["cost_usd"].as_f64().is_some_and(|c| c > 0.0),
        "cost filled: {res}"
    );

    let _ = tokio::time::timeout(Duration::from_secs(5), pump).await;
    let frames = frames.lock().unwrap();
    let first: Value = serde_json::from_str(frames.first().unwrap()).unwrap();
    assert_eq!(first["type"], "start");
    let last: Value = serde_json::from_str(frames.last().unwrap()).unwrap();
    assert_eq!(last["type"], "done");
    assert_eq!(last["message"]["content"][0]["text"], "Hello");
    assert_eq!(last["message"]["native_stop_reason"], "end_turn");

    consumer.shutdown();
    router_iii.shutdown();
    provider_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn upstream_401_surfaces_as_auth_expired_error_frame() {
    let engine = engine_or_skip!();
    let stub = stub_upstream(STUB_401).await;
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;
    configure_stub_key(&router_iii, &stub.url).await;

    let consumer = register_worker(&engine.url, test_init_options());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    // The 401 stub fails the models fetch too, so the catalog stays empty —
    // pin the provider explicitly (the harness does the same after
    // router::route) to keep this test on the upstream-auth path.
    let res = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "model": "claude-sonnet-4-6",
                "provider": "anthropic",
                "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 }],
            }),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await
        .expect("chat resolves even on upstream failure");
    assert_eq!(res["ok"], false, "chat response: {res}");

    let _ = tokio::time::timeout(Duration::from_secs(5), pump).await;
    let frames = frames.lock().unwrap();
    let last: Value = serde_json::from_str(frames.last().unwrap()).unwrap();
    assert_eq!(last["type"], "error");
    assert_eq!(last["error"]["error_kind"], "auth_expired");

    consumer.shutdown();
    router_iii.shutdown();
    provider_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn refresh_models_reconciles_live_catalog() {
    let engine = engine_or_skip!();
    let stub = stub_upstream(STUB_SSE).await;
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;
    configure_stub_key(&router_iii, &stub.url).await;
    refresh_and_wait(&router_iii, &provider_iii, "claude-sonnet-4-6").await;

    let list = call(
        &router_iii,
        "router::models::list",
        json!({ "provider": "anthropic" }),
    )
    .await
    .unwrap();
    let models = list["models"].as_array().unwrap().clone();
    let ids: Vec<&str> = models.iter().filter_map(|m| m["id"].as_str()).collect();
    // exactly the live list — no curated union, no static aliases
    assert!(ids.contains(&"claude-sonnet-4-6"), "got {ids:?}");
    assert!(ids.contains(&"claude-sonnet-4-6-20260115"));
    assert!(ids.contains(&"claude-new-1"));
    assert!(
        !ids.contains(&"claude-opus-4-7"),
        "static slice gone, got {ids:?}"
    );

    // limits and capabilities come from the live rows
    let dated = models
        .iter()
        .find(|m| m["id"] == "claude-sonnet-4-6-20260115")
        .unwrap();
    assert_eq!(dated["context_window"], 777_000);
    let sonnet = models
        .iter()
        .find(|m| m["id"] == "claude-sonnet-4-6")
        .unwrap();
    assert_eq!(sonnet["supports_thinking"], true);
    assert_eq!(sonnet["supports_xhigh"], false);
    // pricing is the one locally-filled field
    assert!(sonnet["pricing"]["input"].as_f64().is_some_and(|p| p > 0.0));

    router_iii.shutdown();
    provider_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_redeclares_on_router_ready() {
    let engine = engine_or_skip!();
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;

    // simulate a router restart: drop the first router, boot a fresh one
    router_iii.shutdown();
    tokio::time::sleep(Duration::from_millis(500)).await;
    let router2 = register_worker(&engine.url, test_init_options());
    register_router(router2.clone())
        .await
        .expect("router reboots");

    // router::ready trigger → provider re-declares with its persisted token
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let list = call(&router2, "router::provider::list", json!({}))
            .await
            .unwrap();
        let listed = list["providers"]
            .as_array()
            .is_some_and(|p| p.iter().any(|x| x["id"] == "anthropic"));
        if listed {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "provider never re-declared: {list}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    router2.shutdown();
    provider_iii.shutdown();
}

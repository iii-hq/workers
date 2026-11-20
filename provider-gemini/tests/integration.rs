//! Engine-backed integration suite — real engine, real router, real provider,
//! stubbed upstream. Self-skips when no engine is available.
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::{CallRequest, IIIClient, register_worker};
use llm_router::register::register_router;
use provider_gemini::register::register_provider;
use serde_json::{Value, json};

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
    iii.call(CallRequest {
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

/// Serves Gemini's two routes: `GET /models` for discovery and
/// `POST /interactions` for generation, the latter with a canned response per
/// test. Bodies are live captures from `tests/fixtures/`.
struct StubUpstream {
    url: String, // http://addr/interactions — what goes in the config slice
    /// JSON bodies of the POSTs received, in order.
    posted: Arc<std::sync::Mutex<Vec<Value>>>,
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for StubUpstream {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

fn stub_sse() -> String {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n{}",
        include_str!("fixtures/sse/text.sse")
    )
}

/// A bad key is HTTP 400 with google.rpc `API_KEY_INVALID`, on both routes.
fn stub_bad_key() -> String {
    format!(
        "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{}",
        include_str!("fixtures/errors/400_bad_key.json")
    )
}

fn stub_models() -> String {
    format!(
        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{}",
        include_str!("fixtures/models.json")
    )
}

async fn stub_upstream(interactions_response: String) -> StubUpstream {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let bad_key = interactions_response == stub_bad_key();
    let posted = Arc::new(std::sync::Mutex::new(Vec::new()));
    let posted_w = posted.clone();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let response = interactions_response.clone();
            let posted = posted_w.clone();
            tokio::spawn(async move {
                // Read until the headers and the content-length body are in.
                let mut buf = Vec::new();
                let mut chunk = vec![0u8; 65536];
                let head = loop {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    let Some(split) = text.find("\r\n\r\n") else {
                        if n == 0 {
                            break text;
                        }
                        continue;
                    };
                    let len = text[..split]
                        .lines()
                        .find_map(|l| {
                            let (k, v) = l.split_once(':')?;
                            k.eq_ignore_ascii_case("content-length")
                                .then(|| v.trim().parse::<usize>().ok())?
                        })
                        .unwrap_or(0);
                    if n == 0 || buf.len() >= split + 4 + len {
                        break text;
                    }
                };
                if head.starts_with("POST ")
                    && let Some((_, body)) = head.split_once("\r\n\r\n")
                    && let Ok(v) = serde_json::from_str(body)
                {
                    posted.lock().unwrap().push(v);
                }
                // The bad-key stub answers every path the same way, so
                // discovery and generation both see it.
                let body = if head.starts_with("GET /models") && !bad_key {
                    stub_models()
                } else {
                    response
                };
                let _ = sock.write_all(body.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    StubUpstream {
        url: format!("http://{addr}/interactions"),
        posted,
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
            .is_some_and(|p| p.iter().any(|x| x["id"] == "gemini"));
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

/// Point the gemini slice at the stub.
async fn configure_stub_key(router_iii: &IIIClient, stub_url: &str) {
    call(
        router_iii,
        "configuration::set",
        json!({ "id": "llm-router", "value": { "providers": {
            "gemini": { "api_key": "test-key", "api_url": stub_url }
        } } }),
    )
    .await
    .expect("config set");
}

/// Discover the catalog and wait until routing can see it — the declaration
/// carries no models and the slice is credential-gated, so tests that route by
/// catalog ownership must configure a key and refresh first.
async fn refresh_and_wait(router_iii: &IIIClient, provider_iii: &IIIClient, expect_id: &str) {
    let res = call(provider_iii, "provider::gemini::refresh_models", json!({}))
        .await
        .expect("refresh succeeds");
    assert_eq!(res["ok"], true, "refresh response: {res}");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let list = call(
            router_iii,
            "router::models::list",
            json!({ "provider": "gemini" }),
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
async fn provider_registers_with_persisted_token_and_credential_gated_catalog() {
    // A real key exported on the host would leak into the in-process router's
    // env-var fallback and defeat the no-credential assertions below.
    // SAFETY: test-only; nothing else reads or writes this variable while the test runs.
    unsafe { std::env::remove_var("GEMINI_API_KEY") };
    let engine = engine_or_skip!();
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;

    // No static slice + credential gating: an explicit refresh with no key
    // configured reconciles an empty slice without ever calling the upstream
    // (deterministic — the boot-time refresh may still be in flight).
    let res = call(&provider_iii, "provider::gemini::refresh_models", json!({}))
        .await
        .expect("refresh succeeds");
    assert_eq!(res["ok"], true, "refresh response: {res}");
    assert_eq!(res["count"], 0, "no key → empty reconcile: {res}");
    let list = call(
        &router_iii,
        "router::models::list",
        json!({ "provider": "gemini" }),
    )
    .await
    .unwrap();
    let ids: Vec<&str> = list["models"]
        .as_array()
        .map(|a| a.iter().filter_map(|m| m["id"].as_str()).collect())
        .unwrap_or_default();
    assert!(ids.is_empty(), "catalog empty without a key, got {ids:?}");

    // The registration token lands in the provider's state scope; the write
    // races provider visibility (the router lists the provider before the
    // register response returns), so poll.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let token = call(
            &provider_iii,
            "state::get",
            json!({ "scope": "provider-gemini", "key": "registration_token" }),
        )
        .await
        .unwrap();
        if token.as_str().is_some_and(|t| !t.is_empty()) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "token never persisted, got {token}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    router_iii.shutdown();
    provider_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_streams_end_to_end_with_cost_fill() {
    let engine = engine_or_skip!();
    let stub = stub_upstream(stub_sse()).await;
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;
    configure_stub_key(&router_iii, &stub.url).await;
    // catalog-ownership routing needs the discovered slice in place
    refresh_and_wait(&router_iii, &provider_iii, "gemini-3.8-flash").await;

    let consumer = register_worker(&engine.url, test_init_options());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let res = consumer
        .call(CallRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "model": "gemini-3.8-flash",
                "thinking_level": "xhigh",
                "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 }],
            }),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await
        .expect("chat succeeds");
    // A thinking model gets summaries, and xhigh clamps into its range.
    let sent = stub.posted.lock().unwrap().last().cloned().expect("a POST");
    assert_eq!(
        sent["generation_config"]["thinking_summaries"], "auto",
        "{sent}"
    );
    assert_eq!(
        sent["generation_config"]["thinking_level"], "high",
        "{sent}"
    );
    assert_eq!(res["ok"], true, "chat response: {res}");
    assert_eq!(res["provider"], "gemini");
    assert_eq!(res["stop_reason"], "end");
    // the router filled cost_usd from the curated pricing
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
    let content = &last["message"]["content"];
    assert_eq!(content[0]["type"], "thinking");
    assert!(
        content[0]["signature"]
            .as_str()
            .is_some_and(|s| !s.is_empty())
    );
    assert_eq!(content[1]["text"], "Hi there! How can I help you today?");
    assert_eq!(last["message"]["native_stop_reason"], "completed");
    // thought tokens ride inside `output` so the router bills them
    assert_eq!(last["message"]["usage"]["input"], 4);
    assert_eq!(last["message"]["usage"]["output"], 105);
    assert_eq!(last["message"]["usage"]["reasoning"], 95);

    consumer.shutdown();
    router_iii.shutdown();
    provider_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn bad_key_surfaces_as_auth_expired_error_frame() {
    let engine = engine_or_skip!();
    let stub = stub_upstream(stub_bad_key()).await;
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;
    configure_stub_key(&router_iii, &stub.url).await;

    let consumer = register_worker(&engine.url, test_init_options());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let res = consumer
        .call(CallRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "model": "gemini-3.8-flash",
                "provider": "gemini",
                "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 }],
            }),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await
        .expect("chat resolves even on upstream failure");
    assert_eq!(res["ok"], false, "chat response: {res}");

    let _ = tokio::time::timeout(Duration::from_secs(5), pump).await;
    let last: Value = serde_json::from_str(frames.lock().unwrap().last().unwrap()).unwrap();
    assert_eq!(last["type"], "error");
    assert_eq!(last["error"]["error_kind"], "auth_expired");
    assert_eq!(
        last["error"]["error_message"],
        "API key not valid. Please pass a valid API key."
    );

    // On discovery the same 400 means the key is unusable: empty slice, not
    // a blip that keeps the old one.
    let res = call(&provider_iii, "provider::gemini::refresh_models", json!({}))
        .await
        .expect("refresh succeeds");
    assert_eq!(res["ok"], true, "refresh response: {res}");
    assert_eq!(res["count"], 0, "bad key → empty reconcile: {res}");
    let list = call(
        &router_iii,
        "router::models::list",
        json!({ "provider": "gemini" }),
    )
    .await
    .unwrap();
    assert!(
        list["models"].as_array().is_none_or(|a| a.is_empty()),
        "{list}"
    );

    consumer.shutdown();
    router_iii.shutdown();
    provider_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn refresh_models_discovers_the_live_catalog() {
    let engine = engine_or_skip!();
    let stub = stub_upstream(stub_sse()).await;
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;
    configure_stub_key(&router_iii, &stub.url).await;
    refresh_and_wait(&router_iii, &provider_iii, "gemini-3.8-flash").await;

    let list = call(
        &router_iii,
        "router::models::list",
        json!({ "provider": "gemini" }),
    )
    .await
    .unwrap();
    let models = list["models"].as_array().unwrap().clone();
    // the 17 chat ids of the live list; non-chat and 2.5 ids are filtered out
    assert_eq!(models.len(), 17, "{list}");
    assert!(
        !models
            .iter()
            .any(|m| m["id"].as_str().unwrap().starts_with("gemini-2.5-"))
    );

    let flash = models
        .iter()
        .find(|m| m["id"] == "gemini-3.8-flash")
        .unwrap();
    assert_eq!(flash["display_name"], "Gemini 3.8 Flash");
    assert_eq!(flash["context_window"], 1_048_576);
    assert_eq!(flash["max_output_tokens"], 65_536);
    assert_eq!(flash["supports_structured_output"], true);
    assert_eq!(flash["supports_thinking"], true);
    assert_eq!(flash["supports_thinking_off"], false);
    assert_eq!(flash["pricing"]["input"], 0.75);

    let carbon = models
        .iter()
        .find(|m| m["id"] == "gemini-4-carbon")
        .unwrap();
    assert_eq!(carbon["max_output_tokens"], 32_768);

    let alias = models
        .iter()
        .find(|m| m["id"] == "gemini-flash-latest")
        .unwrap();
    assert!(alias.get("pricing").is_none() || alias["pricing"].is_null());

    router_iii.shutdown();
    provider_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_redeclares_on_router_ready() {
    let engine = engine_or_skip!();
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;

    // simulate a router restart that LOST its registry: wipe the persisted
    // record so only a real re-declare (router::on-ready → on_router_ready →
    // declare_and_refresh) can bring the provider back — a durable-registry
    // restore alone must not satisfy this test.
    router_iii.shutdown();
    call(
        &provider_iii,
        "state::set",
        json!({ "scope": "llm-router", "key": "registry", "value": {} }),
    )
    .await
    .expect("wipe persisted registry");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let router2 = register_worker(&engine.url, test_init_options());
    register_router(router2.clone())
        .await
        .expect("router reboots");

    // router::on-ready → provider re-declares with its persisted token
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let list = call(&router2, "router::provider::list", json!({}))
            .await
            .unwrap();
        let listed = list["providers"]
            .as_array()
            .is_some_and(|p| p.iter().any(|x| x["id"] == "gemini"));
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

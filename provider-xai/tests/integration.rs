// Discovery feedback regression uses a real isolated engine/router and local HTTP only.
//! Engine-backed integration suite — real engine, real router, real provider,
//! stubbed upstream. Self-skips when no engine is available.
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient};
use llm_router::register::register_router;
use provider_xai::register::register_provider;
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
    url: String, // http://addr/v1/chat/completions — what goes in the config slice
    handle: tokio::task::JoinHandle<()>,
}

#[tokio::test(flavor = "multi_thread")]
async fn discovery_feedback_recovery_authorization_and_stale_attempts() {
    use iii_sdk::{errors::Error, protocol::RegisterTriggerInput, RegisterFunction};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let engine = engine_or_skip!();
    let iii = register_worker(&engine.url, test_init_options());
    register_router(iii.clone()).await.unwrap();
    let registered = call(
        &iii,
        "router::provider::register",
        serde_json::to_value(provider_xai::register::declaration()).unwrap(),
    )
    .await
    .unwrap();
    let token = registered["registration_token"].as_str().unwrap();
    let (events_tx, mut events_rx) = tokio::sync::mpsc::unbounded_channel();
    iii.register_function(
        "probe::discovery_event",
        RegisterFunction::new_async(move |input: Value| {
            let events_tx = events_tx.clone();
            async move {
                let _ = events_tx.send(input);
                Ok::<_, Error>(json!({}))
            }
        }),
    );
    iii.register_trigger(RegisterTriggerInput::new(
        "router::models::changed",
        "probe::discovery_event",
        json!({}),
    ))
    .unwrap();
    // A bus roundtrip ensures the fixture subscription is registered before production.
    call(&iii, "router::provider::list", json!({}))
        .await
        .unwrap();
    provider_xai::state::store_token(&iii, token).await.unwrap();
    let response = Arc::new(std::sync::Mutex::new((403, json!({"code":"permission-denied", "error":"team-PRIVATE has no credits or reached its monthly spending limit; sk-PRIVATE https://private.invalid"}).to_string())));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!(
        "http://{}/v1/chat/completions",
        listener.local_addr().unwrap()
    );
    let current = response.clone();
    let server = tokio::spawn(async move {
        loop {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut line = String::new();
            loop {
                line.clear();
                assert!(reader.read_line(&mut line).await.unwrap() > 0);
                if line == "\r\n" {
                    break;
                }
            }
            let (status, body) = current.lock().unwrap().clone();
            socket.write_all(format!("HTTP/1.1 {status} Stub\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
        }
    });
    configure_stub_key(&iii, &url).await;
    call(
        &iii,
        "router::on_config_changed",
        json!({"id":"llm-router"}),
    )
    .await
    .unwrap();
    let http = reqwest::Client::new();
    let failed = provider_xai::discovery::refresh_models(&iii, &http)
        .await
        .unwrap();
    assert!(!failed.refreshed.ok);
    assert_eq!(
        failed.discovery,
        llm_router::types::router::DiscoveryOutcome::Billing
    );
    let event = tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(event["count"], 0);
    let list = call(&iii, "router::provider::list", json!({}))
        .await
        .unwrap();
    assert_eq!(list["providers"][0]["discovery"]["outcome"], "billing");
    assert_eq!(list["providers"][0]["configured"], true);
    let wire = list.to_string();
    assert!(!wire.contains("PRIVATE"));
    assert!(!wire.contains("private.invalid"));
    let restored = llm_router::registry::store::RegistryStore::new(iii.clone());
    restored.load().await.unwrap();
    assert_eq!(
        restored
            .get("xai")
            .await
            .unwrap()
            .discovery
            .unwrap()
            .outcome,
        failed.discovery
    );

    let begin = || {
        call(
            &iii,
            "router::provider::resolve",
            json!({"id":"xai", "token":token, "begin_discovery":true}),
        )
    };
    assert!(call(
        &iii,
        "router::provider::resolve",
        json!({"id":"xai", "begin_discovery":true})
    )
    .await
    .is_err());
    let first = begin().await.unwrap()["discovery_attempt"].clone();
    let latest = begin().await.unwrap()["discovery_attempt"].clone();
    let report = |attempt: Value| json!({"provider":"xai", "token":token, "models":[], "discovery":{"attempt":attempt, "outcome":"empty", "http_status":200}});
    let mut unauthorized = report(latest.clone());
    unauthorized["token"] = json!("wrong-token");
    assert!(call(&iii, "router::models::reconcile", unauthorized)
        .await
        .is_err());
    assert!(call(&iii, "router::models::reconcile", report(first))
        .await
        .is_err());
    let mut unsafe_code = report(latest.clone());
    unsafe_code["discovery"]["code"] = json!("sk-PRIVATE");
    assert!(call(&iii, "router::models::reconcile", unsafe_code)
        .await
        .is_err());
    call(&iii, "router::models::reconcile", report(latest))
        .await
        .unwrap();
    let event = tokio::time::timeout(Duration::from_secs(5), events_rx.recv())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        event["count"], 0,
        "empty recovery must emit even when count stays zero"
    );
    assert_eq!(
        call(&iii, "router::provider::list", json!({}))
            .await
            .unwrap()["providers"][0]["discovery"]["outcome"],
        "empty"
    );

    *response.lock().unwrap() = (200, json!({"data":[{"id":"grok-4"}]}).to_string());
    assert!(
        provider_xai::discovery::refresh_models(&iii, &http)
            .await
            .unwrap()
            .refreshed
            .ok
    );
    *response.lock().unwrap() = (503, "private outage".into());
    let transient = provider_xai::discovery::refresh_models(&iii, &http)
        .await
        .unwrap();
    assert!(!transient.refreshed.ok);
    assert_eq!(transient.refreshed.count, 1);
    assert_eq!(
        call(&iii, "router::provider::list", json!({}))
            .await
            .unwrap()["providers"][0]["discovery"]["stale"],
        true
    );
    let old_config = begin().await.unwrap()["discovery_attempt"].clone();
    call(&iii, "configuration::set", json!({"id":"llm-router", "value":{"providers":{"xai":{"api_key":"new-fixture-key", "api_url":url}}}})).await.unwrap();
    call(
        &iii,
        "router::on_config_changed",
        json!({"id":"llm-router"}),
    )
    .await
    .unwrap();
    assert!(call(&iii, "router::models::reconcile", report(old_config))
        .await
        .is_err());
    *response.lock().unwrap() = (200, json!({"data":[]}).to_string());
    let empty = provider_xai::discovery::refresh_models(&iii, &http)
        .await
        .unwrap();
    assert!(empty.refreshed.ok);
    assert_eq!(empty.refreshed.count, 0);
    let list = call(&iii, "router::provider::list", json!({}))
        .await
        .unwrap();
    assert_eq!(list["providers"][0]["discovery"]["outcome"], "empty");
    assert_eq!(list["providers"][0]["discovery"]["stale"], false);
    assert!(list["providers"][0]["discovery"].get("code").is_none());
    // Legacy callers still replace their own slice without a discovery report.
    call(
        &iii,
        "router::models::reconcile",
        json!({"provider":"xai", "token":token, "models":[]}),
    )
    .await
    .unwrap();
    server.abort();
}

impl Drop for StubUpstream {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

const STUB_SSE: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":2,\"prompt_tokens_details\":{\"cached_tokens\":4}}}\n\ndata: [DONE]\n\n";

const STUB_401: &str = "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"message\":\"Incorrect API key provided.\",\"type\":\"invalid_request_error\",\"code\":\"invalid_api_key\"}}";

const STUB_MODELS: &str = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"data\":[{\"id\":\"grok-4\",\"object\":\"model\"},{\"id\":\"grok-4-0709\",\"object\":\"model\"},{\"id\":\"grok-3-mini-0625\",\"object\":\"model\"},{\"id\":\"grok-9-foo\",\"object\":\"model\"},{\"id\":\"grok-2-1212\",\"object\":\"model\"},{\"id\":\"text-embedding-3-large\",\"object\":\"model\"}]}";

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
        url: format!("http://{addr}/v1/chat/completions"),
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
            .is_some_and(|p| p.iter().any(|x| x["id"] == "xai"));
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

/// Point the xai slice at the stub.
async fn configure_stub_key(router_iii: &IIIClient, stub_url: &str) {
    call(
        router_iii,
        "configuration::set",
        json!({ "id": "llm-router", "value": { "providers": {
            "xai": { "api_key": "sk-test", "api_url": stub_url }
        } } }),
    )
    .await
    .expect("config set");
}

/// Pull the live (stubbed) list into the catalog and wait until routing can
/// see it — the declaration carries no models, so tests that route by
/// catalog ownership must refresh first.
async fn refresh_and_wait(router_iii: &IIIClient, provider_iii: &IIIClient, expect_id: &str) {
    let res = call(provider_iii, "provider::xai::refresh_models", json!({}))
        .await
        .expect("refresh succeeds");
    assert_eq!(res["ok"], true, "refresh response: {res}");
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let list = call(
            router_iii,
            "router::models::list",
            json!({ "provider": "xai" }),
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
        json!({ "provider": "xai" }),
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
        json!({ "scope": "provider-xai", "key": "registration_token" }),
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
    refresh_and_wait(&router_iii, &provider_iii, "grok-4").await;

    let consumer = register_worker(&engine.url, test_init_options());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let res = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "model": "grok-4",
                "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 }],
            }),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await
        .expect("chat succeeds");
    assert_eq!(res["ok"], true, "chat response: {res}");
    assert_eq!(res["provider"], "xai");
    assert_eq!(res["stop_reason"], "end");
    // the router filled cost_usd from the curated pricing (8 in + 4 cached + 2 out)
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
    assert_eq!(last["message"]["native_stop_reason"], "stop");
    assert_eq!(last["message"]["usage"]["cache_read"], 4);

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
    let res = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "model": "grok-4",
                "provider": "xai",
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
async fn refresh_models_reconciles_filtered_live_catalog() {
    let engine = engine_or_skip!();
    let stub = stub_upstream(STUB_SSE).await;
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;
    configure_stub_key(&router_iii, &stub.url).await;
    refresh_and_wait(&router_iii, &provider_iii, "grok-4").await;

    let list = call(
        &router_iii,
        "router::models::list",
        json!({ "provider": "xai" }),
    )
    .await
    .unwrap();
    let models = list["models"].as_array().unwrap().clone();
    let ids: Vec<&str> = models.iter().filter_map(|m| m["id"].as_str()).collect();

    // exactly the filtered live list: the undated alias wins over its dated
    // snapshot, a dated id with no live alias stays, legacy generations and
    // non-chat ids are gone
    assert!(ids.contains(&"grok-4"), "got {ids:?}");
    assert!(
        !ids.contains(&"grok-4-0709"),
        "dated snapshot should fold into the live alias: {ids:?}"
    );
    assert!(ids.contains(&"grok-3-mini-0625"), "got {ids:?}");
    assert!(
        !ids.contains(&"grok-2-1212"),
        "legacy generation should be filtered: {ids:?}"
    );
    assert!(
        !ids.contains(&"text-embedding-3-large"),
        "embedding model should be filtered: {ids:?}"
    );

    // known family carries the local metadata; unknown family stays default
    let known = models.iter().find(|m| m["id"] == "grok-4").unwrap();
    assert_eq!(known["context_window"], 256_000);
    assert_eq!(known["supports_structured_output"], true);
    assert!(known["pricing"]["input"].as_f64().is_some_and(|p| p > 0.0));
    let unknown = models.iter().find(|m| m["id"] == "grok-9-foo").unwrap();
    assert_eq!(unknown["context_window"], 131_072);

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
            .is_some_and(|p| p.iter().any(|x| x["id"] == "xai"));
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

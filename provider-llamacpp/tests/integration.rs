//! Engine-backed integration suite — real engine, real router, real provider,
//! stubbed upstream. Self-skips when no engine is available.
use std::io::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use llm_router::register::register_router;
use provider_llamacpp::register::register_provider;
use serde_json::{json, Value};

// ── engine bootstrap ────────────────────────────────────────────────────────

struct Engine {
    url: String,
    child: std::process::Child,
    dir: std::path::PathBuf,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn engine_bin() -> Option<std::path::PathBuf> {
    if let Ok(p) = std::env::var("III_ENGINE_BIN") {
        return Some(p.into());
    }
    let on_path = std::process::Command::new("iii")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    on_path.then(|| "iii".into())
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Spawn a minimal engine in a temp dir; poll until WS-reachable.
/// None = no engine available on this host → the caller self-skips.
async fn spawn_engine() -> Option<Engine> {
    let bin = engine_bin()?;
    let port = free_port();
    let dir = std::env::temp_dir().join(format!("provider-llamacpp-it-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let config = format!(
        r#"workers:
  - name: iii-worker-manager
    config:
      port: {port}
  - name: iii-pubsub
    config:
      adapter:
        name: local
  - name: configuration
    config:
      adapter:
        name: fs
        config:
          directory: {dir}/configuration
      ttl_seconds: 0
  - name: iii-state
    config:
      adapter:
        name: kv
        config:
          file_path: {dir}/state_store.db
          store_method: file_based
"#,
        port = port,
        dir = dir.display(),
    );
    let config_path = dir.join("config.yaml");
    std::fs::File::create(&config_path)
        .and_then(|mut f| f.write_all(config.as_bytes()))
        .expect("write config");

    let child = std::process::Command::new(&bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config_path)
        .current_dir(&dir)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn engine");

    let url = format!("ws://127.0.0.1:{port}");
    let probe = register_worker(&url, InitOptions::default());
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let ready = probe
            .trigger(TriggerRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(1000),
            })
            .await
            .is_ok();
        if ready {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "engine did not become ready in 15s"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    probe.shutdown();

    Some(Engine { url, child, dir })
}

/// Self-skip macro: returns from the test when no engine is available.
macro_rules! engine_or_skip {
    () => {
        match spawn_engine().await {
            Some(e) => e,
            None => {
                eprintln!("skipping: no iii engine (set III_ENGINE_BIN or put `iii` on PATH)");
                return;
            }
        }
    };
}

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

/// Serves `chat_response` for any `/v1/chat/completions` request, and canned
/// `/v1/models` + `/props` bodies for discovery — a single llama-server stub
/// good enough for both the streaming and the catalog-discovery scenarios.
/// Captures each request's raw head (request line + headers) so tests can
/// assert on what was (or wasn't) sent, e.g. no `Authorization` header when
/// no credential is configured.
struct StubUpstream {
    url: String, // http://addr/v1/chat/completions — what goes in the config slice
    handle: tokio::task::JoinHandle<()>,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
}

impl Drop for StubUpstream {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

const STUB_SSE: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":2,\"prompt_tokens_details\":{\"cached_tokens\":4}}}\n\ndata: [DONE]\n\n";

const STUB_401: &str = "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"message\":\"Incorrect API key provided.\",\"type\":\"invalid_request_error\",\"code\":\"invalid_api_key\"}}";

const STUB_MODEL_ID: &str = "llama-test-model";

const STUB_MODELS_BODY: &str = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"object\":\"list\",\"data\":[{\"id\":\"llama-test-model\",\"object\":\"model\",\"owned_by\":\"llamacpp\"}]}";

const STUB_PROPS_BODY: &str = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"default_generation_settings\":{\"n_ctx\":8192},\"modalities\":{\"vision\":false,\"audio\":false}}";

async fn stub_upstream(chat_response: &'static str) -> StubUpstream {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let requests = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let requests2 = requests.clone();
    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                break;
            };
            let requests = requests2.clone();
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65536];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).into_owned();
                let path = head
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("")
                    .to_string();
                requests.lock().unwrap().push(head);
                let body = if path.starts_with("/v1/models") {
                    STUB_MODELS_BODY
                } else if path.starts_with("/props") {
                    STUB_PROPS_BODY
                } else {
                    chat_response
                };
                let _ = sock.write_all(body.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    StubUpstream {
        url: format!("http://{addr}/v1/chat/completions"),
        handle,
        requests,
    }
}

// ── boot + config ───────────────────────────────────────────────────────────

/// Boot router + provider on one engine; wait until the provider is listed.
async fn boot_stack(engine_url: &str) -> (IIIClient, IIIClient) {
    let router_iii = register_worker(engine_url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots");
    let provider_iii = register_worker(engine_url, InitOptions::default());
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
            .is_some_and(|p| p.iter().any(|x| x["id"] == "llamacpp"));
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

/// Point the llamacpp slice at the stub, with an (unnecessary but harmless)
/// api_key configured.
async fn configure_stub_key(router_iii: &IIIClient, stub_url: &str) {
    call(
        router_iii,
        "configuration::set",
        json!({ "id": "llm-router", "value": { "providers": {
            "llamacpp": { "api_key": "sk-test", "api_url": stub_url }
        } } }),
    )
    .await
    .expect("config set");
}

/// Point the llamacpp slice at the stub with NO credential — the common
/// local llama.cpp setup (no `--api-key` on the server).
async fn configure_stub_no_key(router_iii: &IIIClient, stub_url: &str) {
    call(
        router_iii,
        "configuration::set",
        json!({ "id": "llm-router", "value": { "providers": {
            "llamacpp": { "api_url": stub_url }
        } } }),
    )
    .await
    .expect("config set");
}

/// Discover the live catalog and wait until routing can see it.
async fn refresh_and_wait(router_iii: &IIIClient, provider_iii: &IIIClient, expect_id: &str) {
    let res = call(
        provider_iii,
        "provider::llamacpp::refresh_models",
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
            json!({ "provider": "llamacpp" }),
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
async fn provider_registers_with_persisted_token_and_discovers_catalog_without_a_credential() {
    // A real key exported on the host would leak into the in-process router's
    // env-var fallback and defeat the no-credential assertions below.
    std::env::remove_var("LLAMACPP_API_KEY");
    let engine = engine_or_skip!();
    let stub = stub_upstream(STUB_SSE).await;
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;
    configure_stub_no_key(&router_iii, &stub.url).await;

    // Unlike every cloud provider here, no credential is required at all:
    // discovery succeeds and reconciles the stub's one model.
    let res = call(
        &provider_iii,
        "provider::llamacpp::refresh_models",
        json!({}),
    )
    .await
    .expect("refresh succeeds without a credential");
    assert_eq!(res["ok"], true, "refresh response: {res}");
    assert_eq!(
        res["count"], 1,
        "no-credential discovery still lands: {res}"
    );
    let list = call(
        &router_iii,
        "router::models::list",
        json!({ "provider": "llamacpp" }),
    )
    .await
    .unwrap();
    let ids: Vec<&str> = list["models"]
        .as_array()
        .map(|a| a.iter().filter_map(|m| m["id"].as_str()).collect())
        .unwrap_or_default();
    assert_eq!(ids, vec![STUB_MODEL_ID]);

    // The registration token lands in the provider's state scope; the write
    // races provider visibility (the router lists the provider before the
    // register response returns), so poll.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let token = call(
            &provider_iii,
            "state::get",
            json!({ "scope": "provider-llamacpp", "key": "registration_token" }),
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
async fn chat_streams_end_to_end_with_no_credential_and_no_authorization_header() {
    let engine = engine_or_skip!();
    let stub = stub_upstream(STUB_SSE).await;
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;
    configure_stub_no_key(&router_iii, &stub.url).await;
    refresh_and_wait(&router_iii, &provider_iii, STUB_MODEL_ID).await;

    let consumer = register_worker(&engine.url, InitOptions::default());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let res = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "model": STUB_MODEL_ID,
                "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 }],
            }),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await
        .expect("chat succeeds");
    assert_eq!(res["ok"], true, "chat response: {res}");
    assert_eq!(res["provider"], "llamacpp");
    assert_eq!(res["stop_reason"], "end");

    let _ = tokio::time::timeout(Duration::from_secs(5), pump).await;
    let frames = frames.lock().unwrap();
    let first: Value = serde_json::from_str(frames.first().unwrap()).unwrap();
    assert_eq!(first["type"], "start");
    let last: Value = serde_json::from_str(frames.last().unwrap()).unwrap();
    assert_eq!(last["type"], "done");
    assert_eq!(last["message"]["content"][0]["text"], "Hello");
    assert_eq!(last["message"]["native_stop_reason"], "stop");
    assert_eq!(last["message"]["usage"]["cache_read"], 4);

    // The main behavioral divergence from every other provider here: no
    // credential configured → no Authorization header sent at all.
    let requests = stub.requests.lock().unwrap();
    let chat_request = requests
        .iter()
        .find(|r| {
            r.lines()
                .next()
                .is_some_and(|l| l.contains("/v1/chat/completions"))
        })
        .expect("chat request captured");
    assert!(
        !chat_request.to_ascii_lowercase().contains("authorization:"),
        "no credential configured → no Authorization header, got:\n{chat_request}"
    );

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

    let consumer = register_worker(&engine.url, InitOptions::default());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let res = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "model": STUB_MODEL_ID,
                "provider": "llamacpp",
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
async fn refresh_models_discovers_live_catalog_via_v1_models_and_props() {
    let engine = engine_or_skip!();
    let stub = stub_upstream(STUB_SSE).await;
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;
    configure_stub_key(&router_iii, &stub.url).await;
    refresh_and_wait(&router_iii, &provider_iii, STUB_MODEL_ID).await;

    let list = call(
        &router_iii,
        "router::models::list",
        json!({ "provider": "llamacpp" }),
    )
    .await
    .unwrap();
    let models = list["models"].as_array().unwrap().clone();
    let ids: Vec<&str> = models.iter().filter_map(|m| m["id"].as_str()).collect();
    assert_eq!(
        ids,
        vec![STUB_MODEL_ID],
        "live listing, not a curated table"
    );

    // rows carry the /props-derived runtime context, not a hand-maintained
    // pricing table (self-hosted → no cost).
    let model = &models[0];
    assert_eq!(model["context_window"], 8192, "from /props' n_ctx");
    assert_eq!(model["supports_structured_output"], true);
    assert_eq!(model["supports_tools"], true);
    assert_eq!(model["supports_vision"], false);
    assert!(model.get("pricing").map_or(true, |p| p.is_null()));

    router_iii.shutdown();
    provider_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_redeclares_on_router_ready() {
    let engine = engine_or_skip!();
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;

    // simulate a router restart that LOST its registry: wipe the persisted
    // record so only a real re-declare (router::ready → on_router_ready →
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
    let router2 = register_worker(&engine.url, InitOptions::default());
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
            .is_some_and(|p| p.iter().any(|x| x["id"] == "llamacpp"));
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

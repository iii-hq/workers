//! Engine-backed integration suite — real engine, real router, real provider,
//! stubbed upstream. Self-skips when no engine is available.
use std::io::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use llm_router::register::register_router;
use provider_github_copilot::register::register_provider;
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
    let dir = std::env::temp_dir().join(format!(
        "provider-github-copilot-it-{}",
        uuid::Uuid::new_v4()
    ));
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

/// Hermetic credential state per test. Env is process-wide, so the returned
/// guard serializes the whole suite; `with_bearer` short-circuits the token
/// exchange via the GITHUB_COPILOT_TOKEN ready-bearer path; either way the
/// local editor credential import is disabled so a dev machine's real login
/// never leaks into the suite.
static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn set_credential_env(with_bearer: bool) {
    std::env::set_var("GITHUB_COPILOT_NO_LOCAL_IMPORT", "1");
    std::env::remove_var("GITHUB_COPILOT_OAUTH_TOKEN");
    if with_bearer {
        std::env::set_var("GITHUB_COPILOT_TOKEN", "stub-bearer");
    } else {
        std::env::remove_var("GITHUB_COPILOT_TOKEN");
    }
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

/// Routes by request line; loops over connections until dropped.
struct StubUpstream {
    url: String, // http://addr/v1/chat/completions — what goes in the config slice
    handle: tokio::task::JoinHandle<()>,
}

impl Drop for StubUpstream {
    fn drop(&mut self) {
        self.handle.abort();
    }
}

const STUB_SSE: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":2,\"prompt_tokens_details\":{\"cached_tokens\":4}}}\n\ndata: [DONE]\n\n";

const STUB_401: &str = "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"code\":401,\"message\":\"unauthorized: token expired\"}}";

// Copilot-shaped listing. gpt-agentic: full metadata, admitted.
// bare-chat: sparse but chat+tools, admitted with conservative defaults.
// chatty: no tool support -> filtered. embed: non-chat type -> filtered.
// locked: policy disabled for this account -> filtered by admission.
// premium-only: passes every listing gate, but the upstream refuses it
// (model_not_supported) -> filtered by the discovery probe.
const STUB_MODELS: &str = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"data\":[\
{\"id\":\"gpt-agentic\",\"name\":\"GPT Agentic\",\"model_picker_enabled\":false,\
\"capabilities\":{\"type\":\"chat\",\"limits\":{\"max_context_window_tokens\":200000,\"max_output_tokens\":64000},\
\"supports\":{\"tool_calls\":true,\"streaming\":true,\"vision\":true,\"structured_outputs\":true}}},\
{\"id\":\"bare-chat\",\"capabilities\":{\"type\":\"chat\",\"supports\":{\"tool_calls\":true}}},\
{\"id\":\"chatty\",\"capabilities\":{\"type\":\"chat\",\"supports\":{\"tool_calls\":false}}},\
{\"id\":\"embed\",\"capabilities\":{\"type\":\"embeddings\",\"supports\":{}}},\
{\"id\":\"locked\",\"policy\":{\"state\":\"disabled\"},\"capabilities\":{\"type\":\"chat\",\"supports\":{\"tool_calls\":true}}},\
{\"id\":\"premium-only\",\"capabilities\":{\"type\":\"chat\",\"supports\":{\"tool_calls\":true}}}]}";

/// What the upstream answers for a model this plan may not call.
const STUB_NOT_SUPPORTED: &str = "HTTP/1.1 400 Bad Request\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"message\":\"The requested model is not supported.\",\"code\":\"model_not_supported\",\"param\":\"model\",\"type\":\"invalid_request_error\"}}";

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
                // One read can land mid-request: keep reading until the
                // headers are complete and any body has arrived, so routing
                // on the requested model is not a race.
                let mut acc = Vec::new();
                let mut chunk = vec![0u8; 65536];
                loop {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        break;
                    }
                    acc.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&acc);
                    let headers_done = text.contains("\r\n\r\n");
                    let body_len = text
                        .to_ascii_lowercase()
                        .split("content-length:")
                        .nth(1)
                        .and_then(|rest| rest.split(['\r', '\n']).next())
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    let body_start = text.find("\r\n\r\n").map(|i| i + 4).unwrap_or(acc.len());
                    if headers_done && acc.len() >= body_start + body_len {
                        break;
                    }
                }
                let head = String::from_utf8_lossy(&acc);
                let response = if head.contains("GET ") && head.contains("/models") {
                    STUB_MODELS
                } else if head.contains("premium-only") {
                    STUB_NOT_SUPPORTED
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
            .is_some_and(|p| p.iter().any(|x| x["id"] == "github-copilot"));
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

/// Point the github-copilot slice at the stub (api_url only — the bearer
/// comes from the GITHUB_COPILOT_TOKEN env, skipping the token exchange).
async fn configure_stub_key(router_iii: &IIIClient, stub_url: &str) {
    call(
        router_iii,
        "configuration::set",
        json!({ "id": "llm-router", "value": { "providers": {
            "github-copilot": { "api_url": stub_url }
        } } }),
    )
    .await
    .expect("config set");
}

/// Pull the live (stubbed) list into the catalog and wait until routing can
/// see it — the declaration carries no models, so tests that route by
/// catalog ownership must refresh first. The refresh itself is retried: the
/// router's config snapshot syncs off the configuration:updated trigger, so
/// the first attempt can run before the stub api_url is visible.
async fn refresh_and_wait(router_iii: &IIIClient, provider_iii: &IIIClient, expect_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let refreshed = call(
            provider_iii,
            "provider::github-copilot::refresh_models",
            json!({}),
        )
        .await
        .is_ok_and(|r| r["ok"] == true && r["count"].as_u64().unwrap_or(0) > 0);
        if refreshed {
            let list = call(
                router_iii,
                "router::models::list",
                json!({ "provider": "github-copilot" }),
            )
            .await
            .unwrap();
            let present = list["models"]
                .as_array()
                .is_some_and(|a| a.iter().any(|m| m["id"] == expect_id));
            if present {
                return;
            }
        }
        assert!(
            Instant::now() < deadline,
            "catalog never gained {expect_id}"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}

// ── scenarios ───────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn provider_registers_with_persisted_token_and_live_only_catalog() {
    let _env = ENV_LOCK.lock().await;
    set_credential_env(false);
    let engine = engine_or_skip!();
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;

    // No static slice and no sign-in anywhere: the catalog stays empty
    // (models come from GET /models only once a credential exists).
    let list = call(
        &router_iii,
        "router::models::list",
        json!({ "provider": "github-copilot" }),
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
        json!({ "scope": "provider-github-copilot", "key": "registration_token" }),
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
async fn chat_streams_end_to_end_through_the_ready_bearer_path() {
    let _env = ENV_LOCK.lock().await;
    set_credential_env(true);
    let engine = engine_or_skip!();
    let stub = stub_upstream(STUB_SSE).await;
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;
    configure_stub_key(&router_iii, &stub.url).await;
    // catalog-ownership routing needs the live slice in place
    refresh_and_wait(&router_iii, &provider_iii, "copilot/gpt-agentic").await;

    let consumer = register_worker(&engine.url, InitOptions::default());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let res = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "model": "copilot/gpt-agentic",
                "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 }],
            }),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await
        .expect("chat succeeds");
    assert_eq!(res["ok"], true, "chat response: {res}");
    assert_eq!(res["provider"], "github-copilot");
    assert_eq!(res["stop_reason"], "end");
    // subscription metering: no per-token pricing, so no cost fill — but
    // native token counts still ride the final usage chunk.
    assert!(
        res["usage"]["cost_usd"].is_null(),
        "no cost on this wire: {res}"
    );
    assert_eq!(res["usage"]["input"], 12);

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
    let _env = ENV_LOCK.lock().await;
    set_credential_env(true);
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
                "model": "copilot/gpt-agentic",
                "provider": "github-copilot",
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
async fn refresh_models_reconciles_admitted_live_catalog_with_metadata() {
    let _env = ENV_LOCK.lock().await;
    set_credential_env(true);
    let engine = engine_or_skip!();
    let stub = stub_upstream(STUB_SSE).await;
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;
    configure_stub_key(&router_iii, &stub.url).await;
    refresh_and_wait(&router_iii, &provider_iii, "copilot/gpt-agentic").await;

    let list = call(
        &router_iii,
        "router::models::list",
        json!({ "provider": "github-copilot" }),
    )
    .await
    .unwrap();
    let models = list["models"].as_array().unwrap().clone();
    let ids: Vec<&str> = models.iter().filter_map(|m| m["id"].as_str()).collect();

    // exactly the admitted live list under the copilot/ prefix: chat+tools
    // models stay; tool-less, non-chat, and plan-disabled rows are gone.
    assert!(ids.contains(&"copilot/gpt-agentic"), "got {ids:?}");
    assert!(
        ids.contains(&"copilot/bare-chat"),
        "sparse chat+tools model still routes: {ids:?}"
    );
    assert!(
        !ids.contains(&"copilot/chatty"),
        "tool-less model should be filtered: {ids:?}"
    );
    assert!(
        !ids.contains(&"copilot/embed"),
        "non-chat model should be filtered: {ids:?}"
    );
    assert!(
        !ids.contains(&"copilot/locked"),
        "policy-disabled model should be filtered: {ids:?}"
    );
    assert!(
        !ids.contains(&"copilot/premium-only"),
        "model the upstream refuses should be dropped by the probe: {ids:?}"
    );

    // the listing's own metadata landed on the record; the sparse row got
    // conservative defaults
    let known = models
        .iter()
        .find(|m| m["id"] == "copilot/gpt-agentic")
        .unwrap();
    assert_eq!(known["display_name"], "GPT Agentic");
    assert_eq!(known["context_window"], 200_000);
    assert_eq!(known["max_output_tokens"], 64_000);
    assert_eq!(known["supports_vision"], true);
    assert!(
        known["supports_thinking"].is_null(),
        "no effort surface on this wire"
    );
    assert_eq!(known["supports_structured_output"], true);
    // subscription metering: no per-token pricing on the record
    assert!(known["pricing"].is_null());
    let unknown = models
        .iter()
        .find(|m| m["id"] == "copilot/bare-chat")
        .unwrap();
    assert_eq!(unknown["context_window"], 32_768);

    router_iii.shutdown();
    provider_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_redeclares_on_router_ready() {
    let _env = ENV_LOCK.lock().await;
    set_credential_env(false);
    let engine = engine_or_skip!();
    let (router_iii, provider_iii) = boot_stack(&engine.url).await;

    // simulate a router restart: drop the first router, boot a fresh one
    router_iii.shutdown();
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
            .is_some_and(|p| p.iter().any(|x| x["id"] == "github-copilot"));
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

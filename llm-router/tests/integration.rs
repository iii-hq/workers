//! Engine-backed integration suite — every bus-shaped flow runs against a
//! real iii engine (binary-worker.md § 9): registration + token gate, resolve
//! precedence, paste-a-key, chat relay, cancellation, abort, restart.
//!
//! **Self-skips** when no engine is available (storage-worker pattern):
//! set `III_ENGINE_BIN=/path/to/iii` or have `iii` on PATH.
use std::collections::HashMap;
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::channel::StreamChannelRef;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use llm_router::register::register_router;
use llm_router::registry::store::RegistryStore;
use llm_router::types::router::ProviderDeclaration;
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

/// Bare engine mirroring CI's interface-boot smoke (`workers: []`): builtin
/// daemons only — no `iii-state`, no `iii-pubsub`. Port pinned through
/// iii-worker-manager so parallel tests don't collide on the default port.
async fn spawn_bare_engine() -> Option<Engine> {
    let config_for = |port: u16, _dir: &std::path::Path| {
        format!(
            r#"workers:
  - name: iii-worker-manager
    config:
      port: {port}
"#
        )
    };
    spawn_engine_with(config_for).await
}

/// Spawn a minimal engine in a temp dir; poll until WS-reachable.
/// None = no engine available on this host → the caller self-skips.
async fn spawn_engine() -> Option<Engine> {
    let config_for = |port: u16, dir: &std::path::Path| {
        format!(
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
        )
    };
    spawn_engine_with(config_for).await
}

/// Shared engine bootstrap: pick a port + temp dir, write the config the
/// caller composes for them, spawn, poll until WS-reachable.
async fn spawn_engine_with(
    config_for: impl FnOnce(u16, &std::path::Path) -> String,
) -> Option<Engine> {
    let bin = engine_bin()?;
    let port = free_port();
    let dir = std::env::temp_dir().join(format!("llm-router-it-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");

    let config = config_for(port, &dir);
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

/// Same self-skip, for the bare (no iii-state / no iii-pubsub) engine.
macro_rules! bare_engine_or_skip {
    () => {
        match spawn_bare_engine().await {
            Some(e) => e,
            None => {
                eprintln!("skipping: no iii engine (set III_ENGINE_BIN or put `iii` on PATH)");
                return;
            }
        }
    };
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
}

async fn call_until(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
    predicate: impl Fn(&Value) -> bool,
) -> Value {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let value = call(iii, function_id, payload.clone())
            .await
            .unwrap_or_else(|error| panic!("{function_id} failed while polling: {error}"));
        if predicate(&value) {
            return value;
        }
        assert!(
            Instant::now() < deadline,
            "{function_id} did not observe the reactive configuration update; last response: {value}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// Register the minimal state surface on a bare engine and fail exactly one
/// numbered write. Used to prove router stores do not publish uncommitted
/// in-memory state when persistence rejects a mutation.
async fn start_flaky_state(url: &str, fail_on_set_call: u64) -> (IIIClient, Arc<AtomicU64>) {
    let iii = register_worker(url, InitOptions::default());
    let values = Arc::new(std::sync::Mutex::new(HashMap::<String, Value>::new()));
    let get_values = values.clone();
    iii.register_function(
        "state::get",
        RegisterFunction::new_async(move |input: Value| {
            let values = get_values.clone();
            async move {
                let key = input["key"].as_str().unwrap_or_default();
                Ok::<Value, Error>(values.lock().unwrap().get(key).cloned().unwrap_or_default())
            }
        }),
    );
    let set_calls = Arc::new(AtomicU64::new(0));
    let calls = set_calls.clone();
    let set_values = values;
    iii.register_function(
        "state::set",
        RegisterFunction::new_async(move |input: Value| {
            let call = calls.fetch_add(1, Ordering::SeqCst) + 1;
            let values = set_values.clone();
            async move {
                if call == fail_on_set_call {
                    Err(Error::Handler("injected state write failure".into()))
                } else {
                    let key = input["key"].as_str().unwrap_or_default().to_string();
                    values.lock().unwrap().insert(key, input["value"].clone());
                    Ok::<Value, Error>(json!({ "ok": true }))
                }
            }
        }),
    );
    call_until(
        &iii,
        "engine::functions::list",
        json!({ "include_internal": true }),
        |value| {
            value["functions"].as_array().is_some_and(|functions| {
                ["state::get", "state::set"].iter().all(|id| {
                    functions
                        .iter()
                        .any(|function| function["function_id"] == *id)
                })
            })
        },
    )
    .await;
    (iii, set_calls)
}

fn remote_code(err: &Error) -> &str {
    match err {
        Error::Remote { code, .. } => code,
        _ => "",
    }
}

// ── live provider helper ────────────────────────────────────────────────────

#[derive(Clone, Default)]
struct ProviderOptions {
    ping_forever: bool,
    /// Keep the function handler alive after the terminal frame. A compliant
    /// router must complete from the stream terminal, not this RPC lifetime.
    done_linger_ms: Option<u64>,
    credential_env_var: Option<String>,
    supports_model_listing: bool,
    /// model ids returned by provider::real::refresh_models
    discovered: Vec<String>,
}

struct LiveProvider {
    iii: IIIClient,
    token: String,
    stream_calls: Arc<AtomicU64>,
    write_failed: Arc<AtomicBool>,
    fail_at_ms: Arc<AtomicU64>,
}

fn live_provider_declaration(opts: &ProviderOptions, token: Option<String>) -> Value {
    let mut payload = json!({
        "id": "real",
        "credential_env_var": opts.credential_env_var,
        "defaults": { "api_url": "https://api.example.test/v1", "max_tokens": 8192 },
        "supports_model_listing": opts.supports_model_listing,
        "models": [{ "id": "live-1", "provider": "real", "context_window": 100000, "max_output_tokens": 8192 }]
    });
    if let Some(token) = token {
        payload["token"] = json!(token);
    }
    payload
}

/// A live provider worker on its own connection: registers
/// provider::real::stream (+ refresh_models) and declares itself.
async fn start_live_provider(url: &str, opts: ProviderOptions) -> LiveProvider {
    let iii = register_worker(url, InitOptions::default());
    let write_failed = Arc::new(AtomicBool::new(false));
    let fail_at_ms = Arc::new(AtomicU64::new(0));
    let stream_calls = Arc::new(AtomicU64::new(0));
    let token_cell: Arc<std::sync::Mutex<Option<String>>> = Arc::default();

    let address = url.to_string();
    let wf = write_failed.clone();
    let fam = fail_at_ms.clone();
    let calls = stream_calls.clone();
    let ping_forever = opts.ping_forever;
    let done_linger_ms = opts.done_linger_ms;
    iii.register_function(
        "provider::real::stream",
        RegisterFunction::new_async(move |input: Value| {
            let address = address.clone();
            let (wf, fam) = (wf.clone(), fam.clone());
            let calls = calls.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let r: StreamChannelRef =
                    serde_json::from_value(input["writer_ref"].clone())
                        .map_err(|e| Error::Serde(e.to_string()))?;
                let writer = iii_sdk::channel::ChannelWriter::new(&address, &r);
                let model = input["model"].clone();
                let start = json!({ "type": "start", "partial": { "role": "assistant", "content": [], "stop_reason": "end", "model": model, "provider": "real", "timestamp": 1 } });
                writer
                    .send_message(&start.to_string())
                    .await
                    .map_err(|e| Error::Handler(e.to_string()))?;
                if ping_forever {
                    let begun = Instant::now();
                    loop {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        if writer
                            .send_message(&json!({ "type": "ping" }).to_string())
                            .await
                            .is_err()
                        {
                            wf.store(true, Ordering::SeqCst);
                            fam.store(begun.elapsed().as_millis() as u64, Ordering::SeqCst);
                            return Ok(json!({ "ok": true, "status": "aborted" }));
                        }
                    }
                }
                // Slim streaming shape (contract: deltas carry no partial;
                // boundary frames carry the cumulative snapshot).
                let message = json!({
                    "role": "assistant", "content": [{ "type": "text", "text": "live" }],
                    "stop_reason": "end", "model": model, "provider": "real", "timestamp": 2
                });
                let start_snapshot = json!({
                    "role": "assistant", "content": [], "stop_reason": "end",
                    "model": model, "provider": "real", "timestamp": 1
                });
                for frame in [
                    json!({ "type": "text_start", "partial": start_snapshot }),
                    json!({ "type": "text_delta", "delta": "li" }),
                    json!({ "type": "text_delta", "delta": "ve" }),
                    json!({ "type": "text_end", "partial": message }),
                    json!({ "type": "done", "message": message }),
                ] {
                    writer
                        .send_message(&frame.to_string())
                        .await
                        .map_err(|e| Error::Handler(e.to_string()))?;
                }
                let _ = writer.close().await;
                if let Some(delay_ms) = done_linger_ms {
                    tokio::time::sleep(Duration::from_millis(delay_ms)).await;
                }
                Ok(json!({ "ok": true }))
            }
        }),
    );

    if !opts.discovered.is_empty() {
        let iii2 = iii.clone();
        let token_for_refresh = token_cell.clone();
        let discovered = opts.discovered.clone();
        iii.register_function(
            "provider::real::refresh_models",
            RegisterFunction::new_async(move |_input: Value| {
                let iii = iii2.clone();
                let token = token_for_refresh.lock().unwrap().clone();
                let models: Vec<Value> = discovered
                    .iter()
                    .map(|id| json!({ "id": id, "provider": "real", "context_window": 100000, "max_output_tokens": 8192, "supports_vision": true }))
                    .collect();
                async move {
                    iii.trigger(TriggerRequest {
                        function_id: "router::models::reconcile".into(),
                        payload: json!({ "provider": "real", "token": token, "models": models }),
                        action: None,
                        timeout_ms: Some(5000),
                    })
                    .await?;
                    Ok::<Value, Error>(json!({ "ok": true }))
                }
            }),
        );
    }

    // Match the production provider lifecycle: keep a deterministic direct
    // ready handler so a restarted router can rediscover this still-live
    // provider even if the one-shot trigger binding was replayed too late.
    let iii_ready = iii.clone();
    let token_for_ready = token_cell.clone();
    let opts_for_ready = opts.clone();
    iii.register_function(
        "provider::real::on_router_ready",
        RegisterFunction::new_async(move |_input: Value| {
            let iii = iii_ready.clone();
            let token = token_for_ready.lock().unwrap().clone();
            let opts = opts_for_ready.clone();
            async move {
                let token = token.ok_or_else(|| {
                    Error::Handler("provider ready handler has no registration token".into())
                })?;
                iii.trigger(TriggerRequest {
                    function_id: "router::provider::register".into(),
                    payload: live_provider_declaration(&opts, Some(token)),
                    action: None,
                    timeout_ms: Some(5_000),
                })
                .await?;
                Ok::<Value, Error>(json!({ "ok": true }))
            }
        }),
    );
    let _ = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "router::ready".into(),
        function_id: "provider::real::on_router_ready".into(),
        config: json!({}),
        metadata: None,
    });

    // declare (with a short retry in case the router is still booting)
    let mut token = None;
    for _ in 0..50 {
        let res = call(
            &iii,
            "router::provider::register",
            live_provider_declaration(&opts, None),
        )
        .await;
        if let Ok(res) = res {
            token = res["registration_token"].as_str().map(String::from);
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let token = token.expect("provider declared");
    *token_cell.lock().unwrap() = Some(token.clone());

    LiveProvider {
        iii,
        token,
        stream_calls,
        write_failed,
        fail_at_ms,
    }
}

/// Consumer-side channel: collect frames + a pump that drives dispatch.
async fn consumer_channel(
    iii: &IIIClient,
) -> (
    StreamChannelRef,
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
        let _ = channel.reader.read_all().await; // drives text-message dispatch
    });
    (writer_ref, frames, pump)
}

// ── scenarios ───────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn end_to_end_relay_over_a_live_engine() {
    let engine = engine_or_skip!();

    let router_iii = register_worker(&engine.url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots");
    let _provider = start_live_provider(&engine.url, ProviderOptions::default()).await;

    let consumer = register_worker(&engine.url, InitOptions::default());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;

    let res = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({ "writer_ref": writer_ref, "model": "live-1", "messages": [] }),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await
        .expect("chat succeeds");
    assert_eq!(res["ok"], true, "chat response: {res}");
    assert_eq!(res["stop_reason"], "end");

    let _ = tokio::time::timeout(Duration::from_secs(5), pump).await;
    {
        let frames = frames.lock().unwrap();
        assert!(frames.len() >= 2, "want >=2 frames, got {}", frames.len());
        let last: Value = serde_json::from_str(frames.last().unwrap()).unwrap();
        assert_eq!(last["type"], "done");
        // Slim deltas must reach the consumer untouched: no partial key
        // materialized anywhere between provider and consumer channel.
        let deltas: Vec<Value> = frames
            .iter()
            .map(|f| serde_json::from_str::<Value>(f).unwrap())
            .filter(|v| v["type"] == "text_delta")
            .collect();
        assert_eq!(deltas.len(), 2, "want the 2 scripted slim deltas");
        for d in &deltas {
            assert!(
                d.get("partial").is_none(),
                "slim delta grew a partial in transit: {d}"
            );
        }
    }

    let completed = call(
        &consumer,
        "router::complete",
        json!({ "model": "live-1", "messages": [] }),
    )
    .await
    .expect("complete succeeds");
    assert_eq!(completed["message"]["content"][0]["text"], "live");

    consumer.shutdown();
    router_iii.shutdown();
}

/// `router::route` must preview exactly the provider `router::chat` would
/// execute on, and throw the same typed codes when nothing routes — consumers
/// pin the preview as the explicit `provider` on the chat call.
#[tokio::test(flavor = "multi_thread")]
async fn route_previews_the_same_provider_chat_executes() {
    let engine = engine_or_skip!();

    let router_iii = register_worker(&engine.url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots");
    let _provider = start_live_provider(&engine.url, ProviderOptions::default()).await;

    let consumer = register_worker(&engine.url, InitOptions::default());

    // catalog-owner routing: live-1 sits in provider "real"'s static slice.
    let route = call(&consumer, "router::route", json!({ "model": "live-1" }))
        .await
        .expect("route succeeds");
    assert_eq!(route["provider"], "real");
    assert_eq!(route["candidates"], json!(["real"]));

    // pinning the preview as the explicit provider executes on that provider.
    let (writer_ref, _frames, pump) = consumer_channel(&consumer).await;
    let res = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "model": "live-1",
                "provider": route["provider"],
                "messages": []
            }),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await
        .expect("chat succeeds");
    assert_eq!(res["ok"], true, "chat response: {res}");
    assert_eq!(res["provider"], route["provider"]);
    let _ = tokio::time::timeout(Duration::from_secs(5), pump).await;

    // an unrouteable model throws the same typed code the chat path throws.
    let err = call(&consumer, "router::route", json!({ "model": "ghost" }))
        .await
        .expect_err("ghost model cannot route");
    assert_eq!(remote_code(&err), "router/no_provider_for_model");

    // a composite `provider::model` id (the console's display form) previews
    // and executes on the embedded provider, with the split id on the wire —
    // dispatch agrees with the models surface about what the id means.
    let route = call(
        &consumer,
        "router::route",
        json!({ "model": "real::live-1" }),
    )
    .await
    .expect("composite id routes");
    assert_eq!(route["provider"], "real");
    let (writer_ref, _frames, pump) = consumer_channel(&consumer).await;
    let res = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({ "writer_ref": writer_ref, "model": "real::live-1", "messages": [] }),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await
        .expect("composite chat succeeds");
    assert_eq!(res["ok"], true, "chat response: {res}");
    assert_eq!(res["provider"], "real");
    assert_eq!(
        res["model"], "live-1",
        "the split id is what the provider served"
    );
    let _ = tokio::time::timeout(Duration::from_secs(5), pump).await;

    consumer.shutdown();
    router_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn consumer_cancellation_propagates_to_the_provider() {
    let engine = engine_or_skip!();

    let router_iii = register_worker(&engine.url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots");
    let provider = start_live_provider(
        &engine.url,
        ProviderOptions {
            ping_forever: true,
            ..Default::default()
        },
    )
    .await;

    let consumer = register_worker(&engine.url, InitOptions::default());
    let channel = iii_sdk::helpers::create_channel(&consumer, None)
        .await
        .expect("channel");
    let reader = channel.reader;
    let pump = tokio::spawn(async move {
        // start reading, then drop the reader after 300ms = consumer walks away
        let _ = tokio::time::timeout(Duration::from_millis(300), reader.read_all()).await;
        let _ = reader.close().await;
        drop(reader);
    });

    let chat = {
        let consumer = consumer.clone();
        let writer_ref = channel.writer_ref.clone();
        tokio::spawn(async move {
            consumer
                .trigger(TriggerRequest {
                    function_id: "router::chat".into(),
                    payload: json!({ "writer_ref": writer_ref, "model": "live-1", "messages": [] }),
                    action: None,
                    timeout_ms: Some(30_000),
                })
                .await
        })
    };
    let _ = pump.await;

    // the provider must observe the abort (its writes start failing) within 5s
    let deadline = Instant::now() + Duration::from_secs(5);
    while !provider.write_failed.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "provider writes still succeeding 5s after the consumer left"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    println!(
        "cancellation latency: provider write failed {}ms after stream start",
        provider.fail_at_ms.load(Ordering::SeqCst)
    );

    // router::chat resolves (does not hang)
    let res = tokio::time::timeout(Duration::from_secs(10), chat)
        .await
        .expect("chat resolved")
        .expect("join");
    println!("chat outcome after cancellation: {res:?}");

    consumer.shutdown();
    router_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn registry_survives_a_router_restart_and_token_stays_bound() {
    let engine = engine_or_skip!();

    let first = register_worker(&engine.url, InitOptions::default());
    register_router(first.clone()).await.expect("router boots");
    let provider = start_live_provider(&engine.url, ProviderOptions::default()).await;
    first.shutdown(); // "crash" the router connection

    let second = register_worker(&engine.url, InitOptions::default());
    register_router(second.clone())
        .await
        .expect("router reboots");

    // Availability is pessimistically reset on load. The router must recover
    // this still-live provider itself through its direct engine-function nudge;
    // relying only on the one-shot router::ready replay has a boot race.
    let list = call_until(&second, "router::provider::list", json!({}), |value| {
        value["providers"].as_array().is_some_and(|providers| {
            providers
                .iter()
                .any(|provider| provider["id"] == "real" && provider["available"] == true)
        })
    })
    .await;
    assert_eq!(list["providers"][0]["id"], "real", "list: {list}");

    // The automatic re-declare used the original bearer token; it remains
    // accepted for an explicit idempotent declaration too.
    let again = call(
        &provider.iii,
        "router::provider::register",
        json!({ "id": "real", "token": provider.token }),
    )
    .await
    .expect("re-declare accepted");
    assert_eq!(again["registration_token"], json!(provider.token.clone()));

    provider.iii.shutdown();
    second.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn system_prompt_get_resolves_declared_override_unset_and_default_provider() {
    let engine = engine_or_skip!();

    let router_iii = register_worker(&engine.url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots");

    // a provider that declares an identity prompt (registry only; no stream fn needed)
    let provider_iii = register_worker(&engine.url, InitOptions::default());
    call(
        &provider_iii,
        "router::provider::register",
        json!({ "id": "prompty", "system_prompt": "DECLARED IDENTITY" }),
    )
    .await
    .expect("provider declared");

    // declared prompt serves
    let res = call(
        &router_iii,
        "router::system_prompt::get",
        json!({ "provider": "prompty" }),
    )
    .await
    .unwrap();
    assert_eq!(res["provider"], "prompty");
    assert_eq!(res["system_prompt"], "DECLARED IDENTITY");

    // unknown provider → resolved id, null prompt (caller falls back)
    let res = call(
        &router_iii,
        "router::system_prompt::get",
        json!({ "provider": "nope" }),
    )
    .await
    .unwrap();
    assert_eq!(res["system_prompt"], Value::Null);

    // operator override wins; default_provider resolves an absent provider
    call(
        &router_iii,
        "configuration::set",
        json!({ "id": "llm-router", "value": {
            "default_provider": "prompty",
            "providers": { "prompty": { "system_prompt": "OPERATOR OVERRIDE" } }
        } }),
    )
    .await
    .unwrap();
    let res = call_until(
        &router_iii,
        "router::system_prompt::get",
        json!({}),
        |value| value["system_prompt"] == json!("OPERATOR OVERRIDE"),
    )
    .await;
    assert_eq!(res["provider"], "prompty");
    assert_eq!(res["system_prompt"], "OPERATOR OVERRIDE");

    // override unset (null, as the console's set/unset toggle writes) →
    // back to the provider-declared default
    call(
        &router_iii,
        "configuration::set",
        json!({ "id": "llm-router", "value": {
            "default_provider": "prompty",
            "providers": { "prompty": { "system_prompt": null } }
        } }),
    )
    .await
    .unwrap();
    let res = call_until(
        &router_iii,
        "router::system_prompt::get",
        json!({}),
        |value| value["system_prompt"] == json!("DECLARED IDENTITY"),
    )
    .await;
    assert_eq!(res["system_prompt"], "DECLARED IDENTITY");

    provider_iii.shutdown();
    router_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn registration_token_gates_takeover_resolve_and_reconcile() {
    let engine = engine_or_skip!();

    let router_iii = register_worker(&engine.url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots");
    let provider = start_live_provider(&engine.url, ProviderOptions::default()).await;

    // takeover without / with a wrong token is rejected
    let err = call(
        &provider.iii,
        "router::provider::register",
        json!({ "id": "real", "worker_id": "evil" }),
    )
    .await
    .unwrap_err();
    assert_eq!(remote_code(&err), "router/registration_rejected");

    let err = call(
        &provider.iii,
        "router::provider::resolve",
        json!({ "id": "real", "token": "wrong" }),
    )
    .await
    .unwrap_err();
    assert_eq!(remote_code(&err), "router/registration_rejected");

    let err = call(
        &provider.iii,
        "router::models::reconcile",
        json!({ "provider": "real", "token": "wrong", "models": [] }),
    )
    .await
    .unwrap_err();
    assert_eq!(remote_code(&err), "router/registration_rejected");

    let err = call(
        &provider.iii,
        "router::provider::resolve",
        json!({ "id": "missing", "token": "x" }),
    )
    .await
    .unwrap_err();
    assert_eq!(remote_code(&err), "router/unknown_provider");

    router_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn resolve_precedence_config_over_env_over_none() {
    let engine = engine_or_skip!();
    let env_var = "LLM_ROUTER_IT_RESOLVE_KEY";
    std::env::remove_var(env_var);

    let router_iii = register_worker(&engine.url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots");
    let provider = start_live_provider(
        &engine.url,
        ProviderOptions {
            credential_env_var: Some(env_var.into()),
            ..Default::default()
        },
    )
    .await;

    // none + declared defaults
    let res = call(
        &provider.iii,
        "router::provider::resolve",
        json!({ "id": "real", "token": provider.token }),
    )
    .await
    .unwrap();
    assert_eq!(res["configured"], false);
    assert_eq!(res["source"], "none");
    assert_eq!(res["api_url"], "https://api.example.test/v1");
    assert_eq!(res["max_tokens"], 8192);

    // env fallback (the env var is read in the ROUTER's process — same process here)
    std::env::set_var(env_var, "sk-env");
    let res = call(
        &provider.iii,
        "router::provider::resolve",
        json!({ "id": "real", "token": provider.token }),
    )
    .await
    .unwrap();
    assert_eq!(res["source"], "env");
    assert_eq!(res["credential"]["key"], "sk-env");

    // stored slice wins over env; slice max_tokens overrides the default
    call(
        &router_iii,
        "configuration::set",
        json!({ "id": "llm-router", "value": { "providers": { "real": { "api_key": "sk-stored", "max_tokens": 4096 } } } }),
    )
    .await
    .unwrap();
    let res = call_until(
        &provider.iii,
        "router::provider::resolve",
        json!({ "id": "real", "token": provider.token }),
        |value| value["source"] == json!("config"),
    )
    .await;
    assert_eq!(res["source"], "config");
    assert_eq!(res["credential"]["key"], "sk-stored");
    assert_eq!(res["max_tokens"], 4096);
    std::env::remove_var(env_var);

    router_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn paste_a_key_kicks_debounced_discovery_and_models_land() {
    let engine = engine_or_skip!();

    let router_iii = register_worker(&engine.url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots");
    let _provider = start_live_provider(
        &engine.url,
        ProviderOptions {
            supports_model_listing: true,
            discovered: vec!["disc-1".into()],
            ..Default::default()
        },
    )
    .await;

    call(
        &router_iii,
        "configuration::set",
        json!({ "id": "llm-router", "value": { "providers": { "real": { "api_key": "sk-pasted" } } } }),
    )
    .await
    .unwrap();

    // debounce is 2s; poll up to 10s for the discovered model to land
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let list = call(
            &router_iii,
            "router::models::list",
            json!({ "provider": "real" }),
        )
        .await
        .unwrap();
        let ids: Vec<&str> = list["models"]
            .as_array()
            .map(|a| a.iter().filter_map(|m| m["id"].as_str()).collect())
            .unwrap_or_default();
        if ids.contains(&"disc-1") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "discovered model never landed; have {ids:?}"
        );
        tokio::time::sleep(Duration::from_millis(250)).await;
    }

    // models surface: get + supports
    let got = call(
        &router_iii,
        "router::models::get",
        json!({ "provider": "real", "id": "disc-1" }),
    )
    .await
    .unwrap();
    assert_eq!(got["model"]["id"], "disc-1");
    let budget = call(
        &router_iii,
        "router::models::budget",
        json!({ "provider": "real", "id": "disc-1" }),
    )
    .await
    .unwrap();
    assert_eq!(budget["model"]["id"], "disc-1");
    assert_eq!(budget["effective_max_output_tokens"], 8192);
    let sup = call(
        &router_iii,
        "router::models::supports",
        json!({ "provider": "real", "id": "disc-1", "capability": "tools" }),
    )
    .await
    .unwrap();
    assert_eq!(sup["supported"], false); // flag absent on the discovered model

    // Composite `provider::model` ids (the console's display form) resolve
    // across the whole models surface, not just get: the same id must never
    // resolve in get/budget yet read as unsupported or list nothing.
    let got = call(
        &router_iii,
        "router::models::get",
        json!({ "id": "real::disc-1" }),
    )
    .await
    .unwrap();
    assert_eq!(got["model"]["id"], "disc-1");
    let budget = call(
        &router_iii,
        "router::models::budget",
        json!({ "provider": "real", "id": "real::disc-1" }),
    )
    .await
    .unwrap();
    assert_eq!(budget["model"]["id"], "disc-1");
    let sup = call(
        &router_iii,
        "router::models::supports",
        json!({ "provider": "real", "id": "real::disc-1", "capability": "vision" }),
    )
    .await
    .unwrap();
    assert_eq!(sup["supported"], true); // discovered flag — proves resolution
    let list = call(
        &router_iii,
        "router::models::list",
        json!({ "provider": "real::disc-1" }),
    )
    .await
    .unwrap();
    let listed: Vec<&str> = list["models"]
        .as_array()
        .map(|a| a.iter().filter_map(|m| m["id"].as_str()).collect())
        .unwrap_or_default();
    assert!(
        listed.contains(&"disc-1"),
        "composite provider filter lists the prefix's slice; have {listed:?}"
    );

    router_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn abort_stops_the_stream_and_terminates_with_done_aborted() {
    let engine = engine_or_skip!();

    let router_iii = register_worker(&engine.url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots");
    let provider = start_live_provider(
        &engine.url,
        ProviderOptions {
            ping_forever: true,
            ..Default::default()
        },
    )
    .await;

    let consumer = register_worker(&engine.url, InitOptions::default());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let chat = {
        let consumer = consumer.clone();
        tokio::spawn(async move {
            consumer
                .trigger(TriggerRequest {
                    function_id: "router::chat".into(),
                    payload: json!({ "writer_ref": writer_ref, "request_id": "abort-me", "model": "live-1", "messages": [] }),
                    action: None,
                    timeout_ms: Some(30_000),
                })
                .await
        })
    };
    // wait for the stream to start (first frame observed), then abort
    let deadline = Instant::now() + Duration::from_secs(5);
    while frames.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "stream never started");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let aborted = call(
        &consumer,
        "router::abort",
        json!({ "request_id": "abort-me" }),
    )
    .await
    .unwrap();
    assert_eq!(aborted, json!({ "aborted": true }));

    let res = tokio::time::timeout(Duration::from_secs(10), chat)
        .await
        .expect("chat resolved")
        .expect("join")
        .expect("chat ok");
    assert_eq!(res["stop_reason"], "aborted");

    // the provider observes the abort (its channel writes fail)
    let deadline = Instant::now() + Duration::from_secs(5);
    while !provider.write_failed.load(Ordering::SeqCst) {
        assert!(Instant::now() < deadline, "provider never saw the abort");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let _ = tokio::time::timeout(Duration::from_secs(5), pump).await;
    let frames = frames.lock().unwrap();
    let last: Value = serde_json::from_str(frames.last().unwrap()).unwrap();
    assert_eq!(last["type"], "done");
    assert_eq!(last["message"]["stop_reason"], "aborted");

    consumer.shutdown();
    router_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn duplicate_request_id_is_rejected_without_orphaning_the_original() {
    let engine = engine_or_skip!();
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone()).await.expect("router boots");
    let provider = start_live_provider(
        &engine.url,
        ProviderOptions {
            ping_forever: true,
            ..Default::default()
        },
    )
    .await;

    let first_consumer = register_worker(&engine.url, InitOptions::default());
    let (first_ref, first_frames, first_pump) = consumer_channel(&first_consumer).await;
    let first_chat = {
        let consumer = first_consumer.clone();
        tokio::spawn(async move {
            consumer
                .trigger(TriggerRequest {
                    function_id: "router::chat".into(),
                    payload: json!({
                        "writer_ref": first_ref,
                        "request_id": "same-request",
                        "model": "live-1",
                        "messages": []
                    }),
                    action: None,
                    timeout_ms: Some(10_000),
                })
                .await
        })
    };
    let deadline = Instant::now() + Duration::from_secs(3);
    while first_frames.lock().unwrap().is_empty() {
        assert!(Instant::now() < deadline, "first stream did not start");
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 1);

    let second_consumer = register_worker(&engine.url, InitOptions::default());
    let (second_ref, second_frames, second_pump) = consumer_channel(&second_consumer).await;
    let duplicate = second_consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": second_ref,
                "request_id": "same-request",
                "model": "live-1",
                "messages": []
            }),
            action: None,
            timeout_ms: Some(3_000),
        })
        .await
        .expect_err("duplicate live request id must be rejected");
    assert_eq!(
        remote_code(&duplicate),
        "router/invalid_request",
        "{duplicate:?}"
    );
    tokio::time::timeout(Duration::from_secs(2), second_pump)
        .await
        .expect("duplicate channel EOF")
        .expect("duplicate channel pump joins");
    let duplicate_frames: Vec<Value> = second_frames
        .lock()
        .unwrap()
        .iter()
        .map(|frame| serde_json::from_str(frame).expect("valid frame"))
        .collect();
    let duplicate_terminals = duplicate_frames
        .iter()
        .filter(|frame| matches!(frame["type"].as_str(), Some("done" | "error")))
        .count();
    assert_eq!(duplicate_terminals, 1, "frames: {duplicate_frames:?}");
    assert_eq!(duplicate_frames.last().unwrap()["type"], "error");
    assert_eq!(
        provider.stream_calls.load(Ordering::SeqCst),
        1,
        "duplicate request reached the provider"
    );
    assert!(!first_chat.is_finished(), "original stream was disturbed");

    let aborted = call(
        &router,
        "router::abort",
        json!({ "request_id": "same-request" }),
    )
    .await
    .expect("abort original");
    assert_eq!(aborted["aborted"], true, "{aborted}");
    let first = tokio::time::timeout(Duration::from_secs(3), first_chat)
        .await
        .expect("original chat resolves")
        .expect("original task joins")
        .expect("original chat response");
    assert_eq!(first["stop_reason"], "aborted", "{first}");
    tokio::time::timeout(Duration::from_secs(2), first_pump)
        .await
        .expect("original channel EOF")
        .expect("original channel pump joins");

    first_consumer.shutdown();
    second_consumer.shutdown();
    provider.iii.shutdown();
    router.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn update_credential_persists_and_resolves_back() {
    let engine = engine_or_skip!();

    let router_iii = register_worker(&engine.url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots");
    let provider = start_live_provider(&engine.url, ProviderOptions::default()).await;

    let res = call(
        &provider.iii,
        "router::provider::update_credential",
        json!({
            "id": "real", "token": provider.token,
            "credential": { "type": "oauth", "access_token": "at-1", "expires_at": 999 }
        }),
    )
    .await
    .unwrap();
    assert_eq!(res, json!({ "ok": true }));

    let resolved = call(
        &provider.iii,
        "router::provider::resolve",
        json!({ "id": "real", "token": provider.token }),
    )
    .await
    .unwrap();
    assert_eq!(resolved["source"], "config");
    assert_eq!(resolved["credential"]["access_token"], "at-1");

    router_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn models_changed_event_reaches_a_trigger_subscriber() {
    let engine = engine_or_skip!();

    let router_iii = register_worker(&engine.url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots");

    // Probe worker bound to the router-owned trigger type (README § Events):
    // the handler must receive the raw payload, no envelope.
    let probe = register_worker(&engine.url, InitOptions::default());
    let received = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let sink = received.clone();
    probe.register_function(
        "probe::on_models_changed",
        RegisterFunction::new_async(move |input: Value| {
            let sink = sink.clone();
            async move {
                sink.lock().unwrap().push(input);
                Ok::<Value, Error>(json!({}))
            }
        }),
    );
    probe
        .register_trigger(RegisterTriggerInput {
            trigger_type: "router::models::changed".into(),
            function_id: "probe::on_models_changed".into(),
            config: json!({}),
            metadata: None,
        })
        .expect("router::models::changed trigger registered");

    // Declare already emits count=1 from the static model; reconcile two
    // models so the explicit-reconcile emission is unambiguous.
    let provider = start_live_provider(&engine.url, ProviderOptions::default()).await;
    call(
        &provider.iii,
        "router::models::reconcile",
        json!({ "provider": "real", "token": provider.token, "models": [
            { "id": "m-1", "provider": "real", "context_window": 100000, "max_output_tokens": 8192 },
            { "id": "m-2", "provider": "real", "context_window": 100000, "max_output_tokens": 8192 }
        ]}),
    )
    .await
    .expect("reconcile accepted");

    let want_provider = "real";
    let want_count = 2;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let matched = received.lock().unwrap().iter().any(|p| {
            p.get("provider").and_then(Value::as_str) == Some(want_provider)
                && p.get("count").and_then(Value::as_u64) == Some(want_count)
        });
        if matched {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "router::models::changed never reached the trigger subscriber; got {:?}",
            received.lock().unwrap()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    probe.shutdown();
    router_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn router_boots_its_interface_against_a_bare_engine() {
    // Mirrors CI's interface-boot smoke: the registry-publish flow boots the
    // worker against an engine configured with `workers: []` — builtin
    // daemons only, no `iii-state`. Interface collection needs the router to
    // connect and register its functions; boot must tolerate the missing
    // state worker and come up with an empty registry/catalog.
    let engine = bare_engine_or_skip!();

    let router_iii = register_worker(&engine.url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots without iii-state");

    let list = call(&router_iii, "router::models::list", json!({}))
        .await
        .expect("interface answers");
    assert_eq!(list["models"], json!([]));

    let functions = call(
        &router_iii,
        "engine::functions::list",
        json!({ "include_internal": true }),
    )
    .await
    .expect("function catalog answers");
    let config_handler = functions["functions"]
        .as_array()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row["function_id"] == "router::on_config_changed")
        })
        .expect("reactive configuration handler is registered");
    assert_eq!(
        config_handler["metadata"]["trace_hidden"],
        json!(true),
        "reactive configuration plumbing must be hidden from traces by default"
    );

    router_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_registry_persist_does_not_poison_provider_retries() {
    // Bundle workers start concurrently. A provider may declare itself after
    // the router is ready but before the state worker exposes `state::set`.
    // Both attempts below must fail for that transient dependency only; the
    // first must not leave a token hash that turns the second into a takeover.
    let engine = bare_engine_or_skip!();
    let iii = register_worker(&engine.url, InitOptions::default());
    let registry = RegistryStore::new(iii.clone());
    let declaration: ProviderDeclaration =
        serde_json::from_value(json!({ "id": "late-state" })).unwrap();

    for attempt in 1..=2 {
        let error = match registry
            .upsert(
                declaration.clone(),
                Some("provider-late-state".into()),
                None,
            )
            .await
        {
            Err(error) => error,
            Ok(_) => panic!("state persistence is unavailable on the bare engine"),
        };
        assert_eq!(
            error.code,
            llm_router::types::errors::RouterCode::InvalidRequest,
            "attempt {attempt} must remain retryable, got: {error}"
        );
        assert!(
            error.message.contains("registry persist failed"),
            "attempt {attempt} failed for an unexpected reason: {error}"
        );
        assert!(
            registry.ids().await.is_empty(),
            "failed persistence must not publish a provider binding"
        );
    }

    iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_registry_write_rolls_back_and_retry_can_bind_without_a_token() {
    let engine = bare_engine_or_skip!();
    let (state, set_calls) = start_flaky_state(&engine.url, 1).await;
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone()).await.expect("router boots");

    let first = call(
        &router,
        "router::provider::register",
        json!({ "id": "recoverable" }),
    )
    .await
    .expect_err("the injected first state write must fail");
    assert_eq!(remote_code(&first), "router/invalid_request", "{first:?}");

    let after_failure = call(&router, "router::provider::list", json!({}))
        .await
        .expect("provider list");
    assert_eq!(
        after_failure["providers"],
        json!([]),
        "failed persistence must not publish a ghost provider: {after_failure}"
    );

    let retry = call(
        &router,
        "router::provider::register",
        json!({ "id": "recoverable" }),
    )
    .await
    .expect("retry without an undelivered token must bind cleanly");
    assert_eq!(retry["ok"], true, "{retry}");
    assert!(
        retry["registration_token"]
            .as_str()
            .is_some_and(|token| !token.is_empty()),
        "registration token missing: {retry}"
    );
    assert_eq!(set_calls.load(Ordering::SeqCst), 2);

    router.shutdown();
    state.shutdown();
}

async fn assert_static_registration_failure_is_atomic(
    fail_on_set_call: u64,
    expected_set_calls_after_retry: u64,
) {
    let engine = bare_engine_or_skip!();
    let (state, set_calls) = start_flaky_state(&engine.url, fail_on_set_call).await;
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone()).await.expect("router boots");

    let provider_id = format!("atomic-static-{fail_on_set_call}");
    let model_id = format!("atomic-model-{fail_on_set_call}");
    let declaration = json!({
        "id": provider_id,
        "models": [{
            "id": model_id,
            "provider": provider_id,
            "context_window": 1_000,
            "max_output_tokens": 100
        }]
    });
    call(&router, "router::provider::register", declaration.clone())
        .await
        .expect_err("the selected state write must fail registration");

    let providers = call(&router, "router::provider::list", json!({}))
        .await
        .expect("provider list after failure");
    assert!(
        providers["providers"]
            .as_array()
            .is_some_and(|rows| rows.iter().all(|row| row["id"] != provider_id)),
        "failed registration published a provider ghost: {providers}"
    );
    let models = call(
        &router,
        "router::models::list",
        json!({ "provider": provider_id }),
    )
    .await
    .expect("model list after failure");
    assert_eq!(
        models["models"],
        json!([]),
        "failed registration published a catalog ghost: {models}"
    );

    // Rebuild both stores from the fake state's successful writes. In the
    // fail-on-2 case this specifically proves the durable catalog candidate
    // was compensated after the registry commit failed.
    router.shutdown();
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone())
        .await
        .expect("router reboots");
    let providers = call(&router, "router::provider::list", json!({}))
        .await
        .expect("provider list after restart");
    assert!(
        providers["providers"]
            .as_array()
            .is_some_and(|rows| rows.iter().all(|row| row["id"] != provider_id)),
        "failed registration survived restart as a provider ghost: {providers}"
    );
    let models = call(
        &router,
        "router::models::list",
        json!({ "provider": provider_id }),
    )
    .await
    .expect("model list after restart");
    assert_eq!(
        models["models"],
        json!([]),
        "failed registration survived restart as a catalog ghost: {models}"
    );

    let retry = call(&router, "router::provider::register", declaration)
        .await
        .expect("retry without an undelivered token must bind cleanly");
    assert_eq!(retry["ok"], true, "{retry}");
    assert!(
        retry["registration_token"]
            .as_str()
            .is_some_and(|token| !token.is_empty()),
        "registration token missing after retry: {retry}"
    );
    let models = call(
        &router,
        "router::models::list",
        json!({ "provider": provider_id }),
    )
    .await
    .expect("model list after retry");
    assert_eq!(models["models"][0]["id"], model_id, "{models}");
    assert_eq!(
        set_calls.load(Ordering::SeqCst),
        expected_set_calls_after_retry
    );

    router.shutdown();
    state.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_static_catalog_write_keeps_provider_and_models_invisible() {
    // catalog write fails before the registry is attempted; retry performs
    // one catalog and one registry write.
    assert_static_registration_failure_is_atomic(1, 3).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn failed_registry_after_static_catalog_rolls_back_before_retry() {
    // catalog persists, registry fails, catalog rollback persists the prior
    // snapshot, then retry performs catalog + registry writes.
    assert_static_registration_failure_is_atomic(2, 5).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn late_failure_from_an_old_registration_cannot_mark_the_new_generation_down() {
    let engine = engine_or_skip!();
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone()).await.expect("router boots");

    let entered = Arc::new(tokio::sync::Semaphore::new(0));
    let release = Arc::new(tokio::sync::Semaphore::new(0));
    let provider = register_worker(&engine.url, InitOptions::default());
    let handler_entered = entered.clone();
    let handler_release = release.clone();
    provider.register_function(
        "provider::generation-race::stream",
        RegisterFunction::new_async(move |_input: Value| {
            let entered = handler_entered.clone();
            let release = handler_release.clone();
            async move {
                entered.add_permits(1);
                release
                    .acquire()
                    .await
                    .expect("release semaphore stays open")
                    .forget();
                Err::<Value, Error>(Error::Remote {
                    code: "function_not_found".into(),
                    message: "delayed failure from the old registration".into(),
                    stacktrace: None,
                })
            }
        }),
    );

    let declaration = json!({
        "id": "generation-race",
        "models": [{
            "id": "generation-race-model",
            "provider": "generation-race",
            "context_window": 1_000,
            "max_output_tokens": 100
        }]
    });
    let first = call(&provider, "router::provider::register", declaration.clone())
        .await
        .expect("first registration");
    let token = first["registration_token"]
        .as_str()
        .expect("registration token")
        .to_string();

    let consumer = register_worker(&engine.url, InitOptions::default());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let chat = {
        let consumer = consumer.clone();
        tokio::spawn(async move {
            consumer
                .trigger(TriggerRequest {
                    function_id: "router::chat".into(),
                    payload: json!({
                        "writer_ref": writer_ref,
                        "model": "generation-race-model",
                        "messages": []
                    }),
                    action: None,
                    timeout_ms: Some(10_000),
                })
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(3), entered.acquire())
        .await
        .expect("old provider handler was not entered")
        .expect("entered semaphore stays open")
        .forget();

    let mut redeclaration = declaration;
    redeclaration["token"] = json!(token);
    call(&provider, "router::provider::register", redeclaration)
        .await
        .expect("new generation registers before the old failure lands");
    release.add_permits(1);

    let error = tokio::time::timeout(Duration::from_secs(3), chat)
        .await
        .expect("chat resolves after releasing old handler")
        .expect("chat task joins")
        .expect_err("old dispatch still failed");
    assert_eq!(
        remote_code(&error),
        "router/provider_unavailable",
        "{error:?}"
    );
    tokio::time::timeout(Duration::from_secs(2), pump)
        .await
        .expect("consumer channel EOF")
        .expect("consumer pump joins");
    let terminal_count = frames
        .lock()
        .unwrap()
        .iter()
        .filter_map(|frame| serde_json::from_str::<Value>(frame).ok())
        .filter(|frame| matches!(frame["type"].as_str(), Some("done" | "error")))
        .count();
    assert_eq!(terminal_count, 1, "frames: {:?}", frames.lock().unwrap());

    let providers = call(&router, "router::provider::list", json!({}))
        .await
        .expect("provider list");
    let current = providers["providers"]
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["id"] == "generation-race"))
        .expect("generation-race provider remains registered");
    assert_eq!(
        current["available"], true,
        "late old-generation failure marked the new registration down: {providers}"
    );
    let route = call(
        &router,
        "router::route",
        json!({ "model": "generation-race-model" }),
    )
    .await
    .expect("new registration remains routable");
    assert_eq!(route["provider"], "generation-race", "{route}");

    consumer.shutdown();
    provider.shutdown();
    router.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn provider_bad_request_is_permanent_and_is_not_retried() {
    let engine = engine_or_skip!();
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone()).await.expect("router boots");

    let provider = register_worker(&engine.url, InitOptions::default());
    let handler_calls = Arc::new(AtomicU64::new(0));
    let bad_requests = Arc::new(AtomicU64::new(0));
    let observed_handler_calls = handler_calls.clone();
    let observed_bad_requests = bad_requests.clone();
    provider.register_function(
        "provider::typed::stream",
        RegisterFunction::new_async_with_bad_request(
            move |_input: llm_router::types::router::ProviderStreamInput| {
                let handler_calls = observed_handler_calls.clone();
                async move {
                    handler_calls.fetch_add(1, Ordering::SeqCst);
                    Ok::<llm_router::types::router::ProviderStreamOutput, Error>(
                        llm_router::types::router::ProviderStreamOutput { ok: true },
                    )
                }
            },
            move |error| {
                observed_bad_requests.fetch_add(1, Ordering::SeqCst);
                Error::Remote {
                    code: "provider/invalid_request".into(),
                    message: format!("bad ProviderStreamInput: {error}"),
                    stacktrace: None,
                }
            },
        ),
    );
    call(
        &provider,
        "router::provider::register",
        json!({
            "id": "typed",
            "models": [{
                "id": "typed-model",
                "provider": "typed",
                "context_window": 1_000,
                "max_output_tokens": 100
            }]
        }),
    )
    .await
    .expect("typed provider registers");

    let consumer = register_worker(&engine.url, InitOptions::default());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let error = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "request_id": "malformed-provider-input",
                "model": "typed-model",
                "messages": [{
                    "role": "user",
                    "content": "content must be an array",
                    "timestamp": 1
                }]
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect_err("provider input rejection remains a typed bus error");
    assert_eq!(remote_code(&error), "provider/invalid_request", "{error:?}");
    tokio::time::timeout(Duration::from_secs(2), pump)
        .await
        .expect("caller channel reaches EOF")
        .expect("channel pump joins");

    assert_eq!(handler_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        bad_requests.load(Ordering::SeqCst),
        1,
        "a permanent bad request must not be retried"
    );
    let parsed: Vec<Value> = frames
        .lock()
        .unwrap()
        .iter()
        .map(|frame| serde_json::from_str(frame).expect("valid frame"))
        .collect();
    let terminals: Vec<&Value> = parsed
        .iter()
        .filter(|frame| matches!(frame["type"].as_str(), Some("done" | "error")))
        .collect();
    assert_eq!(terminals.len(), 1, "frames: {parsed:?}");
    assert_eq!(terminals[0]["type"], "error", "frames: {parsed:?}");
    assert_eq!(
        terminals[0]["error"]["error_kind"], "permanent",
        "frames: {parsed:?}"
    );

    consumer.shutdown();
    provider.shutdown();
    router.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn transient_pre_stream_error_retries_invisibly_with_one_resolution_key() {
    let engine = engine_or_skip!();
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone()).await.expect("router boots");

    let provider = register_worker(&engine.url, InitOptions::default());
    let calls = Arc::new(AtomicU64::new(0));
    let resolution_keys = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let observed_calls = calls.clone();
    let observed_keys = resolution_keys.clone();
    let address = engine.url.clone();
    provider.register_function(
        "provider::retryable::stream",
        RegisterFunction::new_async(move |input: Value| {
            let address = address.clone();
            let calls = observed_calls.clone();
            let resolution_keys = observed_keys.clone();
            async move {
                let attempt = calls.fetch_add(1, Ordering::SeqCst) + 1;
                resolution_keys.lock().unwrap().push(
                    input["resolution_key"]
                        .as_str()
                        .expect("resolution key")
                        .to_string(),
                );
                let writer_ref: StreamChannelRef =
                    serde_json::from_value(input["writer_ref"].clone())
                        .map_err(|error| Error::Serde(error.to_string()))?;
                let writer = iii_sdk::channel::ChannelWriter::new(&address, &writer_ref);
                let model = input["model"].clone();
                if attempt == 1 {
                    writer
                        .send_message(
                            &json!({
                                "type": "error",
                                "error": {
                                    "role": "assistant",
                                    "content": [],
                                    "stop_reason": "error",
                                    "model": model,
                                    "provider": "retryable",
                                    "timestamp": 1,
                                    "error_message": "retry once",
                                    "error_kind": "rate_limited"
                                }
                            })
                            .to_string(),
                        )
                        .await
                        .map_err(|error| Error::Handler(error.to_string()))?;
                    let _ = writer.close().await;
                    return Ok::<Value, Error>(json!({ "ok": true }));
                }

                let message = json!({
                    "role": "assistant",
                    "content": [{ "type": "text", "text": "recovered" }],
                    "stop_reason": "end",
                    "model": model,
                    "provider": "retryable",
                    "timestamp": 2
                });
                for frame in [
                    json!({
                        "type": "start",
                        "partial": {
                            "role": "assistant",
                            "content": [],
                            "stop_reason": "end",
                            "model": model,
                            "provider": "retryable",
                            "timestamp": 2
                        }
                    }),
                    json!({ "type": "done", "message": message }),
                ] {
                    writer
                        .send_message(&frame.to_string())
                        .await
                        .map_err(|error| Error::Handler(error.to_string()))?;
                }
                let _ = writer.close().await;
                Ok::<Value, Error>(json!({ "ok": true }))
            }
        }),
    );
    call(
        &provider,
        "router::provider::register",
        json!({
            "id": "retryable",
            "models": [{
                "id": "retry-model",
                "provider": "retryable",
                "context_window": 1_000,
                "max_output_tokens": 100
            }]
        }),
    )
    .await
    .expect("retryable provider registers");

    let consumer = register_worker(&engine.url, InitOptions::default());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let response = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "request_id": "stable-retry-id",
                "model": "retry-model",
                "messages": []
            }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .expect("second attempt succeeds");
    assert_eq!(response["ok"], true, "{response}");
    tokio::time::timeout(Duration::from_secs(2), pump)
        .await
        .expect("caller channel reaches EOF")
        .expect("channel pump joins");

    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        *resolution_keys.lock().unwrap(),
        vec!["stable-retry-id", "stable-retry-id"]
    );
    let parsed: Vec<Value> = frames
        .lock()
        .unwrap()
        .iter()
        .map(|frame| serde_json::from_str(frame).expect("valid frame"))
        .collect();
    assert!(
        parsed.iter().all(|frame| frame["type"] != "error"),
        "the first attempt leaked to the caller: {parsed:?}"
    );
    let terminals = parsed
        .iter()
        .filter(|frame| matches!(frame["type"].as_str(), Some("done" | "error")))
        .count();
    assert_eq!(terminals, 1, "frames: {parsed:?}");
    assert_eq!(parsed.last().unwrap()["type"], "done", "frames: {parsed:?}");

    consumer.shutdown();
    provider.shutdown();
    router.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn abort_during_retry_backoff_finishes_without_a_second_dispatch() {
    let engine = engine_or_skip!();
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone()).await.expect("router boots");

    let provider = register_worker(&engine.url, InitOptions::default());
    let calls = Arc::new(AtomicU64::new(0));
    let error_sent = Arc::new(tokio::sync::Semaphore::new(0));
    let observed_calls = calls.clone();
    let observed_error = error_sent.clone();
    let address = engine.url.clone();
    provider.register_function(
        "provider::backoff::stream",
        RegisterFunction::new_async(move |input: Value| {
            let address = address.clone();
            let calls = observed_calls.clone();
            let error_sent = observed_error.clone();
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let writer_ref: StreamChannelRef =
                    serde_json::from_value(input["writer_ref"].clone())
                        .map_err(|error| Error::Serde(error.to_string()))?;
                let writer = iii_sdk::channel::ChannelWriter::new(&address, &writer_ref);
                writer
                    .send_message(
                        &json!({
                            "type": "error",
                            "error": {
                                "role": "assistant",
                                "content": [],
                                "stop_reason": "error",
                                "model": input["model"],
                                "provider": "backoff",
                                "timestamp": 1,
                                "error_message": "retry later",
                                "error_kind": "transient"
                            }
                        })
                        .to_string(),
                    )
                    .await
                    .map_err(|error| Error::Handler(error.to_string()))?;
                let _ = writer.close().await;
                error_sent.add_permits(1);
                Ok::<Value, Error>(json!({ "ok": true }))
            }
        }),
    );
    call(
        &provider,
        "router::provider::register",
        json!({
            "id": "backoff",
            "models": [{
                "id": "backoff-model",
                "provider": "backoff",
                "context_window": 1_000,
                "max_output_tokens": 100
            }]
        }),
    )
    .await
    .expect("backoff provider registers");

    let consumer = register_worker(&engine.url, InitOptions::default());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let chat = {
        let consumer = consumer.clone();
        tokio::spawn(async move {
            consumer
                .trigger(TriggerRequest {
                    function_id: "router::chat".into(),
                    payload: json!({
                        "writer_ref": writer_ref,
                        "request_id": "abort-in-backoff",
                        "model": "backoff-model",
                        "messages": []
                    }),
                    action: None,
                    timeout_ms: Some(5_000),
                })
                .await
        })
    };
    tokio::time::timeout(Duration::from_secs(3), error_sent.acquire())
        .await
        .expect("provider did not emit its retryable error")
        .expect("error semaphore stays open")
        .forget();
    // Give the relay ample time to classify the frame and enter the first
    // backoff, whose documented minimum is 500ms.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let started = Instant::now();
    let aborted = call(
        &router,
        "router::abort",
        json!({ "request_id": "abort-in-backoff" }),
    )
    .await
    .expect("abort request succeeds");
    assert_eq!(aborted["aborted"], true, "{aborted}");
    let response = tokio::time::timeout(Duration::from_millis(350), chat)
        .await
        .unwrap_or_else(|_| panic!("abort waited for retry backoff: {:?}", started.elapsed()))
        .expect("chat task joins")
        .expect("chat returns an aborted response");
    assert_eq!(response["stop_reason"], "aborted", "{response}");
    tokio::time::timeout(Duration::from_secs(2), pump)
        .await
        .expect("caller channel reaches EOF")
        .expect("channel pump joins");

    assert_eq!(calls.load(Ordering::SeqCst), 1, "abort started a retry");
    let parsed: Vec<Value> = frames
        .lock()
        .unwrap()
        .iter()
        .map(|frame| serde_json::from_str(frame).expect("valid frame"))
        .collect();
    let terminals: Vec<&Value> = parsed
        .iter()
        .filter(|frame| matches!(frame["type"].as_str(), Some("done" | "error")))
        .collect();
    assert_eq!(terminals.len(), 1, "frames: {parsed:?}");
    assert_eq!(terminals[0]["type"], "done", "frames: {parsed:?}");
    assert_eq!(
        terminals[0]["message"]["stop_reason"], "aborted",
        "frames: {parsed:?}"
    );

    consumer.shutdown();
    provider.shutdown();
    router.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn extreme_retry_configuration_is_rejected_without_disrupting_chat() {
    let engine = engine_or_skip!();
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone()).await.expect("router boots");
    let provider = start_live_provider(&engine.url, ProviderOptions::default()).await;

    call(
        &router,
        "configuration::set",
        json!({
            "id": "llm-router",
            "value": { "settings": { "retry_max": u32::MAX } }
        }),
    )
    .await
    .expect_err("retry counts above the operational bound must be rejected");

    let consumer = register_worker(&engine.url, InitOptions::default());
    let completed = call(
        &consumer,
        "router::complete",
        json!({ "model": "live-1", "messages": [] }),
    )
    .await
    .expect("rejected configuration must leave chat healthy");
    assert_eq!(completed["message"]["content"][0]["text"], "live");
    assert_eq!(provider.stream_calls.load(Ordering::SeqCst), 1);

    consumer.shutdown();
    provider.iii.shutdown();
    router.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn router_coded_provider_failure_still_emits_one_terminal_and_eof() {
    let engine = engine_or_skip!();
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone()).await.expect("router boots");

    let provider = register_worker(&engine.url, InitOptions::default());
    provider.register_function(
        "provider::coded-error::stream",
        RegisterFunction::new_async(|_input: Value| async move {
            Err::<Value, Error>(Error::Remote {
                code: "router/not_configured".into(),
                message: "provider credentials are not configured".into(),
                stacktrace: None,
            })
        }),
    );
    call(
        &provider,
        "router::provider::register",
        json!({
            "id": "coded-error",
            "models": [{
                "id": "coded-error-model",
                "provider": "coded-error",
                "context_window": 1_000,
                "max_output_tokens": 100
            }]
        }),
    )
    .await
    .expect("provider registers");

    let consumer = register_worker(&engine.url, InitOptions::default());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let error = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "model": "coded-error-model",
                "messages": []
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect_err("router-coded provider failure remains a typed bus error");
    assert_eq!(remote_code(&error), "router/not_configured", "{error:?}");
    tokio::time::timeout(Duration::from_secs(2), pump)
        .await
        .expect("caller channel reaches EOF")
        .expect("channel pump joins");

    let parsed: Vec<Value> = frames
        .lock()
        .unwrap()
        .iter()
        .map(|frame| serde_json::from_str(frame).expect("valid frame"))
        .collect();
    let terminals: Vec<&Value> = parsed
        .iter()
        .filter(|frame| matches!(frame["type"].as_str(), Some("done" | "error")))
        .collect();
    assert_eq!(terminals.len(), 1, "frames: {parsed:?}");
    assert_eq!(terminals[0]["type"], "error", "frames: {parsed:?}");
    assert_eq!(
        terminals[0]["error"]["error_kind"], "permanent",
        "frames: {parsed:?}"
    );

    consumer.shutdown();
    provider.shutdown();
    router.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn internal_channel_creation_failure_still_emits_one_terminal_and_eof() {
    let engine = engine_or_skip!();
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone()).await.expect("router boots");
    let provider = start_live_provider(&engine.url, ProviderOptions::default()).await;

    // Mint the consumer channel before adversarially shadowing the engine's
    // channel factory. The chat pipeline must turn the later internal factory
    // failure into a terminal frame on this already-valid caller channel.
    let consumer = register_worker(&engine.url, InitOptions::default());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let saboteur = register_worker(&engine.url, InitOptions::default());
    saboteur.register_function(
        "engine::channels::create",
        RegisterFunction::new_async(|_input: Value| async move {
            Err::<Value, Error>(Error::Remote {
                code: "injected/channel_unavailable".into(),
                message: "injected channel factory failure".into(),
                stacktrace: None,
            })
        }),
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match call(&saboteur, "engine::channels::create", json!({})).await {
            Err(error) if remote_code(&error) == "injected/channel_unavailable" => break,
            _ => {
                assert!(
                    Instant::now() < deadline,
                    "adversarial channel factory did not become active"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        }
    }

    let error = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "model": "live-1",
                "messages": []
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect_err("internal channel creation failure remains a bus error");
    assert_eq!(
        remote_code(&error),
        "injected/channel_unavailable",
        "{error:?}"
    );
    tokio::time::timeout(Duration::from_secs(2), pump)
        .await
        .expect("caller channel reaches EOF")
        .expect("channel pump joins");

    let parsed: Vec<Value> = frames
        .lock()
        .unwrap()
        .iter()
        .map(|frame| serde_json::from_str(frame).expect("valid frame"))
        .collect();
    let terminals: Vec<&Value> = parsed
        .iter()
        .filter(|frame| matches!(frame["type"].as_str(), Some("done" | "error")))
        .collect();
    assert_eq!(terminals.len(), 1, "frames: {parsed:?}");
    assert_eq!(terminals[0]["type"], "error", "frames: {parsed:?}");
    assert_eq!(
        terminals[0]["error"]["error_kind"], "transient",
        "frames: {parsed:?}"
    );

    saboteur.shutdown();
    consumer.shutdown();
    provider.iii.shutdown();
    router.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn chat_provider_disappears_before_dispatch_emits_one_terminal_and_eof() {
    let engine = engine_or_skip!();
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone()).await.expect("router boots");
    call(
        &router,
        "router::provider::register",
        json!({
            "id": "ghost",
            "models": [{
                "id": "ghost-model",
                "provider": "ghost",
                "context_window": 1000,
                "max_output_tokens": 100
            }]
        }),
    )
    .await
    .expect("declaration accepted");

    let consumer = register_worker(&engine.url, InitOptions::default());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let err = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({
                "writer_ref": writer_ref,
                "model": "ghost-model",
                "messages": []
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect_err("missing provider function must be a typed error");
    assert_eq!(remote_code(&err), "router/provider_unavailable", "{err:?}");
    tokio::time::timeout(Duration::from_secs(2), pump)
        .await
        .expect("caller channel must reach EOF")
        .expect("channel pump joins");

    let parsed: Vec<Value> = frames
        .lock()
        .unwrap()
        .iter()
        .map(|frame| serde_json::from_str(frame).expect("valid frame"))
        .collect();
    let terminals: Vec<&Value> = parsed
        .iter()
        .filter(|frame| matches!(frame["type"].as_str(), Some("done" | "error")))
        .collect();
    assert_eq!(terminals.len(), 1, "frames: {parsed:?}");
    assert_eq!(terminals[0]["type"], "error", "frames: {parsed:?}");
    assert_eq!(
        terminals[0]["error"]["error_kind"], "transient",
        "frames: {parsed:?}"
    );

    let listed = call(&router, "router::provider::list", json!({}))
        .await
        .expect("provider list");
    assert_eq!(listed["providers"][0]["available"], false, "{listed}");
    let route = call(&router, "router::route", json!({ "model": "ghost-model" }))
        .await
        .expect_err("an unavailable catalog owner must not remain routable");
    assert_eq!(
        remote_code(&route),
        "router/provider_unavailable",
        "unexpected routing error: {route:?}"
    );

    consumer.shutdown();
    router.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn terminal_frame_completes_even_when_the_provider_handler_lingers() {
    let engine = engine_or_skip!();
    let router = register_worker(&engine.url, InitOptions::default());
    register_router(router.clone()).await.expect("router boots");
    let provider = start_live_provider(
        &engine.url,
        ProviderOptions {
            done_linger_ms: Some(5_000),
            ..Default::default()
        },
    )
    .await;
    call(
        &router,
        "configuration::set",
        json!({
            "id": "llm-router",
            "value": {
                "settings": {
                    "stream_timeout_ms": 1_000,
                    "idle_timeout_ms": 500,
                    "retry_max": 0
                }
            }
        }),
    )
    .await
    .expect("settings update");
    tokio::time::sleep(Duration::from_millis(250)).await;

    let consumer = register_worker(&engine.url, InitOptions::default());
    let (writer_ref, frames, pump) = consumer_channel(&consumer).await;
    let started = Instant::now();
    let response = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({ "writer_ref": writer_ref, "model": "live-1", "messages": [] }),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await
        .expect("done frame completes chat");
    let elapsed = started.elapsed();
    assert_eq!(response["ok"], true, "{response}");
    assert!(
        elapsed < Duration::from_millis(500),
        "chat waited for the provider RPC after done: {elapsed:?}"
    );
    tokio::time::timeout(Duration::from_secs(2), pump)
        .await
        .expect("caller channel EOF")
        .expect("channel pump joins");
    let terminal_count = frames
        .lock()
        .unwrap()
        .iter()
        .filter_map(|frame| serde_json::from_str::<Value>(frame).ok())
        .filter(|frame| matches!(frame["type"].as_str(), Some("done" | "error")))
        .count();
    assert_eq!(terminal_count, 1, "frames: {:?}", frames.lock().unwrap());

    consumer.shutdown();
    provider.iii.shutdown();
    router.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn complete_fails_fast_when_the_provider_worker_is_gone() {
    // Regression for the completion boundary: a registered provider whose
    // worker is gone emits a terminal error and returns a typed Err. The
    // internal channel drain must still race the pipeline and answer promptly.
    let engine = engine_or_skip!();
    let router_iii = register_worker(&engine.url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots");

    // Declaration only — no live worker serves provider::ghost::stream.
    call(
        &router_iii,
        "router::provider::register",
        json!({ "id": "ghost" }),
    )
    .await
    .expect("declaration accepted");

    let consumer = register_worker(&engine.url, InitOptions::default());
    let started = Instant::now();
    let err = call(
        &consumer,
        "router::complete",
        json!({
            "provider": "ghost",
            "model": "ghost-model",
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 1 }],
        }),
    )
    .await
    .expect_err("typed error, not a hang");
    assert_eq!(
        remote_code(&err),
        "router/provider_unavailable",
        "got {err:?} after {:?}",
        started.elapsed()
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "must answer well before any drain budget, took {:?}",
        started.elapsed()
    );

    consumer.shutdown();
    router_iii.shutdown();
}

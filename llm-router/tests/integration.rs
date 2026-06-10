//! Engine-backed integration suite — every bus-shaped flow runs against a
//! real iii engine (binary-worker.md § 9): registration + token gate, resolve
//! precedence, paste-a-key, chat relay, cancellation, abort, restart.
//!
//! **Self-skips** when no engine is available (storage-worker pattern):
//! set `III_ENGINE_BIN=/path/to/iii` or have `iii` on PATH.
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::{register_worker, InitOptions, RegisterFunction, TriggerRequest, III};
use llm_router::register::register_router;
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
    let dir = std::env::temp_dir().join(format!("llm-router-it-{}", uuid::Uuid::new_v4()));
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

async fn call(iii: &III, function_id: &str, payload: Value) -> Result<Value, iii_sdk::IIIError> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
}

fn remote_code(err: &iii_sdk::IIIError) -> &str {
    match err {
        iii_sdk::IIIError::Remote { code, .. } => code,
        _ => "",
    }
}

// ── live provider helper ────────────────────────────────────────────────────

#[derive(Default)]
struct ProviderOptions {
    ping_forever: bool,
    credential_env_var: Option<String>,
    supports_model_listing: bool,
    /// model ids returned by provider::real::refresh_models
    discovered: Vec<String>,
}

struct LiveProvider {
    iii: III,
    token: String,
    write_failed: Arc<AtomicBool>,
    fail_at_ms: Arc<AtomicU64>,
}

/// A live provider worker on its own connection: registers
/// provider::real::stream (+ refresh_models) and declares itself.
async fn start_live_provider(url: &str, opts: ProviderOptions) -> LiveProvider {
    let iii = register_worker(url, InitOptions::default());
    let write_failed = Arc::new(AtomicBool::new(false));
    let fail_at_ms = Arc::new(AtomicU64::new(0));
    let token_cell: Arc<std::sync::Mutex<Option<String>>> = Arc::default();

    let address = url.to_string();
    let wf = write_failed.clone();
    let fam = fail_at_ms.clone();
    let ping_forever = opts.ping_forever;
    iii.register_function(
        "provider::real::stream",
        RegisterFunction::new_async(move |input: Value| {
            let address = address.clone();
            let (wf, fam) = (wf.clone(), fam.clone());
            async move {
                let r: iii_sdk::StreamChannelRef =
                    serde_json::from_value(input["writer_ref"].clone())
                        .map_err(|e| iii_sdk::IIIError::Serde(e.to_string()))?;
                let writer = iii_sdk::ChannelWriter::new(&address, &r);
                let model = input["model"].clone();
                let start = json!({ "type": "start", "partial": { "role": "assistant", "content": [], "stop_reason": "end", "model": model, "provider": "real", "timestamp": 1 } });
                writer
                    .send_message(&start.to_string())
                    .await
                    .map_err(|e| iii_sdk::IIIError::Handler(e.to_string()))?;
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
                let message = json!({
                    "role": "assistant", "content": [{ "type": "text", "text": "live" }],
                    "stop_reason": "end", "model": model, "provider": "real", "timestamp": 2
                });
                writer
                    .send_message(&json!({ "type": "done", "message": message }).to_string())
                    .await
                    .map_err(|e| iii_sdk::IIIError::Handler(e.to_string()))?;
                let _ = writer.close().await;
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
                    .map(|id| json!({ "id": id, "provider": "real", "context_window": 100000, "max_output_tokens": 8192 }))
                    .collect();
                async move {
                    iii.trigger(TriggerRequest {
                        function_id: "router::models::reconcile".into(),
                        payload: json!({ "provider": "real", "token": token, "models": models }),
                        action: None,
                        timeout_ms: Some(5000),
                    })
                    .await?;
                    Ok::<Value, iii_sdk::IIIError>(json!({ "ok": true }))
                }
            }),
        );
    }

    // declare (with a short retry in case the router is still booting)
    let mut token = None;
    for _ in 0..50 {
        let res = call(
            &iii,
            "router::provider::register",
            json!({
                "id": "real",
                "credential_env_var": opts.credential_env_var,
                "defaults": { "api_url": "https://api.example.test/v1", "max_tokens": 8192 },
                "supports_model_listing": opts.supports_model_listing,
                "models": [{ "id": "live-1", "provider": "real", "context_window": 100000, "max_output_tokens": 8192 }]
            }),
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
        write_failed,
        fail_at_ms,
    }
}

/// Consumer-side channel: collect frames + a pump that drives dispatch.
async fn consumer_channel(
    iii: &III,
) -> (
    iii_sdk::StreamChannelRef,
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

    let list = call(&second, "router::provider::list", json!({}))
        .await
        .expect("provider list");
    assert_eq!(list["providers"][0]["id"], "real", "list: {list}");

    // re-declare with the original token: idempotent, same token accepted
    let again = call(
        &provider.iii,
        "router::provider::register",
        json!({ "id": "real", "token": provider.token }),
    )
    .await
    .expect("re-declare accepted");
    assert_eq!(again["registration_token"], json!(provider.token.clone()));

    second.shutdown();
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
    let res = call(
        &provider.iii,
        "router::provider::resolve",
        json!({ "id": "real", "token": provider.token }),
    )
    .await
    .unwrap();
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
    let sup = call(
        &router_iii,
        "router::models::supports",
        json!({ "provider": "real", "id": "disc-1", "capability": "tools" }),
    )
    .await
    .unwrap();
    assert_eq!(sup["supported"], false); // flag absent on the discovered model

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
async fn models_changed_event_reaches_a_pubsub_subscriber() {
    let engine = engine_or_skip!();

    let router_iii = register_worker(&engine.url, InitOptions::default());
    register_router(router_iii.clone())
        .await
        .expect("router boots");

    // Probe worker bound to the topic through the engine's pubsub trigger type
    // (README § Events): the handler must receive the raw payload, no envelope.
    let probe = register_worker(&engine.url, InitOptions::default());
    let received = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
    let sink = received.clone();
    probe.register_function(
        "probe::on_models_changed",
        RegisterFunction::new_async(move |input: Value| {
            let sink = sink.clone();
            async move {
                sink.lock().unwrap().push(input);
                Ok::<Value, iii_sdk::IIIError>(json!({}))
            }
        }),
    );
    probe
        .register_trigger(iii_sdk::RegisterTriggerInput {
            trigger_type: "subscribe".into(),
            function_id: "probe::on_models_changed".into(),
            config: json!({ "topic": "router::models::changed" }),
            metadata: None,
        })
        .expect("subscribe trigger registered");

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

    let want = json!({ "provider": "real", "count": 2 });
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if received.lock().unwrap().iter().any(|p| p == &want) {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "router::models::changed never reached the pubsub subscriber; got {:?}",
            received.lock().unwrap()
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    probe.shutdown();
    router_iii.shutdown();
}

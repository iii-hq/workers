//! Live-engine integration suite — exercises the two SDK adapter files
//! (bus_sdk.rs, channels.rs) against a real iii engine.
//!
//! Ignored by default; run with:
//!   III_ENGINE_BIN=/path/to/iii cargo test --test integration -- --ignored
use std::io::Write as _;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::{register_worker, InitOptions, RegisterFunction, TriggerRequest};
use llm_router::bus_sdk::SdkBus;
use llm_router::channels::SdkChannels;
use llm_router::register::register_router;
use serde_json::{json, Value};

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

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

/// Spawn a minimal engine in a temp dir; poll until WS-reachable (the contract).
async fn spawn_engine() -> Engine {
    let bin = std::env::var("III_ENGINE_BIN").expect("III_ENGINE_BIN must point at the iii binary");
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

    Engine { url, child, dir }
}

/// A live provider worker on its own connection: registers
/// provider::real::stream and declares itself to the router.
async fn start_live_provider(
    url: &str,
    ping_forever: bool,
) -> (iii_sdk::III, Arc<AtomicBool>, Arc<AtomicU64>) {
    let iii = register_worker(url, InitOptions::default());
    let write_failed = Arc::new(AtomicBool::new(false));
    let fail_at_ms = Arc::new(AtomicU64::new(0));

    let address = url.to_string();
    let wf = write_failed.clone();
    let fam = fail_at_ms.clone();
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

    // declare (with a short retry in case the router is still booting)
    for _ in 0..50 {
        let res = iii
            .trigger(TriggerRequest {
                function_id: "router::provider::register".into(),
                payload: json!({
                    "id": "real",
                    "models": [{ "id": "live-1", "provider": "real", "context_window": 100000, "max_output_tokens": 8192 }]
                }),
                action: None,
                timeout_ms: Some(2000),
            })
            .await;
        if res.is_ok() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    (iii, write_failed, fail_at_ms)
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs III_ENGINE_BIN"]
async fn end_to_end_relay_over_a_live_engine() {
    let engine = spawn_engine().await;

    // router on its own connection
    let router_iii = register_worker(&engine.url, InitOptions::default());
    let bus = Arc::new(SdkBus {
        iii: router_iii.clone(),
    });
    let channels = Arc::new(SdkChannels {
        iii: router_iii.clone(),
    });
    register_router(bus, channels).await.expect("router boots");

    // provider on a second connection
    let (_provider_iii, _, _) = start_live_provider(&engine.url, false).await;

    // consumer on a third connection
    let consumer = register_worker(&engine.url, InitOptions::default());
    let channel = iii_sdk::helpers::create_channel(&consumer, None)
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
    let pump = tokio::spawn(async move {
        let _ = channel.reader.read_all().await; // drives text-message dispatch
    });

    let res = consumer
        .trigger(TriggerRequest {
            function_id: "router::chat".into(),
            payload: json!({ "writer_ref": channel.writer_ref, "model": "live-1", "messages": [] }),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await
        .expect("chat succeeds");
    assert_eq!(res["ok"], true, "chat response: {res}");
    assert_eq!(res["stop_reason"], "end");

    let _ = tokio::time::timeout(Duration::from_secs(5), pump).await;
    let frames = frames.lock().unwrap();
    assert!(frames.len() >= 2, "want >=2 frames, got {}", frames.len());
    let last: Value = serde_json::from_str(frames.last().unwrap()).unwrap();
    assert_eq!(last["type"], "done");

    consumer.shutdown();
    router_iii.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs III_ENGINE_BIN"]
async fn consumer_cancellation_propagates_to_the_provider() {
    let engine = spawn_engine().await;

    let router_iii = register_worker(&engine.url, InitOptions::default());
    let bus = Arc::new(SdkBus {
        iii: router_iii.clone(),
    });
    let channels = Arc::new(SdkChannels {
        iii: router_iii.clone(),
    });
    register_router(bus, channels).await.expect("router boots");

    let (_provider_iii, write_failed, fail_at_ms) = start_live_provider(&engine.url, true).await;

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
    while !write_failed.load(Ordering::SeqCst) {
        assert!(
            Instant::now() < deadline,
            "provider writes still succeeding 5s after the consumer left"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    println!(
        "cancellation latency: provider write failed {}ms after stream start",
        fail_at_ms.load(Ordering::SeqCst)
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
#[ignore = "needs III_ENGINE_BIN"]
async fn registry_survives_a_router_restart() {
    let engine = spawn_engine().await;

    let first = register_worker(&engine.url, InitOptions::default());
    register_router(
        Arc::new(SdkBus { iii: first.clone() }),
        Arc::new(SdkChannels { iii: first.clone() }),
    )
    .await
    .expect("router boots");

    let (_provider_iii, _, _) = start_live_provider(&engine.url, false).await;
    first.shutdown(); // "crash" the router connection

    let second = register_worker(&engine.url, InitOptions::default());
    register_router(
        Arc::new(SdkBus {
            iii: second.clone(),
        }),
        Arc::new(SdkChannels {
            iii: second.clone(),
        }),
    )
    .await
    .expect("router reboots");

    let list = second
        .trigger(TriggerRequest {
            function_id: "router::provider::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .expect("provider list");
    assert_eq!(list["providers"][0]["id"], "real", "list: {list}");

    second.shutdown();
}

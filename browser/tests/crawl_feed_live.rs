//! Live check of the crawl item feed on an engine WITHOUT a stream worker:
//! spawn `iii` (worker manager only) + the worker binary, serve a static
//! site on loopback, bind `browser::crawl-item` from a consumer worker, crawl,
//! and read the retained items back with `browser::crawl::items`.
//!
//! Self-skips when `iii` is not on PATH. Needs no Chromium (http fetcher).
//! Ports default to free ones; `BROWSER_CRAWL_LIVE_ENGINE_PORT` and
//! `BROWSER_CRAWL_LIVE_SITE_PORT` pin them. Scratch files live under
//! cargo's per-target tmp dir; `BROWSER_CRAWL_LIVE_KEEP=1` keeps them (and
//! the engine/worker logs) for inspection.

use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{register_worker, InitOptions, RegisterFunction};
use serde_json::{json, Value};
use tokio::time::{sleep, timeout};

struct Live {
    iii: Child,
    worker: Child,
    engine_ws: String,
    dir: PathBuf,
}

impl Drop for Live {
    fn drop(&mut self) {
        // SAFETY: plain libc call on the child pid we spawned.
        unsafe {
            libc::kill(self.worker.id() as i32, libc::SIGTERM);
        }
        std::thread::sleep(Duration::from_millis(500));
        let _ = self.worker.kill();
        let _ = self.worker.wait();
        let _ = self.iii.kill();
        let _ = self.iii.wait();
        if std::env::var_os("BROWSER_CRAWL_LIVE_KEEP").is_none() {
            let _ = std::fs::remove_dir_all(&self.dir);
        } else {
            eprintln!("kept live crawl scratch dir: {}", self.dir.display());
        }
    }
}

fn port_from_env(var: &str) -> Option<u16> {
    match std::env::var(var) {
        Ok(value) => value.parse().ok(),
        Err(_) => Some(
            TcpListener::bind("127.0.0.1:0")
                .ok()?
                .local_addr()
                .ok()?
                .port(),
        ),
    }
}

/// A loopback site: `/` links to three pages, each with its own text.
fn serve_site(port: u16) -> String {
    let listener = TcpListener::bind(("127.0.0.1", port)).expect("bind site port");
    let addr = listener.local_addr().expect("site addr");
    let pages: HashMap<&'static str, &'static str> = HashMap::from([
        (
            "/",
            "<html><body><h1>home</h1><a href=\"/a\">a</a> <a href=\"/b\">b</a> \
             <a href=\"/c\">c</a></body></html>",
        ),
        ("/a", "<html><body><h1>page a</h1></body></html>"),
        ("/b", "<html><body><h1>page b</h1></body></html>"),
        ("/c", "<html><body><h1>page c</h1></body></html>"),
    ]);
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in listener.incoming().flatten() {
            let mut stream = stream;
            let mut buf = [0u8; 4096];
            let n = stream.read(&mut buf).unwrap_or(0);
            let request = String::from_utf8_lossy(&buf[..n]);
            let path = request.split_whitespace().nth(1).unwrap_or("/");
            let (status, body) = match pages.get(path) {
                Some(body) => ("200 OK", *body),
                None => ("404 Not Found", "missing"),
            };
            let response = format!(
                "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });
    format!("http://{addr}/")
}

async fn boot() -> Option<Live> {
    let iii_bin = which::which("iii").ok()?;
    let port = port_from_env("BROWSER_CRAWL_LIVE_ENGINE_PORT")?;
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "browser-crawl-live-{}",
        uuid::Uuid::new_v4().simple()
    ));
    std::fs::create_dir_all(&dir).ok()?;
    let config_path = dir.join("engine.yaml");
    // Only the worker manager is declared: no stream worker anywhere.
    std::fs::write(
        &config_path,
        format!("workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n"),
    )
    .ok()?;
    let engine_log = std::fs::File::create(dir.join("engine.log")).ok()?;
    let mut iii = Command::new(&iii_bin)
        .args(["--config", config_path.to_str()?, "--no-update-check"])
        .current_dir(&dir)
        .stdout(Stdio::from(engine_log.try_clone().ok()?))
        .stderr(Stdio::from(engine_log))
        .spawn()
        .ok()?;
    sleep(Duration::from_millis(1000)).await;

    let seed_path = dir.join("browser-seed.yaml");
    std::fs::write(
        &seed_path,
        "browser:\n  scrapling:\n    allow_loopback: true\n",
    )
    .ok()?;
    let worker_log = std::fs::File::create(dir.join("worker.log")).ok()?;
    let worker = match Command::new(env!("CARGO_BIN_EXE_browser"))
        .arg("--url")
        .arg(format!("ws://127.0.0.1:{port}"))
        .arg("--config")
        .arg(&seed_path)
        .env("III_COMPOSE_DIR", &dir)
        .env("RUST_LOG", "info")
        .stdout(Stdio::from(worker_log.try_clone().ok()?))
        .stderr(Stdio::from(worker_log))
        .spawn()
    {
        Ok(worker) => worker,
        Err(_) => {
            let _ = iii.kill();
            let _ = iii.wait();
            return None;
        }
    };
    sleep(Duration::from_millis(3000)).await;
    Some(Live {
        iii,
        worker,
        engine_ws: format!("ws://127.0.0.1:{port}"),
        dir,
    })
}

#[tokio::test]
async fn crawl_items_reach_a_bound_consumer_and_are_recoverable_without_a_stream_worker() {
    let Some(live) = boot().await else {
        eprintln!("skipping: `iii` not available");
        return;
    };
    let site_port = port_from_env("BROWSER_CRAWL_LIVE_SITE_PORT").expect("site port");
    let site = serve_site(site_port);
    let client = register_worker(&live.engine_ws, InitOptions::default());
    sleep(Duration::from_millis(500)).await;
    let call = |function_id: &'static str, payload: Value| {
        let client = &client;
        async move {
            timeout(
                Duration::from_secs(60),
                client.trigger(TriggerRequest {
                    function_id: function_id.into(),
                    payload,
                    action: None,
                    timeout_ms: Some(55_000),
                }),
            )
            .await
            .expect("call timed out")
        }
    };

    // 1. The engine runs no stream worker.
    let listed = call("engine::functions::list", json!({}))
        .await
        .expect("functions list");
    let ids: Vec<String> = listed["functions"]
        .as_array()
        .expect("functions array")
        .iter()
        .filter_map(|f| f["function_id"].as_str().map(str::to_string))
        .collect();
    assert!(
        ids.iter().any(|id| id == "browser::crawl::items"),
        "{ids:?}"
    );
    assert!(
        !ids.iter().any(|id| id.starts_with("stream::")),
        "engine must run without a stream worker: {ids:?}"
    );
    let no_stream = call("stream::list", json!({"stream_name": "browser::crawl"})).await;
    eprintln!("stream::list on this engine -> {no_stream:?}");
    assert!(no_stream.is_err(), "stream::list must not exist here");

    // 2. A consumer binds the crawl-item trigger for one crawl id, and a
    //    second binding watches a different crawl.
    let (events_tx, mut events) = tokio::sync::mpsc::unbounded_channel::<Value>();
    client.register_function(
        "crawltest::on-item",
        RegisterFunction::new_async(move |event: Value| {
            let events_tx = events_tx.clone();
            async move {
                let _ = events_tx.send(event);
                Ok::<_, iii_sdk::errors::Error>(json!({"ok": true}))
            }
        }),
    );
    let stray = Arc::new(Mutex::new(Vec::<Value>::new()));
    {
        let stray = stray.clone();
        client.register_function(
            "crawltest::on-other",
            RegisterFunction::new_async(move |event: Value| {
                let stray = stray.clone();
                async move {
                    stray.lock().unwrap().push(event);
                    Ok::<_, iii_sdk::errors::Error>(json!({"ok": true}))
                }
            }),
        );
    }
    client
        .register_trigger(RegisterTriggerInput::new(
            "browser::crawl-item".to_string(),
            "crawltest::on-item".to_string(),
            json!({"crawl_id": "live-1"}),
        ))
        .expect("bind crawl-item");
    client
        .register_trigger(RegisterTriggerInput::new(
            "browser::crawl-item".to_string(),
            "crawltest::on-other".to_string(),
            json!({"crawl_id": "someone-else"}),
        ))
        .expect("bind other crawl-item");
    sleep(Duration::from_millis(1000)).await;

    // 3. Crawl, still passing the deprecated stream_name.
    let result = call(
        "browser::crawl",
        json!({
            "url": site,
            "crawl_id": "live-1",
            "max_depth": 1,
            "concurrency": 1,
            "format": "text",
            "stream_name": "legacy-name",
        }),
    )
    .await
    .expect("crawl");
    eprintln!("browser::crawl result: {result:#}");
    assert_eq!(
        result["stats"],
        json!({"crawled": 4, "items": 4, "errors": 0, "stopped": "done"})
    );
    assert_eq!(result["items"].as_array().unwrap().len(), 4, "sample");
    assert_eq!(
        result["stream"],
        json!({"name": "legacy-name", "group_id": "live-1"}),
        "deprecated echo unchanged"
    );
    assert_eq!(result["crawl"]["id"], "live-1");
    assert_eq!(result["crawl"]["retained"], 4);
    assert_eq!(result["crawl"]["dropped_events"], 0);
    assert!(result["warnings"][0]
        .as_str()
        .unwrap()
        .starts_with("stream_name is deprecated and ignored"));

    // 4. Live events: 4 items in seq order, then done.
    let mut received = Vec::new();
    while received.len() < 5 {
        let event = timeout(Duration::from_secs(10), events.recv())
            .await
            .expect("crawl-item event in time")
            .expect("channel open");
        received.push(event);
    }
    eprintln!(
        "browser::crawl-item events: {}",
        Value::Array(received.clone())
    );
    for (i, event) in received.iter().enumerate() {
        assert_eq!(event["crawl_id"], "live-1");
        assert_eq!(event["seq"], json!(i + 1), "seq order");
    }
    for event in &received[..4] {
        assert_eq!(event["event"], "item");
        assert_eq!(event["truncated"], false);
    }
    assert_eq!(received[4]["event"], "done");
    assert_eq!(received[4]["stats"], result["stats"]);
    assert_eq!(received[4]["retained"], 4);
    let mut urls: Vec<String> = received[..4]
        .iter()
        .map(|e| e["item"]["url"].as_str().unwrap().to_string())
        .collect();
    urls.sort();
    assert_eq!(
        urls,
        ["a", "b", "c"]
            .iter()
            .map(|p| format!("{site}{p}"))
            .chain([site.clone()])
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect::<Vec<_>>()
    );

    // 5. Recovery: page through the retained items; they match the events
    //    and the response sample.
    let first = call(
        "browser::crawl::items",
        json!({"crawl_id": "live-1", "limit": 2}),
    )
    .await
    .expect("items page 1");
    assert_eq!(first["items"].as_array().unwrap().len(), 2);
    assert_eq!(first["next_after"], 2);
    let rest = call(
        "browser::crawl::items",
        json!({"crawl_id": "live-1", "after": first["next_after"], "limit": 2}),
    )
    .await
    .expect("items page 2");
    assert!(rest.get("next_after").is_none());
    assert_eq!(rest["stats"], result["stats"]);
    assert_eq!(rest["running"], false);
    let retained: Vec<Value> = first["items"]
        .as_array()
        .unwrap()
        .iter()
        .chain(rest["items"].as_array().unwrap())
        .cloned()
        .collect();
    for (i, entry) in retained.iter().enumerate() {
        assert_eq!(entry["seq"], json!(i + 1));
        assert_eq!(entry["item"], received[i]["item"], "event == retained item");
        assert_eq!(entry["item"], result["items"][i], "sample == retained item");
    }

    // 6. The other crawl's binding saw nothing; a missing id is an error.
    assert!(stray.lock().unwrap().is_empty(), "crawl_id filter");
    let missing = call("browser::crawl::items", json!({"crawl_id": "nope"})).await;
    assert!(
        format!("{missing:?}").contains("unknown or expired"),
        "{missing:?}"
    );

    // 7. The engine logged no deprecation for this worker.
    let engine_log = std::fs::read_to_string(live.dir.join("engine.log")).unwrap_or_default();
    assert!(
        !engine_log.contains("iii::deprecation"),
        "no stream deprecation warnings expected"
    );
    client.shutdown_async().await;
}

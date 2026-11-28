use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use browser::chromium;
use browser::config::WorkerConfig;
use browser::events::{Emitter, EventKind, EventSink};
use browser::session::{SessionKind, Sessions};
use futures::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;

struct NoopSink;

#[async_trait]
impl EventSink for NoopSink {
    fn is_observed(&self, _kind: EventKind, _session_id: &str) -> bool {
        false
    }

    async fn call(&self, _kind: EventKind, _event: Value) {}
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .expect("bind ephemeral port")
        .local_addr()
        .expect("local addr")
        .port()
}

fn test_sessions(temp_dir: &std::path::Path) -> Arc<Sessions> {
    let cfg = WorkerConfig {
        data_dir: temp_dir.to_string_lossy().to_string(),
        allow_attach: true,
        allowed_schemes: vec![
            "http".to_string(),
            "https".to_string(),
            "about".to_string(),
            "data".to_string(),
        ],
        ..Default::default()
    };
    let shared = cfg.into_shared();
    let emitter = Arc::new(Emitter::new(Arc::new(NoopSink)));
    let iii = Arc::new(iii_sdk::register_worker(
        "ws://127.0.0.1:1",
        iii_sdk::InitOptions::default(),
    ));
    Sessions::new(shared, emitter, iii)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new(prefix: &str) -> Self {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let dir = std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        Self(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Clone, Copy)]
enum CloseBehavior {
    Error,
    Hang,
}

struct MockCdpState {
    close_behavior: CloseBehavior,
    saw_browser_close: AtomicBool,
    closed_targets: tokio::sync::Mutex<Vec<String>>,
}

async fn spawn_mock_cdp_server(
    behavior: CloseBehavior,
) -> (String, Arc<MockCdpState>, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mock cdp");
    let port = listener.local_addr().unwrap().port();
    let state = Arc::new(MockCdpState {
        close_behavior: behavior,
        saw_browser_close: AtomicBool::new(false),
        closed_targets: tokio::sync::Mutex::new(Vec::new()),
    });
    let state_clone = state.clone();

    let handle = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let state = state_clone.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let Ok(n) = stream.peek(&mut buf).await else {
                    return;
                };
                let head = String::from_utf8_lossy(&buf[..n]);
                if head.contains("Upgrade: websocket") || head.contains("upgrade: websocket") {
                    handle_mock_ws(stream, state).await;
                } else {
                    let mut req_buf = vec![0u8; 4096];
                    let _ = stream.read(&mut req_buf).await;
                    let req = String::from_utf8_lossy(&req_buf);
                    let body = if req.starts_with("GET /json/version") {
                        json!({
                            "Browser": "Chrome/120.0.0.0",
                            "Protocol-Version": "1.3",
                            "webSocketDebuggerUrl": format!("ws://127.0.0.1:{port}/devtools/browser/mock")
                        })
                        .to_string()
                    } else {
                        json!([
                            {
                                "id": "adopted-user-tab",
                                "type": "page",
                                "title": "User Tab",
                                "url": "http://127.0.0.1/user",
                                "webSocketDebuggerUrl": format!("ws://127.0.0.1:{port}/devtools/page/adopted-user-tab")
                            }
                        ])
                        .to_string()
                    };
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                }
            });
        }
    });

    (
        format!("ws://127.0.0.1:{port}/devtools/browser/mock"),
        state,
        handle,
    )
}

async fn handle_mock_ws(stream: TcpStream, state: Arc<MockCdpState>) {
    let Ok(ws) = tokio_tungstenite::accept_async(stream).await else {
        return;
    };
    let (mut write, mut read) = ws.split();
    while let Some(Ok(msg)) = read.next().await {
        let Message::Text(text) = msg else {
            continue;
        };
        let Ok(req): Result<Value, _> = serde_json::from_str(&text) else {
            continue;
        };
        let Some(id) = req.get("id").and_then(|v| v.as_u64()) else {
            continue;
        };
        let method = req.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let req_session_id = req
            .get("sessionId")
            .and_then(|v| v.as_str())
            .map(str::to_string);
        let reply = |result: Value| {
            let mut obj = json!({ "id": id, "result": result });
            if let Some(sid) = &req_session_id {
                obj["sessionId"] = json!(sid);
            }
            obj.to_string()
        };
        match method {
            "Browser.close" => {
                state.saw_browser_close.store(true, Ordering::SeqCst);
                let _ = write.send(Message::Text(reply(json!({})).into())).await;
            }
            "Target.createTarget" => {
                let target_id = "owned-target-1";
                let evt = json!({
                    "method": "Target.targetCreated",
                    "params": {
                        "targetInfo": {
                            "targetId": target_id,
                            "type": "page",
                            "title": "",
                            "url": "about:blank",
                            "attached": false,
                            "canAccessOpener": false
                        }
                    }
                });
                let _ = write.send(Message::Text(evt.to_string().into())).await;
                let _ = write
                    .send(Message::Text(
                        reply(json!({ "targetId": target_id })).into(),
                    ))
                    .await;
            }
            "Target.attachToTarget" => {
                let target_id = req["params"]["targetId"]
                    .as_str()
                    .unwrap_or("owned-target-1");
                let session_id = "cdp-session-1";
                let evt = json!({
                    "method": "Target.attachedToTarget",
                    "params": {
                        "sessionId": session_id,
                        "targetInfo": {
                            "targetId": target_id,
                            "type": "page",
                            "title": "",
                            "url": "about:blank",
                            "attached": true,
                            "canAccessOpener": false
                        },
                        "waitingForDebugger": false
                    }
                });
                let _ = write.send(Message::Text(evt.to_string().into())).await;
                let _ = write
                    .send(Message::Text(
                        reply(json!({ "sessionId": session_id })).into(),
                    ))
                    .await;
            }
            "Page.getFrameTree" => {
                let _ = write
                    .send(Message::Text(
                        reply(json!({
                            "frameTree": {
                                "frame": {
                                    "id": "frame-1",
                                    "loaderId": "loader-1",
                                    "url": "about:blank",
                                    "domainAndRegistry": "",
                                    "securityOrigin": "://",
                                    "mimeType": "text/html",
                                    "secureContextType": "Secure",
                                    "crossOriginIsolatedContextType": "NotIsolated",
                                    "gatedAPIFeatures": []
                                }
                            }
                        }))
                        .into(),
                    ))
                    .await;
            }
            "Page.setLifecycleEventsEnabled" => {
                let _ = write.send(Message::Text(reply(json!({})).into())).await;
                let mut evt = json!({
                    "method": "Page.lifecycleEvent",
                    "params": {
                        "frameId": "frame-1",
                        "loaderId": "loader-1",
                        "name": "load",
                        "timestamp": 1.0
                    }
                });
                if let Some(sid) = &req_session_id {
                    evt["sessionId"] = json!(sid);
                }
                let _ = write.send(Message::Text(evt.to_string().into())).await;
            }
            "Runtime.enable" => {
                let _ = write.send(Message::Text(reply(json!({})).into())).await;
                let mut evt = json!({
                    "method": "Runtime.executionContextCreated",
                    "params": {
                        "context": {
                            "id": 1,
                            "origin": "://",
                            "name": "",
                            "auxData": {
                                "isDefault": true,
                                "type": "default",
                                "frameId": "frame-1"
                            }
                        }
                    }
                });
                if let Some(sid) = &req_session_id {
                    evt["sessionId"] = json!(sid);
                }
                let _ = write.send(Message::Text(evt.to_string().into())).await;
            }
            "Network.setCacheDisabled" => {
                let _ = write.send(Message::Text(reply(json!({})).into())).await;
                for name in [
                    "init",
                    "DOMContentLoaded",
                    "load",
                    "networkAlmostIdle",
                    "networkIdle",
                ] {
                    let mut evt = json!({
                        "method": "Page.lifecycleEvent",
                        "params": {
                            "frameId": "frame-1",
                            "loaderId": "loader-1",
                            "name": name,
                            "timestamp": 1.0
                        }
                    });
                    if let Some(sid) = &req_session_id {
                        evt["sessionId"] = json!(sid);
                    }
                    let _ = write.send(Message::Text(evt.to_string().into())).await;
                }
                let mut stopped = json!({
                    "method": "Page.frameStoppedLoading",
                    "params": { "frameId": "frame-1" }
                });
                if let Some(sid) = &req_session_id {
                    stopped["sessionId"] = json!(sid);
                }
                let _ = write.send(Message::Text(stopped.to_string().into())).await;
            }
            "Page.navigate" => {
                let _ = write
                    .send(Message::Text(
                        reply(json!({ "frameId": "frame-1", "loaderId": "loader-1" })).into(),
                    ))
                    .await;
            }
            "Target.closeTarget" => {
                if let Some(tid) = req["params"]["targetId"].as_str() {
                    state.closed_targets.lock().await.push(tid.to_string());
                }
                match state.close_behavior {
                    CloseBehavior::Error => {
                        let mut err = json!({
                            "id": id,
                            "error": { "code": -32000, "message": "injected close error" }
                        });
                        if let Some(sid) = &req_session_id {
                            err["sessionId"] = json!(sid);
                        }
                        let _ = write.send(Message::Text(err.to_string().into())).await;
                    }
                    CloseBehavior::Hang => {
                        // Intentionally withhold response to simulate an unresponsive browser.
                    }
                }
            }
            _ => {
                let _ = write.send(Message::Text(reply(json!({})).into())).await;
            }
        }
    }
}

#[tokio::test]
async fn attached_shutdown_explicit_cdp_error_reports_failure_and_detaches() {
    let dir = TempDir::new("browser-attach-err");
    let sessions = test_sessions(&dir.0);
    let (cdp_url, state, server_task) = spawn_mock_cdp_server(CloseBehavior::Error).await;

    let session = tokio::time::timeout(
        Duration::from_secs(10),
        sessions.attach(cdp_url, None, None, false),
    )
    .await
    .expect("attach timed out")
    .expect("attach to mock cdp");
    assert_eq!(session.kind, SessionKind::Attached { owns_page: true });

    let outcome = tokio::time::timeout(
        Duration::from_secs(10),
        sessions.stop(&session.tab.id, "stopped"),
    )
    .await
    .expect("stop should complete promptly on explicit CDP error");

    assert!(outcome.was_running, "session record should have existed");
    assert!(!outcome.ok, "close failure must set ok=false");
    let err = outcome.error.expect("error message should be populated");
    assert!(
        err.contains("injected close error") || err.contains("failed to close target"),
        "unexpected error: {err}"
    );
    assert!(
        sessions.list_tabs().is_empty(),
        "session record must be removed even on close failure"
    );
    assert!(
        !state.saw_browser_close.load(Ordering::SeqCst),
        "external browser must never be closed as a fallback"
    );
    let closed = state.closed_targets.lock().await.clone();
    assert_eq!(closed, vec!["owned-target-1".to_string()]);

    server_task.abort();
}

#[tokio::test]
async fn attached_shutdown_unresponsive_browser_bounds_timeout_and_detaches() {
    let dir = TempDir::new("browser-attach-hang");
    let sessions = test_sessions(&dir.0);
    let (cdp_url, state, server_task) = spawn_mock_cdp_server(CloseBehavior::Hang).await;

    let session = tokio::time::timeout(
        Duration::from_secs(10),
        sessions.attach(cdp_url, None, None, false),
    )
    .await
    .expect("attach timed out")
    .expect("attach to mock cdp");
    assert_eq!(session.kind, SessionKind::Attached { owns_page: true });

    let outcome = tokio::time::timeout(
        Duration::from_secs(12),
        sessions.stop(&session.tab.id, "stopped"),
    )
    .await
    .expect("stop must finish within bounded timeout instead of hanging");

    assert!(outcome.was_running, "session record should have existed");
    assert!(!outcome.ok, "timed-out close must set ok=false");
    let err = outcome.error.expect("timeout error should be populated");
    assert!(
        err.contains("timed out"),
        "expected timeout message, got: {err}"
    );
    assert!(
        sessions.list_tabs().is_empty(),
        "session record must be removed even on timeout"
    );
    assert!(
        !state.saw_browser_close.load(Ordering::SeqCst),
        "external browser must never be closed on timeout"
    );

    server_task.abort();
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[tokio::test]
async fn attached_shutdown_real_chromium_closes_owned_tabs_stops_activity_and_preserves_adopted() {
    let cfg = WorkerConfig::default();
    let Some(resolved) = chromium::resolve_executable(&cfg) else {
        eprintln!("skipping real-CDP test: Chromium not available");
        return;
    };

    let data_dir = TempDir::new("browser-attach-real-data");
    let profile_dir = TempDir::new("browser-attach-real-profile");
    let sessions = test_sessions(&data_dir.0);

    // Local HTTP server with a user page and a polling page per cycle.
    let http_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind test http");
    let http_port = http_listener.local_addr().unwrap().port();
    let pings: Arc<Vec<AtomicUsize>> = Arc::new((0..3).map(|_| AtomicUsize::new(0)).collect());
    let pings_clone = pings.clone();
    let http_task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = http_listener.accept().await else {
                break;
            };
            let pings = pings_clone.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 2048];
                let Ok(n) = stream.read(&mut buf).await else {
                    return;
                };
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req
                    .lines()
                    .next()
                    .and_then(|l| l.split_whitespace().nth(1))
                    .unwrap_or("/");
                let (content_type, body) = if let Some(rest) = path.strip_prefix("/ping?cycle=") {
                    if let Ok(idx) = rest.parse::<usize>() {
                        if let Some(counter) = pings.get(idx) {
                            counter.fetch_add(1, Ordering::SeqCst);
                        }
                    }
                    ("text/plain", "ok".to_string())
                } else if let Some(rest) = path.strip_prefix("/poll?cycle=") {
                    let html = format!(
                        "<!doctype html><html><body><script>setInterval(() => fetch('/ping?cycle={rest}'), 50);</script></body></html>"
                    );
                    ("text/html", html)
                } else {
                    (
                        "text/html",
                        "<!doctype html><html><body>user-tab</body></html>".to_string(),
                    )
                };
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(resp.as_bytes()).await;
            });
        }
    });

    let cdp_port = free_port();
    let user_url = format!("http://127.0.0.1:{http_port}/user");
    let mut cmd = Command::new(&resolved.path);
    cmd.arg(format!("--remote-debugging-port={cdp_port}"))
        .arg(format!("--user-data-dir={}", profile_dir.0.display()))
        .arg("--headless=new")
        .arg("--no-first-run")
        .arg("--no-default-browser-check")
        .arg(&user_url)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if chromium::needs_no_sandbox(&resolved) {
        cmd.arg("--no-sandbox");
    }
    let mut child = ChildGuard(cmd.spawn().expect("spawn external chromium"));

    let cdp_base = format!("http://127.0.0.1:{cdp_port}");
    let client = reqwest::Client::new();
    // Wait for CDP endpoint to be ready with the user tab.
    let mut ready = false;
    for _ in 0..50 {
        if let Ok(resp) = client.get(format!("{cdp_base}/json/list")).send().await {
            if let Ok(list) = resp.json::<Vec<Value>>().await {
                if list
                    .iter()
                    .any(|t| t["url"].as_str().is_some_and(|u| u.contains("/user")))
                {
                    ready = true;
                    break;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(ready, "external chromium CDP endpoint did not become ready");

    let version: Value = client
        .get(format!("{cdp_base}/json/version"))
        .send()
        .await
        .expect("query /json/version")
        .json()
        .await
        .expect("parse /json/version");
    let cdp_ws = version["webSocketDebuggerUrl"]
        .as_str()
        .expect("webSocketDebuggerUrl")
        .to_string();

    // Adopt the pre-existing user tab.
    let adopted = sessions
        .attach(cdp_ws.clone(), None, Some("/user".to_string()), false)
        .await
        .expect("adopt user tab");
    assert_eq!(adopted.kind, SessionKind::Attached { owns_page: false });
    let adopted_target_id = adopted.page.target_id().as_ref().to_string();

    // Run 3 owned attach/stop cycles; verify physical target removal and JS cessation.
    for cycle in 0..3 {
        let poll_url = format!("http://127.0.0.1:{http_port}/poll?cycle={cycle}");
        let owned = sessions
            .attach(cdp_ws.clone(), Some(poll_url), None, false)
            .await
            .expect("attach owned tab");
        assert_eq!(owned.kind, SessionKind::Attached { owns_page: true });
        let owned_target_id = owned.page.target_id().as_ref().to_string();

        // Wait until the polling page has made at least one request.
        let mut saw_ping = false;
        for _ in 0..40 {
            if pings[cycle].load(Ordering::SeqCst) > 0 {
                saw_ping = true;
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert!(saw_ping, "cycle {cycle}: owned tab never started polling");

        let outcome = sessions.stop(&owned.tab.id, "stopped").await;
        assert!(outcome.was_running, "cycle {cycle}: was_running");
        assert!(outcome.ok, "cycle {cycle}: ok=false: {:?}", outcome.error);
        assert!(outcome.error.is_none());

        // Logical removal: only the adopted session remains.
        let live_ids: Vec<String> = sessions
            .list_tabs()
            .into_iter()
            .map(|t| t.id.clone())
            .collect();
        assert_eq!(live_ids, vec![adopted.tab.id.clone()]);

        // Physical removal: owned target must be absent from CDP /json/list.
        let list: Vec<Value> = client
            .get(format!("{cdp_base}/json/list"))
            .send()
            .await
            .expect("query /json/list")
            .json()
            .await
            .expect("parse /json/list");
        assert!(
            !list
                .iter()
                .any(|t| t["id"].as_str() == Some(owned_target_id.as_str())),
            "cycle {cycle}: owned target {owned_target_id} still present in CDP list: {list:?}"
        );

        // Cessation of page JS activity.
        let count_after_stop = pings[cycle].load(Ordering::SeqCst);
        tokio::time::sleep(Duration::from_millis(300)).await;
        let count_later = pings[cycle].load(Ordering::SeqCst);
        assert_eq!(
            count_later, count_after_stop,
            "cycle {cycle}: owned page kept running after stop ({count_after_stop} -> {count_later})"
        );
    }

    // Stopping the adopted session must leave the user's tab and browser alive.
    let adopted_outcome = sessions.stop(&adopted.tab.id, "stopped").await;
    assert!(adopted_outcome.was_running);
    assert!(adopted_outcome.ok);
    assert!(sessions.list_tabs().is_empty());

    let list: Vec<Value> = client
        .get(format!("{cdp_base}/json/list"))
        .send()
        .await
        .expect("query /json/list after adopted stop")
        .json()
        .await
        .expect("parse /json/list");
    assert!(
        list.iter()
            .any(|t| t["id"].as_str() == Some(adopted_target_id.as_str())),
        "adopted target {adopted_target_id} was closed: {list:?}"
    );
    assert!(
        child.0.try_wait().expect("try_wait").is_none(),
        "external chromium process should still be running"
    );

    http_task.abort();
}

//! `loopback_only` routes, end to end over real sockets with the fake engine:
//! admission is decided per accepted connection from the socket's LOCAL
//! address, so a loopback_only route is reachable over loopback and absent
//! (404, like an unknown path) on any other local address, a rebind included.
mod common;

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::Body;
use axum::http::Request;
use common::fake_engine::FakeEngine;
use iii_http::boot::{self, BootHandle};
use iii_http::config::{RestApiConfig, WebhookListenerConfig};
use iii_http::configuration;
use iii_http::on_request::ON_REQUEST_FN;
use iii_http::types::HttpRequest;
use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, RegisterFunction};
use serde_json::{Value, json};
use tower::ServiceExt;

fn config(host: &str) -> RestApiConfig {
    RestApiConfig {
        host: host.into(),
        port: 0,
        webhook_listener: Some(WebhookListenerConfig {
            port: 0,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Boots the worker on `host` (port 0) with the config observers, so a
/// config change can rebind it. `None` when `host` cannot be bound here.
async fn start_on(host: &str) -> Option<(FakeEngine, BootHandle)> {
    let engine = FakeEngine::start().await;
    let boot = match boot::start(engine.iii.clone(), config(host)).await {
        Ok(boot) => boot,
        Err(e) => {
            eprintln!("skipping: cannot bind {host}: {e}");
            engine.shutdown().await;
            return None;
        }
    };
    configuration::register_config_observer(
        &engine.iii,
        boot.config.clone(),
        boot.hot_router.clone(),
        boot.control.clone(),
        boot.apply_lock.clone(),
    );
    Some((engine, boot))
}

async fn start() -> (FakeEngine, BootHandle) {
    start_on("127.0.0.1").await.expect("127.0.0.1 binds")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap()
}

/// A non-loopback address of this host that this process can connect to,
/// from the route to a TEST-NET address (a UDP connect sends nothing). `None`
/// on a host without one, or where a firewall drops connections to it (macOS
/// application firewall): the constructed-address tests below still cover
/// the admission there.
async fn reachable_non_loopback_ip() -> Option<IpAddr> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    if ip.is_loopback() || ip.is_unspecified() {
        return None;
    }
    let probe = tokio::net::TcpListener::bind("0.0.0.0:0").await.ok()?;
    let port = probe.local_addr().ok()?.port();
    let connect = tokio::net::TcpStream::connect((ip, port));
    match tokio::time::timeout(Duration::from_secs(1), connect).await {
        Ok(Ok(_)) => Some(ip),
        _ => {
            eprintln!("skipping: {ip} is not reachable from this process");
            None
        }
    }
}

/// A target answering 201 that counts its calls.
fn register_target(engine: &FakeEngine, id: &str) -> Arc<AtomicUsize> {
    let calls = Arc::new(AtomicUsize::new(0));
    let counter = calls.clone();
    engine.iii.register_function(
        id.to_string(),
        RegisterFunction::new_async(move |req: HttpRequest| {
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                Ok::<Value, Error>(json!({
                    "status_code": 201,
                    "body": { "path": req.path, "method": req.method }
                }))
            }
        }),
    );
    calls
}

async fn observe(engine: &FakeEngine, id: &str, target: &str, args: Value) -> Option<Value> {
    engine
        .add_observer(
            ON_REQUEST_FN,
            json!({ "id": id, "target": target, "args": args, "filters": {} }),
        )
        .await
}

/// Serves `method` `path` (`loopback_only` as given) through a fresh target.
async fn serve(
    engine: &FakeEngine,
    boot: &BootHandle,
    method: &str,
    path: &str,
    loopback_only: bool,
) -> Arc<AtomicUsize> {
    let target = format!("test::{method}:{path}");
    let calls = register_target(engine, &target);
    let args = json!({ "method": method, "path": path, "loopback_only": loopback_only });
    let rejection = observe(engine, &format!("obs-{method}:{path}"), &target, args).await;
    assert!(rejection.is_none(), "{method} {path}: {rejection:?}");
    common::wait_for_route(&boot.routes, method, path).await;
    calls
}

struct Reply {
    status: u16,
    allow: Option<String>,
    body: Value,
}

async fn send(method: &str, url: String) -> Reply {
    let resp = client()
        .request(method.parse().unwrap(), url)
        .send()
        .await
        .unwrap();
    let status = resp.status().as_u16();
    let allow = resp
        .headers()
        .get("allow")
        .map(|v| v.to_str().unwrap().to_string());
    let body = resp.json().await.unwrap_or(Value::Null);
    Reply {
        status,
        allow,
        body,
    }
}

fn url(addr: SocketAddr, path: &str) -> String {
    format!("http://{addr}{path}")
}

/// Asserts `path` answers on `addr` exactly as an unknown path does.
async fn assert_absent(method: &str, addr: SocketAddr, path: &str) {
    let refused = send(method, url(addr, path)).await;
    let unknown = send(method, url(addr, "/no/such/route")).await;
    assert_eq!(refused.status, 404, "{method} {addr}{path}");
    assert_eq!(refused.allow, None, "{method} {addr}{path}: Allow leaked");
    assert_eq!(refused.body, unknown.body, "{method} {addr}{path}");
    assert_eq!(refused.body["error"]["code"], "NOT_FOUND");
}

#[tokio::test]
async fn a_loopback_only_route_is_served_over_loopback() {
    let (engine, boot) = start().await;
    let mcp = serve(&engine, &boot, "POST", "/mcp", true).await;
    let plain = serve(&engine, &boot, "GET", "/plain", false).await;
    let addr = boot.local_addr;
    assert!(addr.ip().is_loopback());

    let reply = send("POST", url(addr, "/mcp")).await;
    assert_eq!(reply.status, 201);
    assert_eq!(reply.body["path"], "/mcp");
    assert_eq!(mcp.load(Ordering::SeqCst), 1);
    // Over loopback the route exists: another method is a 405 naming it.
    let reply = send("GET", url(addr, "/mcp")).await;
    assert_eq!(reply.status, 405);
    assert_eq!(reply.allow.as_deref(), Some("POST"));
    assert_eq!(send("GET", url(addr, "/plain")).await.status, 201);
    assert_eq!(plain.load(Ordering::SeqCst), 1);

    // The restricted listener never serves it, over loopback or not.
    let restricted = boot.current_webhook_addr().await.unwrap();
    assert!(restricted.ip().is_loopback());
    let reply = send("POST", url(restricted, "/mcp")).await;
    assert_eq!(reply.status, 404);
    assert_eq!(reply.allow, None);
    assert_eq!(mcp.load(Ordering::SeqCst), 1);

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn a_loopback_only_route_is_served_over_ipv6_loopback() {
    let Some((engine, boot)) = start_on("::1").await else {
        return;
    };
    let mcp = serve(&engine, &boot, "POST", "/mcp", true).await;
    let addr = boot.local_addr;
    assert_eq!(addr.ip(), "::1".parse::<IpAddr>().unwrap());
    assert_eq!(send("POST", url(addr, "/mcp")).await.status, 201);
    assert_eq!(mcp.load(Ordering::SeqCst), 1);

    boot.shutdown().await;
    engine.shutdown().await;
}

/// What `method` `path` answers through the server's per-connection router
/// for a connection accepted on `local` (`None`: unknown local address).
async fn send_on(boot: &BootHandle, local: Option<&str>, method: &str, path: &str) -> Reply {
    let mut router = boot.hot_router.clone();
    router.connection_local_addr = local.map(|addr| addr.parse().unwrap());
    let request = Request::builder()
        .method(method)
        .uri(path)
        .body(Body::empty())
        .unwrap();
    let resp = router.oneshot(request).await.unwrap();
    let status = resp.status().as_u16();
    let allow = resp
        .headers()
        .get("allow")
        .map(|v| v.to_str().unwrap().to_string());
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    Reply {
        status,
        allow,
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    }
}

#[tokio::test]
async fn admission_follows_the_connections_local_address() {
    let (engine, boot) = start().await;
    let mcp = serve(&engine, &boot, "POST", "/mcp", true).await;
    let read = serve(&engine, &boot, "GET", "/state", true).await;
    let write = serve(&engine, &boot, "POST", "/state", false).await;
    let plain = serve(&engine, &boot, "GET", "/plain", false).await;

    let loopback = [
        "127.0.0.1:3111",
        "127.1.2.3:3111",
        "[::1]:3111",
        "[::ffff:127.0.0.1]:3111",
    ];
    let remote = [
        "10.0.0.5:3111",
        "192.168.1.20:3111",
        "[2001:db8::5]:3111",
        "[::ffff:192.168.1.20]:3111",
    ];
    let mut served = 0;
    for local in loopback {
        let reply = send_on(&boot, Some(local), "POST", "/mcp").await;
        assert_eq!(reply.status, 201, "{local}");
        served += 1;
        assert_eq!(mcp.load(Ordering::SeqCst), served, "{local}");
        let reply = send_on(&boot, Some(local), "GET", "/mcp").await;
        assert_eq!(reply.status, 405, "{local}");
        assert_eq!(reply.allow.as_deref(), Some("POST"), "{local}");
        let reply = send_on(&boot, Some(local), "DELETE", "/state").await;
        assert_eq!(reply.allow.as_deref(), Some("GET, POST"), "{local}");
    }
    let unknown = send_on(&boot, Some(remote[0]), "POST", "/no/such/route").await;
    assert_eq!(unknown.status, 404);
    for local in remote.iter().map(|addr| Some(*addr)).chain([None]) {
        for method in ["POST", "GET", "PUT"] {
            let reply = send_on(&boot, local, method, "/mcp").await;
            assert_eq!(reply.status, 404, "{local:?} {method}");
            assert_eq!(reply.allow, None, "{local:?} {method}: Allow leaked");
            assert_eq!(reply.body, unknown.body, "{local:?} {method}");
        }
        // The refused GET beside a plain POST: a 405 naming only POST.
        let reply = send_on(&boot, local, "GET", "/state").await;
        assert_eq!(reply.status, 405, "{local:?}");
        assert_eq!(reply.allow.as_deref(), Some("POST"), "{local:?}");
        assert_eq!(send_on(&boot, local, "POST", "/state").await.status, 201);
        // Plain routes are unaffected.
        assert_eq!(send_on(&boot, local, "GET", "/plain").await.status, 201);
    }
    assert_eq!(mcp.load(Ordering::SeqCst), loopback.len());
    assert_eq!(read.load(Ordering::SeqCst), 0);
    assert_eq!(write.load(Ordering::SeqCst), remote.len() + 1);
    assert_eq!(plain.load(Ordering::SeqCst), remote.len() + 1);

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn on_an_unspecified_bind_only_loopback_connections_reach_the_route() {
    let Some(public_ip) = reachable_non_loopback_ip().await else {
        return;
    };
    let (engine, boot) = start_on("0.0.0.0").await.expect("0.0.0.0 binds");
    let port = boot.local_addr.port();
    let loopback: SocketAddr = (IpAddr::from([127, 0, 0, 1]), port).into();
    let public: SocketAddr = (public_ip, port).into();

    let mcp = serve(&engine, &boot, "POST", "/mcp", true).await;
    let admin = serve(&engine, &boot, "GET", "/admin/:section", true).await;
    let read = serve(&engine, &boot, "GET", "/state", true).await;
    let write = serve(&engine, &boot, "POST", "/state", false).await;
    let plain = serve(&engine, &boot, "GET", "/plain", false).await;

    // A client connecting over loopback is admitted.
    assert_eq!(send("POST", url(loopback, "/mcp")).await.status, 201);
    assert_eq!(send("GET", url(loopback, "/admin/users")).await.status, 201);
    assert_eq!(send("GET", url(loopback, "/state")).await.status, 201);
    let reply = send("DELETE", url(loopback, "/state")).await;
    assert_eq!(reply.status, 405);
    assert_eq!(reply.allow.as_deref(), Some("GET, POST"));
    assert_eq!(mcp.load(Ordering::SeqCst), 1);
    assert_eq!(admin.load(Ordering::SeqCst), 1);
    assert_eq!(read.load(Ordering::SeqCst), 1);

    // A connection accepted on the public address is not: the route does not
    // exist there, whatever the method, and its target is never called.
    for method in ["POST", "GET", "PUT"] {
        assert_absent(method, public, "/mcp").await;
    }
    assert_absent("GET", public, "/admin/users").await;
    assert_eq!(mcp.load(Ordering::SeqCst), 1);
    assert_eq!(admin.load(Ordering::SeqCst), 1);
    // A path with a plain route beside a refused one: only the plain method
    // is allowed, and the refused one is a 405 naming only it.
    let reply = send("GET", url(public, "/state")).await;
    assert_eq!(reply.status, 405);
    assert_eq!(reply.allow.as_deref(), Some("POST"));
    assert_eq!(read.load(Ordering::SeqCst), 1);
    assert_eq!(send("POST", url(public, "/state")).await.status, 201);
    assert_eq!(write.load(Ordering::SeqCst), 1);

    // Plain routes are unaffected on both.
    assert_eq!(send("GET", url(loopback, "/plain")).await.status, 201);
    assert_eq!(send("GET", url(public, "/plain")).await.status, 201);
    assert_eq!(plain.load(Ordering::SeqCst), 2);

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn a_rebind_to_a_public_address_never_exposes_the_route() {
    let Some(public_ip) = reachable_non_loopback_ip().await else {
        return;
    };
    let (engine, boot) = start().await;
    let mcp = serve(&engine, &boot, "POST", "/mcp", true).await;
    assert_eq!(send("POST", url(boot.local_addr, "/mcp")).await.status, 201);

    // Rebind to every address; nothing else (no observer) is told about it.
    let mut rebound = config("0.0.0.0");
    rebound.webhook_listener = None;
    engine.set_config(rebound.to_json()).await;
    engine
        .iii
        .call(CallRequest {
            function_id: "http::on-config-change".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .unwrap();
    let port = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(addr) = boot.current_addr().await
                && addr.ip().is_unspecified()
            {
                return addr.port();
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("rebound to 0.0.0.0");

    let public: SocketAddr = (public_ip, port).into();
    assert_absent("POST", public, "/mcp").await;
    assert_eq!(mcp.load(Ordering::SeqCst), 1);
    let loopback: SocketAddr = (IpAddr::from([127, 0, 0, 1]), port).into();
    assert_eq!(send("POST", url(loopback, "/mcp")).await.status, 201);
    assert_eq!(mcp.load(Ordering::SeqCst), 2);

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn public_webhook_with_loopback_only_is_rejected_at_registration() {
    let (engine, boot) = start().await;
    let calls = register_target(&engine, "test::both");
    let rejection = observe(
        &engine,
        "obs-both",
        "test::both",
        json!({
            "method": "POST",
            "path": "/hook",
            "public_webhook": true,
            "loopback_only": true,
        }),
    )
    .await
    .expect("contradictory observer rejected");
    assert_eq!(rejection["code"], "HTTP_ROUTE_INVALID", "{rejection}");
    let message = rejection["message"].as_str().unwrap();
    assert!(message.contains("public_webhook"), "{message}");
    assert!(message.contains("loopback_only"), "{message}");

    assert!(
        boot.routes
            .read()
            .await
            .match_route("POST", "/hook")
            .is_none()
    );
    assert!(!boot.on_request.is_observed());
    let restricted = boot.current_webhook_addr().await.unwrap();
    for addr in [boot.local_addr, restricted] {
        assert_eq!(send("POST", url(addr, "/hook")).await.status, 404);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);

    // Each option alone is accepted.
    for (id, args) in [
        (
            "obs-public",
            json!({"method": "POST", "path": "/hook", "public_webhook": true, "loopback_only": false}),
        ),
        (
            "obs-local",
            json!({"method": "POST", "path": "/local", "public_webhook": false, "loopback_only": true}),
        ),
    ] {
        assert_eq!(observe(&engine, id, "test::both", args).await, None, "{id}");
    }

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn loopback_only_does_not_open_a_separate_route_namespace() {
    let (engine, boot) = start().await;
    serve(&engine, &boot, "GET", "/items/:id", true).await;
    register_target(&engine, "test::second");
    for loopback_only in [false, true] {
        let rejection = observe(
            &engine,
            &format!("obs-second-{loopback_only}"),
            "test::second",
            json!({"method": "GET", "path": "/items/{other}", "loopback_only": loopback_only}),
        )
        .await
        .expect("same shape rejected");
        assert_eq!(rejection["code"], "HTTP_ROUTE_CONFLICT", "{rejection}");
    }

    boot.shutdown().await;
    engine.shutdown().await;
}

#[tokio::test]
async fn route_args_advertise_loopback_only() {
    let (engine, boot) = start().await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while engine.registration(ON_REQUEST_FN).is_none() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("registerfunction for http::on-request");
    let frame = engine.registration(ON_REQUEST_FN).unwrap();
    let property = &frame["args_format"]["properties"]["loopback_only"];
    assert_eq!(property["type"], "boolean", "{frame}");
    let required = frame["args_format"]["required"].as_array().unwrap();
    assert!(!required.contains(&json!("loopback_only")));
    let description = frame["description"].as_str().unwrap_or_default();
    assert!(description.contains("loopback_only"), "{description}");

    boot.shutdown().await;
    engine.shutdown().await;
}

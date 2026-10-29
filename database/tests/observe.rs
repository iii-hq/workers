//! `database::on-row-changed` end to end against a live iii engine (0.26+,
//! `observe.v2`), on SQLite.
//!
//! Gated like the postgres/mysql tests: set `III_TEST_ENGINE_URL` (for example
//! `ws://127.0.0.1:49360`) to run them; without it each test returns early.
//! The zero-wire test also listens on `III_TEST_PROXY_PORT` (default 49365).
//! The engine-restart test starts an engine itself: set `III_TEST_ENGINE_BIN`
//! to an engine binary and `III_TEST_RESTART_PORT` to a free port.
//!
//! Each test runs its database worker in-process (the library's handlers and
//! `row_changes::capture::register`, as `main.rs` wires them) and its
//! consumers as separate connections, in a namespace of its own so tests can
//! run in parallel against one engine.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use database::config::WorkerConfig;
use database::configuration;
use database::handle::HandleRegistry;
use database::handlers::{execute, AppState};
use database::row_changes::{self, NativeCapture, RowChangeBus};
use database::transaction::TxRegistry;
use iii_helpers::observability::Logger;
use iii_sdk::runtime::{FunctionRef, WorkerMetadata};
use iii_sdk::{
    register_worker, CallRequest, Capture, IIIClient, InitOptions, Observed, ObserverOptions,
    ObserverState, RegisterFunction, Source,
};
use serde_json::{json, Value};
use tokio::sync::{mpsc, RwLock};

const WAIT: Duration = Duration::from_secs(10);
const SILENCE: Duration = Duration::from_millis(600);

fn engine_url() -> Option<String> {
    std::env::var("III_TEST_ENGINE_URL")
        .ok()
        .filter(|u| !u.is_empty())
}

fn namespace(test: &str) -> String {
    format!(
        "dbobs-{test}-{}",
        &uuid::Uuid::new_v4().simple().to_string()[..8]
    )
}

fn client(url: &str, ns: &str, name: &str) -> IIIClient {
    let metadata = WorkerMetadata {
        name: name.to_string(),
        ..WorkerMetadata::default()
    };
    register_worker(
        url,
        InitOptions {
            metadata: Some(metadata),
            namespace: Some(ns.to_string()),
            ..Default::default()
        },
    )
}

async fn eventually(what: &str, mut check: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !check() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// The database worker, in-process.
struct DatabaseWorker {
    iii: Arc<IIIClient>,
    state: AppState,
    capture: Arc<NativeCapture>,
    row_changed: FunctionRef,
    dir: tempfile::TempDir,
    _listeners: Arc<row_changes::NativeListeners>,
}

impl DatabaseWorker {
    async fn start(url: &str, ns: &str, extra_dbs: &[&str]) -> Self {
        Self::start_with(url, ns, extra_dbs, tempfile::tempdir().unwrap()).await
    }

    /// A second process on the same database files.
    async fn start_on(url: &str, ns: &str, dir: tempfile::TempDir) -> Self {
        Self::start_with(url, ns, &[], dir).await
    }

    async fn start_with(url: &str, ns: &str, extra_dbs: &[&str], dir: tempfile::TempDir) -> Self {
        let mut yaml = format!(
            "databases:\n  primary:\n    url: \"sqlite:{}\"\n",
            dir.path().join("primary.db").display()
        );
        for db in extra_dbs {
            // `native:<name>`: the primary file, with native capture.
            match db.strip_prefix("native:") {
                Some(name) => yaml.push_str(&format!(
                    "  {name}:\n    url: \"sqlite:{}\"\n    capture: native\n",
                    dir.path().join("primary.db").display()
                )),
                None => yaml.push_str(&format!(
                    "  {db}:\n    url: \"sqlite:{}\"\n",
                    dir.path().join(format!("{db}.db")).display()
                )),
            }
        }
        let cfg = WorkerConfig::from_yaml(&yaml).unwrap();
        let pools = Arc::new(RwLock::new(configuration::build_pools(&cfg).await.unwrap()));
        let config = Arc::new(RwLock::new(cfg));
        let iii = Arc::new(client(url, ns, "database-test"));
        let capture = Arc::new(NativeCapture::new(
            iii.clone(),
            config.clone(),
            pools.clone(),
            10_000,
        ));
        let bus = Arc::new(RowChangeBus::with_capture(iii.clone(), capture.clone()));
        let listeners = Arc::new(row_changes::NativeListeners::new(bus.clone()));
        listeners.sync(&*config.read().await);
        let state = AppState {
            pools,
            config,
            handles: Arc::new(HandleRegistry::new()),
            transactions: TxRegistry::new(),
            log: Logger::new(),
            row_changes: Some(bus),
            runtime: Default::default(),
        };
        {
            let st = state.clone();
            iii.register_function(
                "database::execute",
                RegisterFunction::new_async(move |req: execute::ExecuteReq| {
                    let st = st.clone();
                    async move {
                        execute::handle(&st, req)
                            .await
                            .map_err(iii_sdk::errors::Error::from)
                    }
                }),
            );
        }
        let row_changed = row_changes::capture::register(&iii, &capture);
        iii.wait_until_registered(WAIT).await.unwrap();
        // The engine confirmed the function (functionregistered): its calls
        // run in-process.
        eventually("on-row-changed confirmed", || {
            row_changed.local_dispatch_active()
        })
        .await;
        Self {
            iii,
            state,
            capture,
            row_changed,
            dir,
            _listeners: listeners,
        }
    }

    async fn sql(&self, db: &str, sql: &str) -> Value {
        let req: execute::ExecuteReq =
            serde_json::from_value(json!({ "db": db, "sql": sql })).unwrap();
        serde_json::to_value(execute::handle(&self.state, req).await.unwrap()).unwrap()
    }
}

/// A consumer function that records what it receives.
fn sink(iii: &IIIClient, id: &str) -> mpsc::UnboundedReceiver<Value> {
    let (tx, rx) = mpsc::unbounded_channel();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |payload: Value| {
            let tx = tx.clone();
            async move {
                let _ = tx.send(payload);
                Ok::<(), iii_sdk::errors::Error>(())
            }
        }),
    );
    rx
}

async fn next(rx: &mut mpsc::UnboundedReceiver<Value>, what: &str) -> Value {
    tokio::time::timeout(WAIT, rx.recv())
        .await
        .unwrap_or_else(|_| panic!("{what}: nothing delivered"))
        .unwrap()
}

async fn silent(rx: &mut mpsc::UnboundedReceiver<Value>, what: &str) {
    if let Ok(Some(v)) = tokio::time::timeout(SILENCE, rx.recv()).await {
        panic!("{what}: unexpected delivery {v}");
    }
}

/// (1) A new consumer observes `database::on-row-changed` with args and gets
/// the statements path's events, canonical, with `caller.parent` naming the
/// function that made the write; routing keeps other tables out.
#[tokio::test(flavor = "multi_thread")]
async fn new_consumer_observes_the_statements_path_with_caller_parent() {
    let Some(url) = engine_url() else {
        return eprintln!("skipping: III_TEST_ENGINE_URL not set");
    };
    let ns = namespace("new");
    let db = DatabaseWorker::start(&url, &ns, &[]).await;
    db.sql(
        "primary",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, n INT)",
    )
    .await;
    db.sql(
        "primary",
        "CREATE TABLE payments (id INTEGER PRIMARY KEY, n INT)",
    )
    .await;

    let consumer = Arc::new(client(&url, &ns, "billing"));
    let mut events = sink(&consumer, "billing::on-orders");
    {
        let me = consumer.clone();
        consumer.register_function(
            "billing::close-invoice",
            RegisterFunction::new_async(move |sql: String| {
                let me = me.clone();
                async move {
                    me.call(CallRequest::new(
                        "database::execute",
                        json!({ "db": "primary", "sql": sql }),
                    ))
                    .await
                }
            }),
        );
    }
    let observer = consumer.observe(
        Source::function("database::on-row-changed")
            .args(json!({ "db": "primary", "table": "orders" }))
            .capture([Capture::Input, Capture::Caller]),
        "billing::on-orders",
    );
    observer.ready().await.expect("observer active");
    assert!(db.row_changed.is_observed());
    assert_eq!(db.row_changed.observers().len(), 1);

    consumer
        .call(CallRequest::new(
            "billing::close-invoice",
            json!("INSERT INTO Orders (n) VALUES (1) RETURNING id"),
        ))
        .await
        .unwrap();
    let got: Observed = serde_json::from_value(next(&mut events, "orders insert").await).unwrap();
    let input = got.input.expect("capture input");
    assert_eq!(input["db"], "primary");
    assert_eq!(input["table"], "orders", "canonical table in the event");
    assert_eq!(input["schema"], Value::Null);
    assert_eq!(input["op"], "insert");
    assert_eq!(input["affected_rows"], 1);
    assert_eq!(input["returning"][0]["id"], 1);
    // The call's caller is the invocation that made the write.
    let caller = got.caller.expect("capture caller");
    assert_eq!(caller.function_id.as_deref(), Some("database::execute"));
    let parent = caller.parent.expect("caller.parent on the statements path");
    assert_eq!(
        parent.function_id.as_deref(),
        Some("billing::close-invoice")
    );
    assert_eq!(parent.worker_name.as_deref(), Some("billing"));

    // Routing: an observer on `orders` never receives `payments` events.
    consumer
        .call(CallRequest::new(
            "billing::close-invoice",
            json!("INSERT INTO payments (n) VALUES (1)"),
        ))
        .await
        .unwrap();
    silent(&mut events, "payments write").await;

    observer.dispose().await.unwrap();
    // The SDK drops the observer from the set before the removed hook runs:
    // wait for the hook, not just `is_observed()`.
    eventually("observer removed on the host", || {
        !db.row_changed.is_observed() && db.capture.observer_count() == 0
    })
    .await;
    consumer.shutdown_async().await;
    db.iii.shutdown_async().await;
}

/// The native path calls the function from a listener task, outside any
/// invocation: a write by another client reaches observers with
/// `caller: { worker_name }` only, no `parent`.
#[tokio::test(flavor = "multi_thread")]
async fn native_path_events_have_no_caller_parent() {
    let Some(url) = engine_url() else {
        return eprintln!("skipping: III_TEST_ENGINE_URL not set");
    };
    let ns = namespace("native");
    let db = DatabaseWorker::start(&url, &ns, &["native:watched"]).await;
    db.sql(
        "primary",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, n INT)",
    )
    .await;
    let consumer = client(&url, &ns, "consumer");
    let mut events = sink(&consumer, "consumer::sink");
    let handle = consumer.observe(
        Source::function("database::on-row-changed")
            .args(json!({ "db": "watched", "table": "orders" }))
            .capture([Capture::Input, Capture::Caller]),
        "consumer::sink",
    );
    handle
        .ready()
        .await
        .expect("native observer active (capture installed)");

    // Another client: a plain sqlite connection on the same file.
    let path = db.dir.path().join("primary.db");
    tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute("INSERT INTO orders (n) VALUES (1)", [])
            .unwrap();
    })
    .await
    .unwrap();
    let got: Observed = serde_json::from_value(next(&mut events, "external write").await).unwrap();
    let input = got.input.unwrap();
    assert_eq!(input["db"], "watched");
    assert_eq!(input["table"], "orders");
    assert_eq!(input["returning"][0]["id"], 1);
    let caller = got.caller.unwrap();
    assert_eq!(caller.worker_name.as_deref(), Some("database-test"));
    assert!(caller.function_id.is_none(), "{caller:?}");
    assert!(caller.parent.is_none(), "{caller:?}");

    // A native observer must name a table.
    let tableless = consumer.observe(
        Source::function("database::on-row-changed").args(json!({ "db": "watched" })),
        "consumer::sink",
    );
    let err = tableless.ready().await.unwrap_err();
    assert!(
        err.to_string().contains("NATIVE_CAPTURE_REQUIRES_TABLE"),
        "{err}"
    );
    consumer.shutdown_async().await;
    db.iii.shutdown_async().await;
}

/// A write by another client on the database's file, as `native` capture
/// hears it.
async fn external_insert(path: std::path::PathBuf, n: i64) {
    tokio::task::spawn_blocking(move || {
        let conn = rusqlite::Connection::open(path).unwrap();
        conn.execute("INSERT INTO orders (n) VALUES (?1)", [n])
            .unwrap();
    })
    .await
    .unwrap();
}

/// The direct refcount test, live: two observers on one native table share
/// its capture, and one leaving does not take it from the other.
#[tokio::test(flavor = "multi_thread")]
async fn one_of_two_native_observers_leaving_keeps_the_other_hearing() {
    let Some(url) = engine_url() else {
        return eprintln!("skipping: III_TEST_ENGINE_URL not set");
    };
    let ns = namespace("refcount");
    let db = DatabaseWorker::start(&url, &ns, &["native:watched"]).await;
    db.sql(
        "primary",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, n INT)",
    )
    .await;
    let consumer = client(&url, &ns, "consumer");
    let mut first = sink(&consumer, "consumer::first");
    let mut second = sink(&consumer, "consumer::second");
    let observe = |target: &str| {
        consumer.observe(
            Source::function("database::on-row-changed")
                .args(json!({ "db": "watched", "table": "orders" }))
                .deliver(iii_sdk::Deliver::Input),
            target,
        )
    };
    let leaving = observe("consumer::first");
    let staying = observe("consumer::second");
    leaving.ready().await.unwrap();
    staying.ready().await.unwrap();
    let file = db.dir.path().join("primary.db");

    external_insert(file.clone(), 1).await;
    assert_eq!(
        next(&mut first, "first observer").await["returning"][0]["id"],
        1
    );
    assert_eq!(
        next(&mut second, "second observer").await["returning"][0]["id"],
        1
    );

    leaving.dispose().await.unwrap();
    eventually("one observer left on the host", || {
        db.capture.observer_count() == 1
    })
    .await;
    external_insert(file, 2).await;
    assert_eq!(
        next(&mut second, "the remaining observer").await["returning"][0]["id"],
        2
    );
    silent(&mut first, "the observer that left").await;
    silent(&mut second, "one event per write").await;
    consumer.shutdown_async().await;
    db.iii.shutdown_async().await;
}

/// Engine-side `args_format` and the added hook's checks reject bad args at
/// creation, so the observer's `ready()` fails with the reason.
#[tokio::test(flavor = "multi_thread")]
async fn bad_args_are_rejected_at_creation() {
    let Some(url) = engine_url() else {
        return eprintln!("skipping: III_TEST_ENGINE_URL not set");
    };
    let ns = namespace("args");
    let db = DatabaseWorker::start(&url, &ns, &[]).await;
    let consumer = client(&url, &ns, "consumer");
    let _rx = sink(&consumer, "consumer::sink");
    for (args, code) in [
        (
            json!({ "db": "primary", "tabel": "orders" }),
            "INVALID_ARGS",
        ),
        (json!({ "table": "orders" }), "INVALID_ARGS"),
        (
            json!({ "db": "primary", "table": "Orders" }),
            "NON_CANONICAL_TABLE",
        ),
        (
            json!({ "db": "primary", "table": "public.orders" }),
            "NON_CANONICAL_TABLE",
        ),
        (json!({ "db": "nope" }), "UNKNOWN_DB"),
    ] {
        let handle = consumer.observe(
            Source::function("database::on-row-changed").args(args.clone()),
            "consumer::sink",
        );
        let err = handle.ready().await.expect_err(&args.to_string());
        assert!(
            err.to_string().contains(code),
            "{args}: expected {code}, got {err}"
        );
    }
    assert!(!db.row_changed.is_observed());
    consumer.shutdown_async().await;
    db.iii.shutdown_async().await;
}

/// A config reload that removes a database late-rejects its observers.
#[tokio::test(flavor = "multi_thread")]
async fn config_reload_late_rejects_observers_on_a_removed_database() {
    let Some(url) = engine_url() else {
        return eprintln!("skipping: III_TEST_ENGINE_URL not set");
    };
    let ns = namespace("reload");
    let db = DatabaseWorker::start(&url, &ns, &["secondary"]).await;
    let consumer = client(&url, &ns, "consumer");
    let _rx = sink(&consumer, "consumer::sink");
    let keep = consumer.observe(
        Source::function("database::on-row-changed").args(json!({ "db": "primary" })),
        "consumer::sink",
    );
    let doomed = consumer.observe(
        Source::function("database::on-row-changed").args(json!({ "db": "secondary" })),
        "consumer::sink",
    );
    keep.ready().await.unwrap();
    doomed.ready().await.unwrap();

    let mut cfg = db.state.config.read().await.clone();
    cfg.databases.remove("secondary");
    configuration::apply_config(&db.state, cfg, None)
        .await
        .unwrap();

    eventually("doomed observer in error", || {
        doomed.state() == ObserverState::Error
    })
    .await;
    assert_eq!(doomed.reason().unwrap().code, "UNKNOWN_DB");
    assert_eq!(keep.state(), ObserverState::Active);
    eventually("the host dropped the rejected observer", || {
        db.capture.observer_count() == 1
    })
    .await;
    consumer.shutdown_async().await;
    db.iii.shutdown_async().await;
}

/// A transaction's statements arrive in commit order; a rollback emits nothing.
#[tokio::test(flavor = "multi_thread")]
async fn commit_order_and_rollback() {
    let Some(url) = engine_url() else {
        return eprintln!("skipping: III_TEST_ENGINE_URL not set");
    };
    let ns = namespace("tx");
    let db = DatabaseWorker::start(&url, &ns, &[]).await;
    db.sql(
        "primary",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, n INT)",
    )
    .await;
    let consumer = client(&url, &ns, "consumer");
    let mut events = sink(&consumer, "consumer::sink");
    let handle = consumer.observe_with(
        Source::function("database::on-row-changed")
            .args(json!({ "db": "primary", "table": "orders" }))
            .deliver(iii_sdk::Deliver::Input),
        "consumer::sink",
        ObserverOptions::default().max_in_flight(1),
    );
    handle.ready().await.unwrap();

    let st = &db.state;
    let begin = |_: ()| async {
        let resp = database::handlers::begin_transaction::handle(
            st,
            serde_json::from_value(json!({ "db": "primary" })).unwrap(),
        )
        .await
        .unwrap();
        serde_json::to_value(resp).unwrap()["transaction"]["id"]
            .as_str()
            .unwrap()
            .to_string()
    };
    let tx_exec = |tx: String, sql: &'static str| async move {
        database::handlers::transaction_execute::handle(
            st,
            serde_json::from_value(json!({ "transaction_id": tx, "sql": sql })).unwrap(),
        )
        .await
        .unwrap();
    };
    let tx = begin(()).await;
    tx_exec(tx.clone(), "INSERT INTO orders (n) VALUES (1)").await;
    tx_exec(tx.clone(), "UPDATE orders SET n = 2").await;
    tx_exec(tx.clone(), "DELETE FROM orders").await;
    silent(&mut events, "staged, not committed").await;
    database::handlers::commit_transaction::handle(
        st,
        serde_json::from_value(json!({ "transaction_id": tx })).unwrap(),
    )
    .await
    .unwrap();
    let ops: Vec<String> = [
        next(&mut events, "1st").await,
        next(&mut events, "2nd").await,
        next(&mut events, "3rd").await,
    ]
    .iter()
    .map(|e| e["op"].as_str().unwrap().to_string())
    .collect();
    assert_eq!(ops, ["insert", "update", "delete"]);

    let tx = begin(()).await;
    tx_exec(tx.clone(), "INSERT INTO orders (n) VALUES (3)").await;
    database::handlers::rollback_transaction::handle(
        st,
        serde_json::from_value(json!({ "transaction_id": tx })).unwrap(),
    )
    .await
    .unwrap();
    silent(&mut events, "rolled back").await;
    consumer.shutdown_async().await;
    db.iii.shutdown_async().await;
}

/// What the worker writes to the engine, frame by frame: a WebSocket proxy
/// that decodes the client → engine direction (masked frames) and records
/// each data frame's `type`.
#[derive(Clone, Default)]
struct WireLog(Arc<Mutex<Vec<String>>>);

impl WireLog {
    fn snapshot(&self) -> usize {
        self.0.lock().unwrap().len()
    }
    fn since(&self, mark: usize) -> HashMap<String, usize> {
        let mut out = HashMap::new();
        for t in &self.0.lock().unwrap()[mark..] {
            *out.entry(t.clone()).or_default() += 1;
        }
        out
    }
}

/// Parses complete client frames off `buf`, recording data frames' JSON type.
fn drain_frames(buf: &mut Vec<u8>, log: &WireLog) {
    loop {
        if buf.len() < 2 {
            return;
        }
        let opcode = buf[0] & 0x0f;
        let masked = buf[1] & 0x80 != 0;
        let (len, mut idx) = match buf[1] & 0x7f {
            126 if buf.len() >= 4 => (u16::from_be_bytes([buf[2], buf[3]]) as usize, 4),
            127 if buf.len() >= 10 => (
                u64::from_be_bytes(buf[2..10].try_into().unwrap()) as usize,
                10,
            ),
            126 | 127 => return,
            n => (n as usize, 2),
        };
        let key = if masked {
            if buf.len() < idx + 4 {
                return;
            }
            let k = [buf[idx], buf[idx + 1], buf[idx + 2], buf[idx + 3]];
            idx += 4;
            Some(k)
        } else {
            None
        };
        if buf.len() < idx + len {
            return;
        }
        let mut payload = buf[idx..idx + len].to_vec();
        if let Some(k) = key {
            for (i, b) in payload.iter_mut().enumerate() {
                *b ^= k[i % 4];
            }
        }
        let kind = match opcode {
            0x1 | 0x2 => serde_json::from_slice::<Value>(&payload)
                .ok()
                .and_then(|v| v.get("type").and_then(Value::as_str).map(str::to_string))
                .unwrap_or_else(|| "data".to_string()),
            0x0 => "continuation".to_string(),
            other => format!("control:{other}"),
        };
        log.0.lock().unwrap().push(kind);
        buf.drain(..idx + len);
    }
}

async fn wire_proxy(listen: u16, upstream: String) -> WireLog {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let log = WireLog::default();
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", listen))
        .await
        .unwrap();
    let out = log.clone();
    tokio::spawn(async move {
        while let Ok((client, _)) = listener.accept().await {
            let Ok(server) = tokio::net::TcpStream::connect(&upstream).await else {
                continue;
            };
            let (mut cr, mut cw) = client.into_split();
            let (mut sr, mut sw) = server.into_split();
            tokio::spawn(async move { tokio::io::copy(&mut sr, &mut cw).await });
            let log = out.clone();
            tokio::spawn(async move {
                // The HTTP upgrade request precedes the frames. Only the main
                // socket (`/`) is counted: the SDK's OpenTelemetry exporter
                // also connects through this port (`/otel`) and sends binary
                // OTLP batches whenever its interval comes round.
                let mut handshake_done = false;
                let mut main_socket = false;
                let mut buf = Vec::new();
                let mut chunk = vec![0u8; 64 * 1024];
                loop {
                    let n = match cr.read(&mut chunk).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => n,
                    };
                    if sw.write_all(&chunk[..n]).await.is_err() {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if !handshake_done {
                        let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") else {
                            continue;
                        };
                        let path = String::from_utf8_lossy(&buf[..end])
                            .split_whitespace()
                            .nth(1)
                            .unwrap_or_default()
                            .to_string();
                        main_socket = path == "/" || path.starts_with("/?");
                        buf.drain(..end + 4);
                        handshake_done = true;
                    }
                    if main_socket {
                        drain_frames(&mut buf, &log);
                    } else {
                        buf.clear();
                    }
                }
            });
        }
    });
    log
}

fn data_frames(counts: &HashMap<String, usize>) -> usize {
    counts
        .iter()
        .filter(|(k, _)| !k.starts_with("control:") && k.as_str() != "localinvocationstats")
        .map(|(_, n)| n)
        .sum()
}

/// (2) Unobserved writes put zero frames on the wire: `is_observed()` is
/// false, nothing is serialized, called or sent. An observer of another
/// table sends nothing either (the call's keys match no observer). Observed
/// writes send batched reports; a filter miss sends nothing; after dispose it
/// is zero again.
#[tokio::test(flavor = "multi_thread")]
async fn unobserved_writes_put_zero_frames_on_the_wire() {
    let Some(url) = engine_url() else {
        return eprintln!("skipping: III_TEST_ENGINE_URL not set");
    };
    let port: u16 = std::env::var("III_TEST_PROXY_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(49365);
    let upstream = url
        .trim_start_matches("ws://")
        .trim_end_matches('/')
        .to_string();
    let wire = wire_proxy(port, upstream).await;
    let ns = namespace("wire");
    let db = DatabaseWorker::start(&format!("ws://127.0.0.1:{port}"), &ns, &[]).await;
    db.sql(
        "primary",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, n INT)",
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await; // startup traffic settles

    const WRITES: usize = 1000;
    let writes = |n: usize| {
        let db = &db;
        async move {
            for i in 0..n {
                db.sql(
                    "primary",
                    &format!("INSERT INTO orders (n) VALUES ({i}) RETURNING id"),
                )
                .await;
            }
        }
    };

    // Nobody observes.
    assert!(!db.row_changed.is_observed());
    let mark = wire.snapshot();
    writes(WRITES).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let unobserved = wire.since(mark);
    eprintln!("unobserved: {WRITES} writes -> {unobserved:?}");
    assert_eq!(data_frames(&unobserved), 0, "{unobserved:?}");

    // Observed on another table: the call's keys match no observer, so the
    // function is not called.
    let consumer = client(&url, &ns, "consumer");
    let mut events = sink(&consumer, "consumer::sink");
    let elsewhere = consumer.observe(
        Source::function("database::on-row-changed")
            .args(json!({ "db": "primary", "table": "payments" })),
        "consumer::sink",
    );
    elsewhere.ready().await.unwrap();
    eventually("host sees the observer", || db.row_changed.is_observed()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mark = wire.snapshot();
    writes(100).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let other_table = wire.since(mark);
    eprintln!("observed on another table: 100 writes -> {other_table:?}");
    assert_eq!(data_frames(&other_table), 0, "{other_table:?}");
    silent(&mut events, "another table").await;
    elsewhere.dispose().await.unwrap();

    // Observed, but the observer's filter matches nothing: still no report.
    let miss = consumer.observe(
        Source::function("database::on-row-changed")
            .args(json!({ "db": "primary", "table": "orders" }))
            .filters(json!({ "input.affected_rows": { "$gt": 1000 } })),
        "consumer::sink",
    );
    miss.ready().await.unwrap();
    eventually("host sees the observer", || db.row_changed.is_observed()).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mark = wire.snapshot();
    writes(100).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let missed = wire.since(mark);
    eprintln!("observed, filter miss: 100 writes -> {missed:?}");
    assert_eq!(data_frames(&missed), 0, "{missed:?}");
    silent(&mut events, "filter miss").await;
    miss.dispose().await.unwrap();

    // Observed and matching: reports go out, batched.
    let hit = consumer.observe(
        Source::function("database::on-row-changed")
            .args(json!({ "db": "primary", "table": "orders" })),
        "consumer::sink",
    );
    hit.ready().await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mark = wire.snapshot();
    writes(100).await;
    for i in 0..100 {
        next(&mut events, &format!("matching write {i}")).await;
    }
    let matched = wire.since(mark);
    eprintln!("observed, matching: 100 writes -> {matched:?}");
    let reports = matched.get("invocationsobserved").copied().unwrap_or(0);
    assert!((1..=100).contains(&reports), "{matched:?}");

    // Disposed: back to zero.
    hit.dispose().await.unwrap();
    eventually("observer gone on the host", || {
        !db.row_changed.is_observed()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let mark = wire.snapshot();
    writes(WRITES).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after = wire.since(mark);
    eprintln!("after dispose: {WRITES} writes -> {after:?}");
    assert_eq!(data_frames(&after), 0, "{after:?}");
    consumer.shutdown_async().await;
    db.iii.shutdown_async().await;
}

/// The host restarts (a deploy): its observers wait and are replayed to the
/// new connection, whose added hooks run for each of them again.
#[tokio::test(flavor = "multi_thread")]
async fn host_restart_replays_observers() {
    let Some(url) = engine_url() else {
        return eprintln!("skipping: III_TEST_ENGINE_URL not set");
    };
    let ns = namespace("restart");
    let first = DatabaseWorker::start(&url, &ns, &[]).await;
    first
        .sql(
            "primary",
            "CREATE TABLE orders (id INTEGER PRIMARY KEY, n INT)",
        )
        .await;
    let consumer = client(&url, &ns, "consumer");
    let mut orders = sink(&consumer, "consumer::orders");
    let mut inserts = sink(&consumer, "consumer::inserts");
    let handle = consumer.observe(
        Source::function("database::on-row-changed")
            .args(json!({ "db": "primary", "table": "orders" }))
            .deliver(iii_sdk::Deliver::Input),
        "consumer::orders",
    );
    let insert_only = consumer.observe(
        Source::function("database::on-row-changed")
            .args(json!({ "db": "primary", "ops": ["insert"] }))
            .deliver(iii_sdk::Deliver::Input),
        "consumer::inserts",
    );
    handle.ready().await.unwrap();
    insert_only.ready().await.unwrap();
    eventually("both observers on the first host", || {
        first.capture.observer_count() == 2
    })
    .await;

    // The host goes away; a new process with the same database comes up.
    let dir = first.dir;
    first.iii.shutdown_async().await;
    eventually("observer suspended or pending", || {
        handle.state() != ObserverState::Active
    })
    .await;
    let second = DatabaseWorker::start_on(&url, &ns, dir).await;
    eventually("both observers back on the new host", || {
        second.capture.observer_count() == 2
    })
    .await;
    handle.ready().await.unwrap();
    insert_only.ready().await.unwrap();
    second
        .sql("primary", "INSERT INTO orders (n) VALUES (1)")
        .await;
    assert_eq!(
        next(&mut orders, "table observer after restart").await["table"],
        "orders"
    );
    assert_eq!(
        next(&mut inserts, "ops observer after restart").await["op"],
        "insert"
    );
    second.sql("primary", "DELETE FROM orders").await;
    assert_eq!(next(&mut orders, "delete").await["op"], "delete");
    silent(&mut inserts, "a delete on an insert-only observer").await;
    consumer.shutdown_async().await;
    second.iii.shutdown_async().await;
}

/// The function is private: a worker that calls it with a forged event is
/// refused by the engine, and no observer hears it.
#[tokio::test(flavor = "multi_thread")]
async fn invoking_the_source_directly_is_refused() {
    let Some(url) = engine_url() else {
        return eprintln!("skipping: III_TEST_ENGINE_URL not set");
    };
    let ns = namespace("forged");
    let _db = DatabaseWorker::start(&url, &ns, &[]).await;
    let consumer = client(&url, &ns, "consumer");
    let mut events = sink(&consumer, "consumer::sink");
    let handle = consumer.observe(
        Source::function("database::on-row-changed")
            .args(json!({ "db": "primary", "table": "orders" })),
        "consumer::sink",
    );
    handle.ready().await.unwrap();
    let attacker = client(&url, &ns, "attacker");
    let forged = json!({
        "db": "primary", "schema": null, "table": "orders", "op": "delete",
        "affected_rows": 999, "at": 1
    });
    let err = attacker
        .call(CallRequest::new("database::on-row-changed", forged))
        .await
        .expect_err("a call from another worker is refused");
    assert!(err.to_string().contains("FORBIDDEN"), "{err}");
    silent(&mut events, "forged event").await;
    attacker.shutdown_async().await;
    consumer.shutdown_async().await;
}

/// An engine binary run by a test on a port of its own, in a scratch
/// directory (its configuration store and its log).
struct Engine {
    child: std::process::Child,
    dir: tempfile::TempDir,
}

impl Engine {
    async fn start(bin: &str, port: u16) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("engine.yaml");
        std::fs::write(
            &config,
            format!(
                "workers:\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        \
                 config:\n          directory: {}\n  - name: iii-worker-manager\n    config:\n      \
                 port: {port}\n",
                dir.path().join("configuration").display()
            ),
        )
        .unwrap();
        let log = std::fs::File::create(dir.path().join("engine.log")).unwrap();
        let child = std::process::Command::new(bin)
            .arg("--no-update-check")
            .arg("-c")
            .arg(&config)
            .current_dir(dir.path())
            .env("III_TELEMETRY_ENABLED", "false")
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .spawn()
            .unwrap_or_else(|e| panic!("starting {bin}: {e}"));
        let engine = Self { child, dir };
        let deadline = tokio::time::Instant::now() + WAIT;
        while tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err()
        {
            assert!(
                tokio::time::Instant::now() < deadline,
                "{bin} did not listen on {port}: {}",
                engine.log()
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        engine
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.dir.path().join("engine.log")).unwrap_or_default()
    }

    /// Stops the engine as an operator would (SIGINT), killing it if it
    /// takes too long.
    async fn stop(mut self) {
        let _ = std::process::Command::new("kill")
            .arg("-INT")
            .arg(self.child.id().to_string())
            .status();
        for _ in 0..100 {
            if self.child.try_wait().unwrap().is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// A sink that also keeps every payload it receives, for counts over a run.
fn logged_sink(
    iii: &IIIClient,
    id: &str,
) -> (mpsc::UnboundedReceiver<Value>, Arc<Mutex<Vec<Value>>>) {
    let (tx, rx) = mpsc::unbounded_channel();
    let log = Arc::new(Mutex::new(Vec::new()));
    let kept = log.clone();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |payload: Value| {
            kept.lock().unwrap().push(payload.clone());
            let _ = tx.send(payload);
            async { Ok::<(), iii_sdk::errors::Error>(()) }
        }),
    );
    (rx, log)
}

/// The first event whose `returning[0].n` is `n` within `within`; other
/// events (late probes) are skipped.
async fn heard_n(rx: &mut mpsc::UnboundedReceiver<Value>, n: i64, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while let Ok(Some(event)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        if event["returning"][0]["n"] == n {
            return true;
        }
    }
    false
}

/// How many events with `returning[0].n == n` arrive: waits for the first
/// up to `WAIT` when one is expected, then for the silence after it.
async fn deliveries_of(rx: &mut mpsc::UnboundedReceiver<Value>, n: i64, expected: bool) -> usize {
    let mut count = 0;
    let first = if expected { WAIT } else { SILENCE };
    while let Ok(Some(event)) =
        tokio::time::timeout(if count == 0 { first } else { SILENCE }, rx.recv()).await
    {
        if event["returning"][0]["n"] == n {
            count += 1;
        }
    }
    count
}

/// The engine restarts under a running worker. The worker registers the
/// function again and the consumer's SDK replays its observer to the new
/// engine, which hands it to the host: the added hook runs for it once more.
/// Each write after a restart is heard exactly once, and the host holds one
/// observer, not one per engine it has seen.
#[tokio::test(flavor = "multi_thread")]
async fn observers_follow_the_engine_across_restarts() {
    let (Some(bin), Some(port)) = (
        std::env::var("III_TEST_ENGINE_BIN").ok(),
        std::env::var("III_TEST_RESTART_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok()),
    ) else {
        return eprintln!("skipping: III_TEST_ENGINE_BIN / III_TEST_RESTART_PORT not set");
    };
    // Reconnects back off: give each restart more time than one delivery.
    const SETTLE: Duration = Duration::from_secs(30);

    let url = format!("ws://127.0.0.1:{port}");
    let ns = namespace("engine-restart");
    let mut engine = Engine::start(&bin, port).await;
    let db = DatabaseWorker::start(&url, &ns, &[]).await;
    db.sql(
        "primary",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, n INT)",
    )
    .await;

    let consumer = client(&url, &ns, "consumer");
    let (mut events, log) = logged_sink(&consumer, "consumer::orders");
    let observer = consumer.observe(
        Source::function("database::on-row-changed")
            .args(json!({ "db": "primary", "table": "orders" }))
            .deliver(iii_sdk::Deliver::Input),
        "consumer::orders",
    );
    observer.ready().await.unwrap();

    let mut probe: i64 = 0;
    for (step, n) in [
        ("first engine", 1i64),
        ("restarted", 2),
        ("restarted again", 3),
    ] {
        if n > 1 {
            engine.stop().await;
            engine = Engine::start(&bin, port).await;
        }
        let deadline = tokio::time::Instant::now() + SETTLE;
        while observer.state() != ObserverState::Active || !db.row_changed.is_observed() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "{step}: the worker and the observer never settled"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // The consumer reconnects on its own schedule: write probes until it
        // hears one.
        loop {
            probe -= 1;
            db.sql(
                "primary",
                &format!("INSERT INTO orders (n) VALUES ({probe}) RETURNING n"),
            )
            .await;
            if heard_n(&mut events, probe, Duration::from_secs(1)).await {
                break;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{step}: the consumer never heard a probe"
            );
        }
        assert_eq!(
            db.capture.observer_count(),
            1,
            "{step}: one observer on the host"
        );

        db.sql(
            "primary",
            &format!("INSERT INTO orders (n) VALUES ({n}) RETURNING n"),
        )
        .await;
        assert_eq!(
            deliveries_of(&mut events, n, true).await,
            1,
            "{step}: the consumer hears the write exactly once"
        );
    }
    // Over the whole run, late deliveries included.
    tokio::time::sleep(SILENCE).await;
    for n in 1..=3 {
        let heard = log
            .lock()
            .unwrap()
            .iter()
            .filter(|event| event["returning"][0]["n"] == n)
            .count();
        assert_eq!(heard, 1, "write {n}");
    }
    consumer.shutdown_async().await;
    db.iii.shutdown_async().await;
    engine.stop().await;
}

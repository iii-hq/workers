//! `database::on-row-changed` end to end against a live iii engine (0.25+),
//! on SQLite.
//!
//! Gated like the postgres/mysql tests: set `III_TEST_ENGINE_URL` (for example
//! `ws://127.0.0.1:49360`) to run them; without it each test returns early.
//! The zero-wire test also listens on `III_TEST_PROXY_PORT` (default 49365).
//!
//! Each test runs its database worker in-process (the library's handlers and
//! `triggers::capture::register`, as `main.rs` wires them) and its consumers
//! as separate connections, in a namespace of its own so tests can run in
//! parallel against one engine.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use database::config::WorkerConfig;
use database::configuration;
use database::handle::HandleRegistry;
use database::handlers::{execute, AppState};
use database::transaction::TxRegistry;
use database::triggers::{self, NativeCapture, RowChangeBus};
use iii_helpers::observability::Logger;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::runtime::{FunctionRef, WorkerMetadata};
use iii_sdk::{
    register_worker, Capture, IIIClient, InitOptions, Observed, ObserverOptions, ObserverState,
    RegisterFunction, Source,
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
    let mut metadata = WorkerMetadata::default();
    metadata.name = name.to_string();
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
    _listeners: Arc<triggers::NativeListeners>,
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
        let listeners = Arc::new(triggers::NativeListeners::new(bus.clone()));
        listeners.sync(&*config.read().await);
        let state = AppState {
            pools,
            config,
            handles: Arc::new(HandleRegistry::new()),
            transactions: TxRegistry::new(),
            log: Logger::new(),
            row_changes: Some(bus),
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
        let row_changed = triggers::capture::register(&iii, &capture);
        iii.wait_until_registered(WAIT).await.unwrap();
        // The engine confirmed the function (functionregistered): local dispatch on.
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

/// A sink that also records the invocation's metadata sidecar.
fn sink_with_metadata(iii: &IIIClient, id: &str) -> mpsc::UnboundedReceiver<Value> {
    let (tx, rx) = mpsc::unbounded_channel();
    iii.register_function(
        id,
        RegisterFunction::new_async(move |payload: Value| {
            let tx = tx.clone();
            let metadata = iii_sdk::current_invocation().and_then(|c| c.metadata);
            async move {
                let _ = tx.send(json!({ "payload": payload, "metadata": metadata }));
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
                    me.trigger(TriggerRequest::new(
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
        .trigger(TriggerRequest::new(
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
        .trigger(TriggerRequest::new(
            "billing::close-invoice",
            json!("INSERT INTO payments (n) VALUES (1)"),
        ))
        .await
        .unwrap();
    silent(&mut events, "payments write").await;

    observer.dispose().await.unwrap();
    eventually("observer removed on the owner", || {
        !db.row_changed.is_observed()
    })
    .await;
    assert_eq!(db.capture.route_count(), 0);
    consumer.shutdown_async().await;
    db.iii.shutdown_async().await;
}

/// The native path has no invocation context: a write by another client
/// reaches observers with `caller: { worker_name }` only, no `parent`.
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

/// (2) An old consumer's `register_trigger({ type: 'database::row-changed' })`
/// is served by the engine's legacy shim: an observer with `legacy` set, its
/// non-canonical table still matching, the event in its pre-0.25 shape.
#[tokio::test(flavor = "multi_thread")]
async fn old_consumer_register_trigger_is_served_through_the_shim() {
    let Some(url) = engine_url() else {
        return eprintln!("skipping: III_TEST_ENGINE_URL not set");
    };
    let ns = namespace("legacy");
    let db = DatabaseWorker::start(&url, &ns, &[]).await;
    db.sql(
        "primary",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, n INT)",
    )
    .await;

    let old = client(&url, &ns, "old-consumer");
    let mut events = sink_with_metadata(&old, "legacy::on-orders");
    let mut input = RegisterTriggerInput::new(
        "database::row-changed",
        "legacy::on-orders",
        json!({ "db": "primary", "table": "public.Orders", "ops": ["insert"] }),
    );
    input.metadata = Some(json!({ "__binding": "sub_abc" }));
    #[allow(deprecated)]
    let trigger = old.register_trigger(input).unwrap();
    eventually("the shim observer is active on the owner", || {
        db.row_changed.observers().iter().any(|o| {
            o.legacy()
                .is_some_and(|l| l.trigger_type == "database::row-changed")
        })
    })
    .await;
    let obs = db.row_changed.observers().pop().unwrap();
    assert_eq!(obs.legacy().unwrap().config["table"], "public.Orders");
    assert_eq!(obs.target(), "legacy::on-orders");

    db.sql("primary", "INSERT INTO orders (n) VALUES (7)").await;
    let delivery = next(&mut events, "legacy delivery").await;
    // The trigger's metadata rides along as the sidecar (a harness binding's
    // `__binding` pointer).
    assert_eq!(delivery["metadata"]["__binding"], "sub_abc", "{delivery}");
    let event = delivery["payload"].clone();
    // `deliver: input`: the event itself, in the pre-0.25 shape.
    assert_eq!(event["table"], "orders");
    assert_eq!(event["op"], "insert");
    assert!(
        event.get("schema").is_none(),
        "legacy shape has no schema: {event}"
    );
    assert!(
        event.get("observer_id").is_none(),
        "not an envelope: {event}"
    );
    // `ops: ["insert"]` still restricts.
    db.sql("primary", "DELETE FROM orders").await;
    silent(&mut events, "delete on an insert-only binding").await;

    trigger.unregister();
    eventually("unregister removes the observer", || {
        db.row_changed.observers().is_empty()
    })
    .await;
    old.shutdown_async().await;
    db.iii.shutdown_async().await;
}

/// Engine-side `args_format` and owner-side checks reject bad args at
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
    eventually("owner dropped the rejected observer", || {
        db.capture.route_count() == 1
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
                // The HTTP upgrade request precedes the frames.
                let mut handshake_done = false;
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
                        buf.drain(..end + 4);
                        handshake_done = true;
                    }
                    drain_frames(&mut buf, &log);
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

/// (3) Unobserved writes put zero frames on the wire: `is_observed()` is
/// false, nothing is serialized or sent. Observed writes send batched
/// reports; a filter miss sends nothing; after dispose it is zero again.
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

    // Observed, but the observer's filter matches nothing: still no report.
    let consumer = client(&url, &ns, "consumer");
    let mut events = sink(&consumer, "consumer::sink");
    let miss = consumer.observe(
        Source::function("database::on-row-changed")
            .args(json!({ "db": "primary", "table": "orders" }))
            .filters(json!({ "input.affected_rows": { "$gt": 1000 } })),
        "consumer::sink",
    );
    miss.ready().await.unwrap();
    eventually("owner sees the observer", || db.row_changed.is_observed()).await;
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
    assert!(reports >= 1 && reports <= 100, "{matched:?}");

    // Disposed: back to zero.
    hit.dispose().await.unwrap();
    eventually("observer gone on the owner", || {
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

/// The owner restarts (a deploy): its observers, new and legacy, wait and are
/// replayed to the new connection, which indexes them again.
#[tokio::test(flavor = "multi_thread")]
async fn owner_restart_replays_new_and_legacy_observers() {
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
    let mut fresh = sink(&consumer, "consumer::fresh");
    let mut legacy = sink(&consumer, "consumer::legacy");
    let handle = consumer.observe(
        Source::function("database::on-row-changed")
            .args(json!({ "db": "primary", "table": "orders" }))
            .deliver(iii_sdk::Deliver::Input),
        "consumer::fresh",
    );
    handle.ready().await.unwrap();
    #[allow(deprecated)]
    let _trigger = consumer
        .register_trigger(RegisterTriggerInput::new(
            "database::row-changed",
            "consumer::legacy",
            json!({ "db": "primary", "table": "orders" }),
        ))
        .unwrap();
    eventually("both observers on the first owner", || {
        first.capture.route_count() == 2
    })
    .await;

    // The owner goes away; a new process with the same database comes up.
    let dir = first.dir;
    first.iii.shutdown_async().await;
    eventually("observer suspended or pending", || {
        handle.state() != ObserverState::Active
    })
    .await;
    let second = DatabaseWorker::start_on(&url, &ns, dir).await;
    eventually("both observers replayed to the new owner", || {
        second.capture.route_count() == 2
    })
    .await;
    handle.ready().await.unwrap();
    second
        .sql("primary", "INSERT INTO orders (n) VALUES (1)")
        .await;
    assert_eq!(
        next(&mut fresh, "new observer after restart").await["table"],
        "orders"
    );
    let old = next(&mut legacy, "legacy observer after restart").await;
    assert!(old.get("schema").is_none(), "{old}");
    consumer.shutdown_async().await;
    second.iii.shutdown_async().await;
}

/// Events are routed in-process only: a worker that invokes the function
/// directly with a forged event is refused, and no observer hears it.
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
        .trigger(TriggerRequest::new("database::on-row-changed", forged))
        .await
        .expect_err("a direct invocation is refused");
    assert!(err.to_string().contains("FORBIDDEN"), "{err}");
    silent(&mut events, "forged event").await;
    attacker.shutdown_async().await;
    consumer.shutdown_async().await;
}

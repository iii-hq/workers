//! `database::on-row-changed` end to end against a live iii engine (0.25+),
//! on SQLite.
//!
//! Gated like the postgres/mysql tests: set `III_TEST_ENGINE_URL` (for example
//! `ws://127.0.0.1:49360`) to run them; without it each test returns early.
//! The zero-wire test also listens on `III_TEST_PROXY_PORT` (default 49365).
//! The engine-switch test starts engines itself: set `III_TEST_ENGINE_BINS`
//! to `<engine without observe>,<engine with observe>` (0.24 and 0.25 binaries)
//! and `III_TEST_SWITCH_PORT` to a free port.
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
use database::triggers::legacy::Fallback;
use database::triggers::{self, NativeCapture, RowChangeBus};
use iii_helpers::observability::Logger;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::runtime::{FunctionRef, WorkerMetadata};
use iii_sdk::{
    register_worker, Capture, IIIClient, InitOptions, ObserveSupportListener, Observed,
    ObserverOptions, ObserverState, RegisterFunction, Source,
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
    fallback: Arc<Fallback>,
    dir: tempfile::TempDir,
    _listeners: Arc<triggers::NativeListeners>,
    _follower: ObserveSupportListener,
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
        let row_changed = triggers::capture::register(&iii, &capture);
        // As `main.rs` does: the old trigger type while the engine lacks observe.
        let fallback = Arc::new(Fallback::new((*iii).clone(), capture.clone()));
        let follower = {
            let fallback = fallback.clone();
            iii.on_engine_supports_observe(move |observes| fallback.follow(observes))
        };
        iii.wait_until_registered(WAIT).await.unwrap();
        eventually("the engine's observe support known", || {
            iii.engine_supports_observe().is_some()
        })
        .await;
        if iii.engine_supports_observe() == Some(true) {
            // The engine confirmed the function (functionregistered): local dispatch on.
            eventually("on-row-changed confirmed", || {
                row_changed.local_dispatch_active()
            })
            .await;
        }
        Self {
            iii,
            state,
            capture,
            row_changed,
            fallback,
            dir,
            _listeners: listeners,
            _follower: follower,
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
    // The SDK drops the observer from the set before the removal hook runs:
    // wait for the hook, not just `is_observed()`.
    eventually("observer removed on the owner", || {
        !db.row_changed.is_observed() && db.capture.route_count() == 0
    })
    .await;
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
    eventually("one observer left on the owner", || {
        db.capture.route_count() == 1
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
    assert!((1..=100).contains(&reports), "{matched:?}");

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

/// The engine changes under a running worker: upgraded to one with observe,
/// rolled back, upgraded again. The worker registers its old trigger type
/// only while the engine lacks observe; an old consumer's binding hears each
/// write exactly once on every engine (the fallback, then the engine's
/// shim), and a new consumer's observer hears them whenever the engine has
/// observe.
#[tokio::test(flavor = "multi_thread")]
async fn the_fallback_follows_the_engine_across_upgrades_and_rollbacks() {
    let (Some(bins), Some(port)) = (
        std::env::var("III_TEST_ENGINE_BINS").ok(),
        std::env::var("III_TEST_SWITCH_PORT")
            .ok()
            .and_then(|p| p.parse::<u16>().ok()),
    ) else {
        return eprintln!("skipping: III_TEST_ENGINE_BINS / III_TEST_SWITCH_PORT not set");
    };
    let (without_observe, with_observe) = bins
        .split_once(',')
        .expect("III_TEST_ENGINE_BINS=<engine without observe>,<engine with observe>");
    // Reconnects back off: give each switch more time than one delivery.
    const SWITCH: Duration = Duration::from_secs(30);
    async fn settled(what: &str, mut check: impl FnMut() -> bool) {
        let deadline = tokio::time::Instant::now() + SWITCH;
        while !check() {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for {what}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    let url = format!("ws://127.0.0.1:{port}");
    let ns = namespace("switch");
    let mut engine = Engine::start(without_observe, port).await;
    let db = DatabaseWorker::start(&url, &ns, &[]).await;
    db.sql(
        "primary",
        "CREATE TABLE orders (id INTEGER PRIMARY KEY, n INT)",
    )
    .await;
    settled("the fallback on an engine without observe", || {
        db.fallback.is_registered()
    })
    .await;

    let old = client(&url, &ns, "old-consumer");
    let mut legacy = sink(&old, "legacy::on-orders");
    #[allow(deprecated)]
    let _binding = old
        .register_trigger(RegisterTriggerInput::new(
            "database::row-changed",
            "legacy::on-orders",
            json!({ "db": "primary", "table": "orders" }),
        ))
        .unwrap();
    let new = client(&url, &ns, "new-consumer");
    let mut fresh = sink(&new, "fresh::on-orders");
    let observer = new.observe(
        Source::function("database::on-row-changed")
            .args(json!({ "db": "primary", "table": "orders" }))
            .deliver(iii_sdk::Deliver::Input),
        "fresh::on-orders",
    );
    let err = observer
        .ready()
        .await
        .expect_err("no observe on this engine");
    assert!(err.to_string().contains("UNSUPPORTED"), "{err}");
    settled("the binding on the fallback", || {
        db.capture.route_count() == 1
    })
    .await;

    let mut n = 0;
    for step in [
        "0.24",
        "upgraded to 0.25",
        "rolled back to 0.24",
        "upgraded again",
    ] {
        if n > 0 {
            engine.stop().await;
            let observes = n % 2 == 1;
            engine = Engine::start(
                if observes {
                    with_observe
                } else {
                    without_observe
                },
                port,
            )
            .await;
            settled(&format!("{step}: the worker follows"), || {
                db.iii.engine_supports_observe() == Some(observes)
                    && db.fallback.is_registered() != observes
            })
            .await;
            if observes {
                // The binding moved to the shim, the observer was replayed,
                // and the fallback's route is gone: two observers filed.
                settled(&format!("{step}: the observer is replayed"), || {
                    observer.state() == ObserverState::Active
                })
                .await;
                settled(&format!("{step}: the shim serves the binding"), || {
                    let observers = db.row_changed.observers();
                    observers.iter().any(|o| o.legacy().is_some())
                        && observers.iter().any(|o| o.legacy().is_none())
                        && db.capture.route_count() == 2
                })
                .await;
            } else {
                settled(&format!("{step}: the fallback serves the binding"), || {
                    db.capture.is_listened()
                })
                .await;
            }
            // The consumers' own replays land too.
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        n += 1;
        db.sql("primary", &format!("INSERT INTO orders (n) VALUES ({n})"))
            .await;
        let heard = next(&mut legacy, &format!("{step}: old consumer")).await;
        assert_eq!(heard["table"], "orders", "{step}: {heard}");
        assert!(
            heard.get("schema").is_none(),
            "{step}: legacy shape {heard}"
        );
        silent(&mut legacy, &format!("{step}: exactly once")).await;
        if n % 2 == 0 {
            let event = next(&mut fresh, &format!("{step}: new consumer")).await;
            assert_eq!(event["affected_rows"], 1, "{step}: {event}");
        }
        silent(&mut fresh, &format!("{step}: new consumer, nothing more")).await;
    }
    old.shutdown_async().await;
    new.shutdown_async().await;
    db.iii.shutdown_async().await;
    engine.stop().await;
}

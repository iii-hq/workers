//! Guest observer hooks over a real bus: a guest registers a function and
//! sets `onObserverAdded` / `onObserverRemoved` on it, another worker
//! observes it, and the production `IIIEngine` (node_bus.rs) carries each
//! observer to the guest's hooks and the guest's calls back through it.
//!
//! Asserted: the added hook sees `{ id, function_id, args, filters, target }`;
//! a hook that throws an error with a `code` rejects the observer with that
//! code; `iii.call({ observer, payload })` reaches the observer's target,
//! whose metadata argument is the observer's, and resolves to its result; a
//! call through an observer that names metadata is refused before anything
//! is sent, and `metadata: null` is no metadata on every call; the removed
//! hook runs on dispose, and never for the rejected observer. A guest's
//! observe the engine refuses (another connection holds its id) is that
//! observer's final answer: its dispose resolves at once, and the guest's
//! next observe under the id is the one calls reach.
//!
//! The engine is `III_ENGINE_WS_URL` when set; otherwise `III_ENGINE_BIN`
//! (or an `iii` 0.26+ on PATH) is spawned with an engine-only config on
//! `III_TEST_ENGINE_PORT` or an ephemeral port. Without one the test skips;
//! `III_E2E_REQUIRE=1` turns the skip into a failure.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use code_runner::config::CodeRunnerConfig;
use code_runner::node_bus::IIIEngine;
use iii_node_core::manager::RuntimeManager;
use iii_node_core::wire::run::RunRequest;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, IIIClient, InitOptions, ObserverOptions, RegisterFunction, Source,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::{sleep, timeout};

struct Engine {
    url: String,
    child: Option<Child>,
    dir: Option<PathBuf>,
}

impl Drop for Engine {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
        if let Some(dir) = &self.dir {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

/// `iii` on PATH, when it reports 0.26 or later.
fn engine_on_path() -> Option<PathBuf> {
    let out = Command::new("iii").arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let mut parts = text
        .split_whitespace()
        .last()?
        .split('.')
        .map(|p| p.parse::<u64>().ok());
    let (major, minor) = (parts.next()??, parts.next()??);
    (major > 0 || minor >= 26).then(|| PathBuf::from("iii"))
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .expect("bind an ephemeral port")
}

async fn engine() -> Option<Engine> {
    if let Ok(url) = std::env::var("III_ENGINE_WS_URL") {
        return Some(Engine {
            url,
            child: None,
            dir: None,
        });
    }
    let bin = std::env::var_os("III_ENGINE_BIN")
        .map(PathBuf::from)
        .or_else(engine_on_path)?;
    let port = std::env::var("III_TEST_ENGINE_PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or_else(free_port);
    let dir = std::env::temp_dir().join(format!(
        "code-runner-guest-observers-{}-{port}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).expect("create engine dir");
    let config = dir.join("config.yaml");
    std::fs::write(
        &config,
        format!(
            "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n  - name: configuration\n    config:\n      adapter:\n        name: fs\n        config:\n          directory: {}\n",
            dir.join("config").display()
        ),
    )
    .expect("write engine config");
    let child = Command::new(&bin)
        .arg("--config")
        .arg(&config)
        .arg("--no-update-check")
        .env("III_TELEMETRY_ENABLED", "false")
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn iii engine");
    let engine = Engine {
        url: format!("ws://127.0.0.1:{port}"),
        child: Some(child),
        dir: Some(dir),
    };
    let deadline = Instant::now() + Duration::from_secs(60);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(
            Instant::now() < deadline,
            "the engine never listened on {port}"
        );
        sleep(Duration::from_millis(100)).await;
    }
    Some(engine)
}

/// A connection named `name`: two connections of one process need names of
/// their own.
async fn connect(url: &str, name: &str) -> IIIClient {
    let client = register_worker(
        url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: name.to_string(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let answer = client
            .call(CallRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(2_000),
            })
            .await;
        match answer {
            Ok(_) => return client,
            Err(error) => assert!(
                Instant::now() < deadline,
                "the engine at {url} never answered: {error}"
            ),
        }
        sleep(Duration::from_millis(250)).await;
    }
}

/// `value` without the engine's top-level `_caller_worker_id` stamp, which
/// it adds to the payload of every call a worker makes.
fn unstamped(mut value: Value) -> Value {
    if let Some(object) = value.as_object_mut() {
        object.remove("_caller_worker_id");
    }
    value
}

/// Evaluate `code` in the kept runtime `runtime_id` and return its result.
async fn eval(node: &RuntimeManager, runtime_id: &str, code: &str) -> Value {
    node.run(RunRequest {
        code: code.to_string(),
        runtime_id: Some(runtime_id.to_string()),
        namespace: None,
        keep: false,
        timeout_ms: Some(10_000),
    })
    .await
    .unwrap_or_else(|e| panic!("eval failed: {e}"))
    .result
}

/// The guest hooks' record, polled until `done` holds for it.
async fn seen_until(
    node: &RuntimeManager,
    runtime_id: &str,
    done: impl Fn(&[Value]) -> bool,
) -> Vec<Value> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let seen = eval(node, runtime_id, "return globalThis.seen").await;
        let seen = seen.as_array().cloned().unwrap_or_default();
        if done(&seen) {
            return seen;
        }
        assert!(
            Instant::now() < deadline,
            "the guest hooks recorded {seen:?}"
        );
        sleep(Duration::from_millis(100)).await;
    }
}

const GUEST: &str = r#"
globalThis.seen = [];
iii.registerFunction('probe::on-thing', (event) => event, {
  args_format: {
    type: 'object',
    properties: { tag: { type: 'string' }, refuse: { type: 'string' } },
    additionalProperties: false,
  },
});
iii.onObserverAdded('probe::on-thing', (obs) => {
  globalThis.seen.push({ method: 'added', observer: obs });
  if (obs.args.refuse) {
    const error = new Error('refused by the guest');
    error.code = obs.args.refuse;
    throw error;
  }
  globalThis.accepted = obs;
});
iii.onObserverRemoved('probe::on-thing', async (obs) => {
  globalThis.seen.push({ method: 'removed', observer: obs });
});
return 'ready';
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guest_hooks_see_accept_reject_and_call_through_observers() {
    let Some(engine) = engine().await else {
        assert!(
            std::env::var("III_E2E_REQUIRE").as_deref() != Ok("1"),
            "III_E2E_REQUIRE=1 but no engine: set III_ENGINE_WS_URL or III_ENGINE_BIN"
        );
        eprintln!("skipping: no iii engine (set III_ENGINE_WS_URL or III_ENGINE_BIN)");
        return;
    };

    iii_node_core::runtime::init_v8_platform();
    let host = Arc::new(connect(&engine.url, "code-runner").await);
    let node = RuntimeManager::new(
        Arc::new(CodeRunnerConfig::default().node()),
        Arc::new(IIIEngine::new(host.clone())),
        &code_runner::functions::seeded_ids(),
    );
    let created = node
        .run(RunRequest {
            code: GUEST.to_string(),
            runtime_id: None,
            namespace: Some("probe".to_string()),
            keep: true,
            timeout_ms: Some(10_000),
        })
        .await
        .unwrap_or_else(|e| panic!("the guest failed to boot: {e}"));
    assert_eq!(created.result, "ready");
    let runtime_id = created.runtime_id.expect("a kept runtime has an id");

    let consumer = connect(&engine.url, "code-runner-test-consumer").await;
    let (tx, mut reached) = mpsc::unbounded_channel();
    consumer.register_function(
        "test::target",
        RegisterFunction::new(move |payload: Value, metadata: Option<Value>| {
            let _ = tx.send((payload.clone(), metadata));
            Ok::<_, iii_sdk::Error>(json!({ "echoed": unstamped(payload) }))
        }),
    );

    // Accepted: the added hook sees the observer as the guest API describes
    // it, with its filters in iii's normalized form.
    let filters = json!({ "input.kind": { "$eq": "x" } });
    let binding = json!({ "binding": "b-1" });
    let accepted = consumer.observe_with(
        Source::function("probe::on-thing")
            .args(json!({ "tag": "a" }))
            .filters(filters.clone()),
        "test::target",
        ObserverOptions::default().metadata(binding.clone()),
    );
    timeout(Duration::from_secs(15), accepted.ready())
        .await
        .expect("the observer never became active")
        .expect("the guest rejected an observer its hook accepts");
    let seen = seen_until(&node, &runtime_id, |seen| seen.len() == 1).await;
    assert_eq!(seen[0]["method"], "added");
    assert_eq!(
        seen[0]["observer"],
        json!({
            "id": accepted.id(),
            "function_id": "probe::on-thing",
            "args": { "tag": "a" },
            "filters": { "$and": [filters] },
            "target": "test::target",
        })
    );

    // A call through the observer runs its target, whose metadata argument
    // is the observer's, and resolves to its result.
    let answer = eval(
        &node,
        &runtime_id,
        "return await iii.call({ observer: globalThis.accepted, payload: { hello: 'world' } })",
    )
    .await;
    assert_eq!(answer, json!({ "echoed": { "hello": "world" } }));
    let (payload, metadata) = timeout(Duration::from_secs(5), reached.recv())
        .await
        .expect("the call through the observer never reached its target")
        .unwrap();
    assert_eq!(unstamped(payload), json!({ "hello": "world" }));
    assert_eq!(metadata, Some(binding.clone()));

    // A call through an observer takes no metadata: refused before anything
    // is sent, with the SDKs' message.
    let refused_metadata = eval(
        &node,
        &runtime_id,
        "try { await iii.call({ observer: globalThis.accepted, payload: { forged: true }, \
                               metadata: { binding: 'forged' } }); return 'called' } \
         catch (e) { return e.message }",
    )
    .await;
    assert_eq!(
        refused_metadata,
        "a call through an observer takes no metadata; its target receives the observer's metadata"
    );

    // `metadata: null` is no metadata: through the observer the target gets
    // the observer's (the refused call above never reached it), and a call
    // of the function gets none.
    let answer = eval(
        &node,
        &runtime_id,
        "return await iii.call({ observer: globalThis.accepted, payload: { n: 2 }, metadata: null })",
    )
    .await;
    assert_eq!(answer, json!({ "echoed": { "n": 2 } }));
    let (payload, metadata) = timeout(Duration::from_secs(5), reached.recv())
        .await
        .expect("the call through the observer never reached its target")
        .unwrap();
    assert_eq!(unstamped(payload), json!({ "n": 2 }));
    assert_eq!(metadata, Some(binding.clone()));
    let answer = eval(
        &node,
        &runtime_id,
        "return await iii.call({ function_id: 'test::target', payload: { n: 3 }, metadata: null })",
    )
    .await;
    assert_eq!(answer, json!({ "echoed": { "n": 3 } }));
    let (payload, metadata) = timeout(Duration::from_secs(5), reached.recv())
        .await
        .expect("the call never reached its target")
        .unwrap();
    assert_eq!(unstamped(payload), json!({ "n": 3 }));
    assert_eq!(metadata, None);

    // Rejected: the hook's thrown code is the observer's reason.
    let refused = consumer.observe(
        Source::function("probe::on-thing").args(json!({ "refuse": "GUEST_REFUSED" })),
        "test::target",
    );
    let error = timeout(Duration::from_secs(15), refused.ready())
        .await
        .expect("the refused observer never settled")
        .expect_err("the guest's throw must reject the observer");
    assert_eq!(
        refused.reason().map(|r| r.code),
        Some("GUEST_REFUSED".to_string()),
        "{error}"
    );
    seen_until(&node, &runtime_id, |seen| seen.len() == 2).await;

    // Removed: disposing the accepted observer runs the removed hook once.
    accepted.dispose().await.expect("dispose the observer");
    let seen = seen_until(&node, &runtime_id, |seen| seen.len() == 3).await;
    assert_eq!(seen[2]["method"], "removed");
    assert_eq!(seen[2]["observer"]["id"], accepted.id());
    sleep(Duration::from_millis(300)).await;
    let seen = eval(&node, &runtime_id, "return globalThis.seen").await;
    let removed: Vec<&Value> = seen
        .as_array()
        .unwrap()
        .iter()
        .filter(|entry| entry["method"] == "removed")
        .collect();
    assert_eq!(
        removed.len(),
        1,
        "the removed hook runs once, and never for the rejected observer: {seen}"
    );

    // A call through a removed observer is refused.
    let refused_call = eval(
        &node,
        &runtime_id,
        "try { await iii.call({ observer: globalThis.accepted, payload: {} }); return 'called' } \
         catch (e) { return String(e.message || e) }",
    )
    .await;
    assert_ne!(refused_call, "called");
    assert!(
        refused_call
            .as_str()
            .unwrap_or_default()
            .contains(accepted.id()),
        "{refused_call}"
    );

    // The guest observes an id another connection holds: the engine refuses
    // it with `OBSERVER_ID_CONFLICT`, and that refusal is the observer's
    // final answer, so its dispose waits on nothing. Once the holder lets
    // the id go, the guest's next observe under it is the observer the
    // source's calls reach.
    const HELD: &str = "probe::held";
    consumer.register_function(
        "test::held-source",
        RegisterFunction::new(|payload: Value| Ok::<_, iii_sdk::Error>(payload)),
    );
    let held = consumer.observe_with(
        Source::function("test::held-source"),
        "test::target",
        ObserverOptions::default().id(HELD),
    );
    timeout(Duration::from_secs(15), held.ready())
        .await
        .expect("the holder's observer never became active")
        .expect("the holder's observer was refused");
    let observe_held = format!(
        "globalThis.held = await iii.observe('test::held-source', 'test::target', \
         {{ id: '{HELD}' }}); return globalThis.held.id"
    );
    assert_eq!(eval(&node, &runtime_id, &observe_held).await, json!(HELD));
    sleep(Duration::from_millis(500)).await;
    let elapsed = eval(
        &node,
        &runtime_id,
        "const started = Date.now(); await globalThis.held.dispose(); \
         return Date.now() - started",
    )
    .await;
    let elapsed = elapsed.as_u64().expect("milliseconds");
    assert!(
        elapsed < 1_000,
        "the refused observer's dispose took {elapsed} ms"
    );
    held.dispose().await.expect("dispose the holder's observer");
    assert_eq!(eval(&node, &runtime_id, &observe_held).await, json!(HELD));
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        consumer
            .call(CallRequest {
                function_id: "test::held-source".into(),
                payload: json!({ "n": 4 }),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await
            .expect("call the source");
        if let Ok(Some((envelope, _))) = timeout(Duration::from_millis(500), reached.recv()).await {
            assert_eq!(envelope["observer_id"], HELD, "{envelope}");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "no call of the source reached the guest's second observer"
        );
    }

    node.teardown_runtime(&runtime_id)
        .await
        .unwrap_or_else(|e| panic!("teardown failed: {e}"));
    consumer.shutdown_async().await;
    host.shutdown_async().await;
}

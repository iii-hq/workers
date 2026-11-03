//! `tmux::on-changed` through a real engine (`III_ENGINE_BIN`; the test skips
//! when it is not set): a new observer receives the current overview once,
//! as a call of the function for it alone, in the shape every overview
//! reaches it (the envelope for a default observer, the overview itself under
//! `deliver: output`); the other observers do not receive it, and a change
//! reaches every observer. `III_TEST_ENGINE_PORT` picks the engine's port
//! (default: a free one).

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, Deliver, IIIClient, InitOptions, RegisterFunction, Source,
};
use serde_json::{json, Value};
use tmux::collector::{Collector, ConsoleClients};
use tmux::config::WorkerConfig;
use tmux::events::{self, Monitor, ON_CHANGED_FN};

const WAIT: Duration = Duration::from_secs(15);

struct RealEngine {
    child: Child,
    url: String,
    root: PathBuf,
}

impl RealEngine {
    /// `III_ENGINE_BIN` with only its worker manager listening on 127.0.0.1.
    /// `None` when the variable is not set.
    async fn start() -> Option<Self> {
        let binary = PathBuf::from(std::env::var_os("III_ENGINE_BIN")?);
        let port = match std::env::var("III_TEST_ENGINE_PORT") {
            Ok(port) => port.parse::<u16>().expect("III_TEST_ENGINE_PORT is a port"),
            Err(_) => TcpListener::bind("127.0.0.1:0")
                .unwrap()
                .local_addr()
                .unwrap()
                .port(),
        };
        let root = std::env::temp_dir().join(format!("tmux-engine-{}", uid()));
        std::fs::create_dir_all(&root).unwrap();
        let config = root.join("engine.yaml");
        std::fs::write(
            &config,
            format!(
                "workers:\n  - name: iii-worker-manager\n    config:\n      host: 127.0.0.1\n      port: {port}\n"
            ),
        )
        .unwrap();
        let child = Command::new(binary)
            .args(["--no-update-check", "--config"])
            .arg(&config)
            .current_dir(&root)
            .env("III_TELEMETRY_ENABLED", "false")
            .env("OTEL_ENABLED", "false")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let engine = Self {
            child,
            url: format!("ws://127.0.0.1:{port}"),
            root,
        };
        let probe = engine.client("engine-probe").await;
        tokio::time::timeout(Duration::from_secs(30), async {
            while call(&probe, "engine::functions::list", json!({}))
                .await
                .is_err()
            {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("the engine answers");
        probe.shutdown_async().await;
        Some(engine)
    }

    /// A worker connection named `name`, once the engine registered it.
    async fn client(&self, name: &str) -> IIIClient {
        let iii = register_worker(
            &self.url,
            InitOptions {
                metadata: Some(WorkerMetadata {
                    name: name.into(),
                    ..Default::default()
                }),
                ..Default::default()
            },
        );
        iii.wait_until_registered(Duration::from_secs(30))
            .await
            .expect("registered with the engine");
        iii
    }

    /// Waits until the engine lists `function_id`.
    async fn function(&self, iii: &IIIClient, function_id: &str) {
        tokio::time::timeout(WAIT, async {
            while call(
                iii,
                "engine::functions::info",
                json!({ "function_id": function_id }),
            )
            .await
            .is_err()
            {
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
        })
        .await
        .expect("the function is registered");
    }
}

impl Drop for RealEngine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, Error> {
    iii.call(CallRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
}

fn uid() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

async fn until(what: &str, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Long enough for a delivery that should not happen to show up.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(500)).await;
}

/// A target function that records what it receives.
#[derive(Clone, Default)]
struct Inbox(Arc<Mutex<Vec<Value>>>);

impl Inbox {
    fn function(&self) -> RegisterFunction {
        let inbox = self.clone();
        RegisterFunction::new(move |input: Value| {
            inbox.0.lock().unwrap().push(input);
            Ok::<_, Error>(json!({ "ok": true }))
        })
    }

    fn len(&self) -> usize {
        self.0.lock().unwrap().len()
    }

    async fn wait(&self, n: usize) -> Vec<Value> {
        until(&format!("{n} deliveries"), || self.len() >= n).await;
        self.0.lock().unwrap().clone()
    }
}

fn is_overview(data: &Value) -> bool {
    data["sessions"]["sessions"].is_array() && data["ports"]["listeners"].is_array()
}

fn tmux_error(overview: &Value) -> String {
    overview["sessions"]["tmux"]["error"]
        .as_str()
        .unwrap_or_default()
        .to_string()
}

/// The envelope without what differs between any two deliveries.
fn stable(mut envelope: Value) -> Value {
    for key in [
        "invocation_id",
        "activation_id",
        "started_at_ms",
        "duration_ms",
        "input",
        "output",
    ] {
        envelope.as_object_mut().unwrap().remove(key);
    }
    envelope
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_new_observer_receives_the_current_overview_alone_in_its_own_shape() {
    let Some(engine) = RealEngine::start().await else {
        eprintln!("skipped: III_ENGINE_BIN is not set");
        return;
    };
    let id = uid();

    // The worker's feed, on a tmux server nobody else uses: the overview is
    // empty, or reports tmux missing. Listening ports are scanned once, so
    // the host's own ports coming and going do not change it.
    let host = engine.client("tmux").await;
    let base = WorkerConfig {
        socket_name: Some(format!("iii-tmux-engine-{id}")),
        poll_interval_ms: 50,
        ports_interval_ms: 3_600_000,
        ..WorkerConfig::default()
    };
    let config = base.clone().into_shared();
    let collector = Arc::new(Collector::new(config.clone(), ConsoleClients::default()));
    let monitor = Monitor::new(host.clone(), collector);
    events::register(&host, monitor.clone());
    monitor.spawn();
    engine.function(&host, ON_CHANGED_FN).await;

    let watcher = engine.client("console").await;
    let (envelopes, outputs, late) = (Inbox::default(), Inbox::default(), Inbox::default());
    let to_envelopes = format!("console-{id}::envelope");
    let to_outputs = format!("console-{id}::output");
    let to_late = format!("console-{id}::late");
    watcher.register_function(&to_envelopes, envelopes.function());
    watcher.register_function(&to_outputs, outputs.function());
    watcher.register_function(&to_late, late.function());

    // A, iii's default delivery: its first overview arrives as an envelope.
    let a = watcher.observe(Source::function(ON_CHANGED_FN), &to_envelopes);
    tokio::time::timeout(WAIT, a.ready())
        .await
        .unwrap()
        .unwrap();
    envelopes.wait(1).await;

    // B, `deliver: output`: its first overview is the overview itself, and
    // it reaches B alone.
    let b = watcher.observe(
        Source::function(ON_CHANGED_FN).deliver(Deliver::Output),
        &to_outputs,
    );
    tokio::time::timeout(WAIT, b.ready())
        .await
        .unwrap()
        .unwrap();
    outputs.wait(1).await;
    settle().await;
    assert_eq!(envelopes.len(), 1, "B's first overview reached A");
    assert_eq!(outputs.len(), 1, "B received its first overview once");

    // A change: one call of the function, delivered to both.
    let missing = format!("iii-tmux-missing-{id}");
    config.store(Arc::new(WorkerConfig {
        tmux_binary: missing.clone(),
        ..base
    }));
    monitor.poke();
    let got_a = envelopes.wait(2).await;
    let got_b = outputs.wait(2).await;

    // C, added after the change: its first overview is the current one, and
    // it reaches C alone.
    let c = watcher.observe(
        Source::function(ON_CHANGED_FN).deliver(Deliver::Output),
        &to_late,
    );
    tokio::time::timeout(WAIT, c.ready())
        .await
        .unwrap()
        .unwrap();
    let got_c = late.wait(1).await;
    settle().await;
    assert_eq!(late.len(), 1, "C received its first overview once");
    assert_eq!(envelopes.len(), 2, "C's first overview reached A");
    assert_eq!(outputs.len(), 2, "C's first overview reached B");

    // A receives every overview in one shape: the envelope of the call, its
    // output the overview.
    let (first, change) = (&got_a[0], &got_a[1]);
    let keys = |envelope: &Value| {
        let mut keys: Vec<String> = envelope.as_object().unwrap().keys().cloned().collect();
        keys.sort();
        keys
    };
    assert_eq!(keys(first), keys(change), "{got_a:#?}");
    assert_eq!(stable(first.clone()), stable(change.clone()), "{got_a:#?}");
    for envelope in &got_a {
        assert_eq!(envelope["observer_id"], a.id(), "{envelope}");
        assert_eq!(envelope["function_id"], ON_CHANGED_FN, "{envelope}");
        assert!(is_overview(&envelope["output"]), "{envelope}");
    }
    assert!(!tmux_error(&first["output"]).contains(&missing), "{first}");
    assert!(tmux_error(&change["output"]).contains(&missing), "{change}");

    // B and C receive the overview itself, every time.
    for overview in got_b.iter().chain(&got_c) {
        assert!(is_overview(overview), "{overview}");
    }
    assert!(!tmux_error(&got_b[0]).contains(&missing), "{got_b:#?}");
    assert!(tmux_error(&got_b[1]).contains(&missing), "{got_b:#?}");
    assert!(tmux_error(&got_c[0]).contains(&missing), "{got_c:#?}");

    for client in [watcher, host] {
        client.shutdown_async().await;
    }
}

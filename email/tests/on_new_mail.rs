//! `email::on-new-mail` through an isolated real engine (`III_ENGINE_BIN`, on
//! an ephemeral 127.0.0.1 port; the test skips when it is not set): the
//! function is private with the `NewMailEvent` formats, the dispatcher calls
//! it only while it is observed and without waiting for targets, and iii
//! delivers each event to the observers whose filters on `input.account` and
//! `input.folder` match it.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use email::events::dispatcher::{EngineDispatcher, EventDispatcher};
use email::events::new_mail::{self, NewMailEvent, ON_NEW_MAIL_FN};
use email::events::Event;
use iii_sdk::runtime::{IIIConnectionState, WorkerMetadata};
use iii_sdk::{
    register_worker, CallRequest, Deliver, IIIClient, InitOptions, RegisterFunction, Source,
};
use serde_json::{json, Value};

const TARGET: &str = "support-bot::on-mail";

struct RealEngine {
    child: Child,
    url: String,
    _root: tempfile::TempDir,
}

impl RealEngine {
    async fn start() -> Option<Self> {
        let binary = PathBuf::from(std::env::var_os("III_ENGINE_BIN")?);
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let root = tempfile::tempdir().unwrap();
        let config = root.path().join("engine.yaml");
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
            .current_dir(root.path())
            .env("III_TELEMETRY_ENABLED", "false")
            .env("OTEL_ENABLED", "false")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let engine = Self {
            child,
            url: format!("ws://127.0.0.1:{port}"),
            _root: root,
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

    async fn client(&self, name: &str) -> Arc<IIIClient> {
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
        tokio::time::timeout(Duration::from_secs(30), async {
            while iii.get_connection_state() != IIIConnectionState::Connected {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("connected to the engine");
        Arc::new(iii)
    }
}

impl Drop for RealEngine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, iii_sdk::Error> {
    iii.call(CallRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
}

async fn until(what: &str, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(Duration::from_secs(10), async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{what}"));
}

fn mail(account: &str, folder: &str, uid: u32) -> Event {
    Event {
        account: account.into(),
        folder: folder.into(),
        uid,
        message_id: Some(format!("<{uid}@mx.example.com>")),
        from: Some("alice@example.com".into()),
        subject: Some(format!("Ticket #{uid}")),
        snippet: Some("hello".into()),
        ts: None,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_new_mail_is_private_and_reaches_the_matching_observers() {
    let Some(engine) = RealEngine::start().await else {
        eprintln!("skipping: set III_ENGINE_BIN to run against a real engine");
        return;
    };
    let iii = engine.client("email").await;
    new_mail::register(&iii);
    let consumer = engine.client("support-bot").await;

    let row = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(row) = call(
                &consumer,
                "engine::functions::info",
                json!({ "function_id": ON_NEW_MAIL_FN }),
            )
            .await
            {
                return row;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .expect("email::on-new-mail is registered");
    assert_eq!(row["private"], true, "{row}");
    assert!(row.get("args_format").is_none(), "takes no args: {row}");
    assert_eq!(row["request_schema"]["title"], "NewMailEvent", "{row}");
    assert_eq!(row["response_schema"]["title"], "NewMailEvent", "{row}");
    let refused = call(
        &consumer,
        ON_NEW_MAIL_FN,
        serde_json::to_value(NewMailEvent::from(mail("support", "INBOX", 0))).unwrap(),
    )
    .await
    .unwrap_err();
    assert!(refused.to_string().contains("FORBIDDEN"), "{refused}");

    let dispatcher = EngineDispatcher::new(iii.clone());
    // Unobserved: the dispatch builds and calls nothing, and the event is lost
    // to an observer added later.
    assert!(!iii.is_observed(ON_NEW_MAIL_FN));
    dispatcher.dispatch(mail("support", "INBOX", 1)).await;

    let received: Arc<Mutex<Vec<Value>>> = Arc::default();
    let log = received.clone();
    consumer.register_function(
        TARGET,
        RegisterFunction::new_async(move |event: Value| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                // A slow target: the dispatch does not wait for it.
                tokio::time::sleep(Duration::from_secs(2)).await;
                Ok::<_, iii_sdk::Error>(Value::Null)
            }
        }),
    );
    let observer = consumer.observe(
        Source::function(ON_NEW_MAIL_FN)
            .filters(json!({
                "input.account": { "$eq": "support" },
                "input.folder": { "$eq": "INBOX" },
            }))
            .deliver(Deliver::Output),
        TARGET,
    );
    tokio::time::timeout(Duration::from_secs(10), observer.ready())
        .await
        .expect("observer answered")
        .expect("email::on-new-mail accepts an observer without args");
    until("the email worker sees the observer", || {
        iii.is_observed(ON_NEW_MAIL_FN)
    })
    .await;

    let started = Instant::now();
    dispatcher.dispatch(mail("sales", "INBOX", 2)).await;
    dispatcher.dispatch(mail("support", "Archive", 3)).await;
    dispatcher.dispatch(mail("support", "INBOX", 4)).await;
    assert!(
        started.elapsed() < Duration::from_secs(1),
        "the dispatch does not wait for targets"
    );
    until("the observer receives the matching event", || {
        !received.lock().unwrap().is_empty()
    })
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let events = received.lock().unwrap().clone();
    assert_eq!(
        events,
        vec![serde_json::to_value(NewMailEvent::from(mail("support", "INBOX", 4))).unwrap()]
    );

    observer.dispose().await.unwrap();
    until("the email worker sees the observer leave", || {
        !iii.is_observed(ON_NEW_MAIL_FN)
    })
    .await;
    dispatcher.dispatch(mail("support", "INBOX", 5)).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(received.lock().unwrap().len(), 1);

    consumer.shutdown_async().await;
    iii.shutdown_async().await;
}

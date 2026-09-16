use std::io::Read as _;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use harness::config::WorkerConfig;
use harness::configuration::ConfigCell;
use harness::deps::Deps;
use harness::discovery;
use harness::events::TurnEvents;
use harness::functions::send::{self, MessageInput, SendRequest};
use harness::hooks::HookRegistry;
use harness::skills;
use harness::types::message::AgentMessage;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, InitOptions, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

struct Engine {
    url: String,
    child: std::process::Child,
    _dir: tempfile::TempDir,
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn worker_options(name: &str) -> InitOptions {
    InitOptions {
        metadata: Some(WorkerMetadata {
            name: format!("harness-read-only-test-{name}-{}", std::process::id()),
            ..WorkerMetadata::default()
        }),
        ..InitOptions::default()
    }
}

async fn isolated_engine() -> Option<Engine> {
    let binary = std::env::var_os("III_ENGINE_BIN")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::process::Command::new("iii")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .ok()
                .filter(|status| status.success())
                .map(|_| std::path::PathBuf::from("iii"))
        })?;
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()?
        .port();
    let dir = tempfile::tempdir().ok()?;
    let config = dir.path().join("config.yaml");
    std::fs::write(
        &config,
        format!("workers:\n  - name: iii-worker-manager\n    config:\n      port: {port}\n"),
    )
    .ok()?;
    let mut child = std::process::Command::new(binary)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&config)
        .current_dir(dir.path())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .ok()?;
    let url = format!("ws://127.0.0.1:{port}");
    let probe = register_worker(&url, worker_options("probe"));
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if probe
            .trigger(TriggerRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(500),
            })
            .await
            .is_ok()
        {
            probe.shutdown();
            return Some(Engine {
                url,
                child,
                _dir: dir,
            });
        }
        if Instant::now() >= deadline {
            probe.shutdown();
            let _ = child.kill();
            let _ = child.wait();
            if std::env::var_os("III_ENGINE_BIN").is_some() {
                let mut stderr = String::new();
                if let Some(mut pipe) = child.stderr.take() {
                    let _ = pipe.read_to_string(&mut stderr);
                }
                panic!("iii engine did not become ready: {stderr}");
            }
            return None;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn imported_session_start_requires_explicit_runtime_options() {
    let Some(engine) = isolated_engine().await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };
    let reads = Arc::new(AtomicUsize::new(0));
    let sessions = register_worker(&engine.url, worker_options("sessions"));
    let observed = reads.clone();
    sessions.register_function(
        "session::get",
        RegisterFunction::new_async(move |payload: Value| {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                let session_id = payload["session_id"].as_str().unwrap();
                Ok::<_, iii_sdk::Error>(match session_id {
                    "missing-session" => Value::Null,
                    "native-session" => json!({
                        "meta": {
                            "session_id": session_id,
                            "metadata": {}
                        }
                    }),
                    "read-only-session" => json!({
                        "meta": {
                            "session_id": session_id,
                            "metadata": { "read_only": true }
                        }
                    }),
                    _ => json!({
                        "meta": {
                            "session_id": session_id,
                            "metadata": { "external_source": "codex" }
                        }
                    }),
                })
            }
        }),
    );
    sessions.register_function(
        "state::get",
        RegisterFunction::new_async(|_: Value| async { Ok::<_, iii_sdk::Error>(Value::Null) }),
    );
    let ensures = Arc::new(AtomicUsize::new(0));
    let observed = ensures.clone();
    sessions.register_function(
        "session::ensure",
        RegisterFunction::new_async(move |_: Value| {
            let observed = observed.clone();
            async move {
                observed.fetch_add(1, Ordering::SeqCst);
                Ok::<_, iii_sdk::Error>(json!({}))
            }
        }),
    );
    sessions
        .wait_until_registered(Duration::from_secs(5))
        .await
        .expect("session stub registered");

    let iii = Arc::new(register_worker(&engine.url, worker_options("caller")));
    iii.wait_until_registered(Duration::from_secs(5))
        .await
        .expect("caller registered");
    let config: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
    let deps = Deps::new(
        iii.clone(),
        config,
        discovery::new_cell(),
        skills::new_cell(),
        TurnEvents::register(&iii),
        HookRegistry::register(&iii),
    );

    let send_error = send::handle(
        &deps,
        SendRequest {
            session_id: Some("read-only-session".into()),
            message: MessageInput::Text("must not append".into()),
            model: Some("must-not-call-model".into()),
            provider: None,
            idempotency_key: None,
            session: None,
            options: None,
        },
    )
    .await
    .expect_err("send must reject a read-only session");
    assert_eq!(send_error.code(), "harness/invalid_request");
    assert!(send_error.to_string().contains("read-only"));

    let inject_error = match send::inject(
        &deps,
        "read-only-session",
        AgentMessage::user_text("must not inject"),
        Some("notification-entry"),
        None,
    )
    .await
    {
        Ok(_) => panic!("inject must reject a read-only session"),
        Err(error) => error,
    };
    assert_eq!(inject_error.code(), "harness/invalid_request");

    let missing_model = send::handle(
        &deps,
        SendRequest {
            session_id: Some("imported-session".into()),
            message: MessageInput::Text("continue".into()),
            model: None,
            provider: None,
            idempotency_key: None,
            session: None,
            options: Some(send::SendOptions {
                metadata: Some(json!({ "fs_scope": { "root": "/workspace" } })),
                ..Default::default()
            }),
        },
    )
    .await
    .expect_err("an imported session needs an explicit model");
    assert_eq!(missing_model.code(), "harness/invalid_request");
    assert!(missing_model.to_string().contains("explicit `model`"));

    let missing_root = send::handle(
        &deps,
        SendRequest {
            session_id: Some("imported-session".into()),
            message: MessageInput::Text("continue".into()),
            model: Some("test-model".into()),
            provider: None,
            idempotency_key: None,
            session: None,
            options: None,
        },
    )
    .await
    .expect_err("an imported session needs an explicit filesystem root");
    assert_eq!(missing_root.code(), "harness/invalid_request");
    assert!(missing_root
        .to_string()
        .contains("options.metadata.fs_scope.root"));
    assert_eq!(ensures.load(Ordering::SeqCst), 0);

    for (session_id, options) in [
        (
            "imported-session",
            Some(send::SendOptions {
                metadata: Some(json!({ "fs_scope": { "root": "/workspace" } })),
                ..Default::default()
            }),
        ),
        ("native-session", None),
        ("missing-session", None),
    ] {
        let error = send::handle(
            &deps,
            SendRequest {
                session_id: Some(session_id.into()),
                message: MessageInput::Text("continue".into()),
                model: Some("test-model".into()),
                provider: None,
                idempotency_key: None,
                session: None,
                options,
            },
        )
        .await
        .expect_err("the ensure stub deliberately returns a malformed response");
        assert_eq!(error.code(), "harness/dependency");
    }

    assert_eq!(ensures.load(Ordering::SeqCst), 3);
    assert_eq!(reads.load(Ordering::SeqCst), 7);

    iii.shutdown();
    sessions.shutdown();
}

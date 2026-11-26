//! Through a real engine (`III_ENGINE_BIN`; skipped when unset): the token
//! reaches the vault and nowhere else, `secret::provide` and `connect` refuse
//! observers, and `on-change` reaches an observer with the record.

mod support;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_cloud::config::Config;
use iii_cloud::ctx::{Ctx, IiiPlatform};
use iii_cloud::functions::register_all;
use iii_cloud::store::Store;
use iii_sdk::{Capture, Error, RegisterFunction, Source};
use serde_json::{Value, json};
use support::real_engine::{RealEngine, call};
use tokio::sync::RwLock;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const TOKEN: &str = "iiia_engine-test-token-0123456789";

fn remote_code(e: Error) -> String {
    match e {
        Error::Remote { code, .. } => code,
        other => other.to_string(),
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_token_stays_in_the_vault_and_the_events_reach_an_observer() {
    let Some(engine) = RealEngine::start().await else {
        eprintln!("skipped: III_ENGINE_BIN unset (the real-engine test did not run)");
        return;
    };
    // The vault, faked: the two calls the worker makes.
    let vault: Arc<Mutex<HashMap<String, String>>> = Arc::default();
    let secrets = engine.client("secrets").await;
    let v = vault.clone();
    secrets.register_function(
        "secrets::resolve",
        RegisterFunction::new_async(move |req: Value| {
            let v = v.clone();
            async move {
                let name = req["ref"]
                    .as_str()
                    .unwrap_or_default()
                    .trim_start_matches("secret://")
                    .to_string();
                match v.lock().unwrap().get(&name) {
                    Some(value) => Ok::<Value, Error>(json!({"name": name, "value": value})),
                    None => Err(Error::Remote {
                        code: "SECRET_NOT_FOUND".into(),
                        message: "no".into(),
                        stacktrace: None,
                    }),
                }
            }
        }),
    );
    let v = vault.clone();
    secrets.register_function(
        "secrets::compare-and-set",
        RegisterFunction::new_async(move |req: Value| {
            let v = v.clone();
            async move {
                let name = req["name"].as_str().unwrap().to_string();
                let mut map = v.lock().unwrap();
                let swapped = map.get(&name).map(String::as_str) == req["expected"].as_str();
                if swapped {
                    map.insert(name, req["value"].as_str().unwrap().to_string());
                }
                Ok::<Value, Error>(json!({"swapped": swapped}))
            }
        }),
    );
    // The mock cloud: the device flow ends at once.
    let server = MockServer::start().await;
    Mock::given(method("POST")).and(path("/api/v1/auth/device")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"device_code": "dc", "user_code": "ABCD-1234", "verification_uri": "https://cloud.test/device", "expires_in": 600, "interval": 5}))).mount(&server).await;
    Mock::given(method("POST")).and(path("/api/v1/auth/device/token")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"token": TOKEN, "session_id": "s", "expires_at": "2027-02-01T00:00:00Z", "user": {"id": "u", "email": "ada@example.test", "name": null}}))).mount(&server).await;
    Mock::given(method("GET"))
        .and(path("/api/v1/me"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(
                json!({"user": {"id": "u", "email": "ada@example.test"}, "teams": []}),
            ),
        )
        .mount(&server)
        .await;

    let iii = engine.client("iii-cloud").await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        cloud_url: server.uri(),
        data_dir: dir.path().join("data").to_string_lossy().into_owned(),
        ..Config::default()
    };
    let store = Arc::new(Store::open(&cfg.data_path()));
    let ctx = Ctx::with_tick(
        Arc::new(RwLock::new(Arc::new(cfg))),
        store,
        Arc::new(IiiPlatform { iii: iii.clone() }),
        Some(Duration::from_millis(10)),
    );
    register_all(&iii, &ctx);

    let probe = engine.client("probe").await;
    for id in [
        "iii-cloud::connect",
        "iii-cloud::secret::provide",
        "iii-cloud::values::save",
        "iii-cloud::on-change",
    ] {
        engine.function(&probe, id).await;
    }
    // Hidden functions are marked for the traces page and refuse observers.
    for id in ["iii-cloud::secret::provide", "iii-cloud::values::save"] {
        let info = engine.function(&probe, id).await;
        assert_eq!(
            info["metadata"]["trace_hidden"],
            json!(true),
            "{id}: {info}"
        );
    }
    probe.register_function(
        "probe::sink",
        RegisterFunction::new_async(|_: Value| async { Ok::<Value, Error>(json!({})) }),
    );
    engine.function(&probe, "probe::sink").await;
    for id in [
        "iii-cloud::secret::provide",
        "iii-cloud::values::save",
        "iii-cloud::connect",
    ] {
        let observer = probe.observe(Source::function(id), "probe::sink");
        let refused = tokio::time::timeout(Duration::from_secs(5), observer.ready())
            .await
            .unwrap()
            .unwrap_err();
        assert_eq!(remote_code(refused), "NOT_OBSERVABLE", "{id}");
    }

    // on-change reaches an observer, with the record.
    let seen: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sink = seen.clone();
    probe.register_function(
        "probe::events",
        RegisterFunction::new_async(move |event: Value| {
            let sink = sink.clone();
            async move {
                sink.lock().unwrap().push(event);
                Ok::<Value, Error>(json!({}))
            }
        }),
    );
    engine.function(&probe, "probe::events").await;
    let observer = probe.observe(
        Source::function("iii-cloud::on-change")
            .filters(json!({"input.kind": {"$eq": "connection"}}))
            .capture([Capture::Output]),
        "probe::events",
    );
    tokio::time::timeout(Duration::from_secs(10), observer.ready())
        .await
        .unwrap()
        .expect("on-change accepts an observer");

    let waiting = call(&probe, "iii-cloud::connect", json!({"method": "browser"}))
        .await
        .unwrap();
    assert_eq!(waiting["status"], "waiting");
    assert_eq!(waiting["device"]["user_code"], "ABCD-1234");
    let mut status = Value::Null;
    for _ in 0..200 {
        status = call(&probe, "iii-cloud::status", json!({})).await.unwrap();
        if status["connection"]["status"] == "connected" {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(status["connection"]["status"], "connected", "{status}");
    assert_eq!(
        vault
            .lock()
            .unwrap()
            .get("IIICLOUD_SESSION")
            .map(String::as_str),
        Some(TOKEN),
        "the vault has the token"
    );
    for _ in 0..100 {
        if seen
            .lock()
            .unwrap()
            .iter()
            .any(|e| e["output"]["terminal"] == true)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let events: Vec<Value> = seen
        .lock()
        .unwrap()
        .iter()
        .map(|e| e["output"].clone())
        .collect();
    assert!(
        events
            .iter()
            .any(|e| e["kind"] == "connection" && e["record"]["status"] == "connected"),
        "{events:?}"
    );
    // The token is in no output, event, state file or trace.
    let traces = call(&probe, "engine::traces::list", json!({}))
        .await
        .map(|v| v.to_string())
        .unwrap_or_default();
    let state = std::fs::read_to_string(ctx.store.dir().join("state.json")).unwrap();
    for text in [
        waiting.to_string(),
        status.to_string(),
        serde_json::to_string(&events).unwrap(),
        state,
        traces,
    ] {
        assert!(!text.contains(TOKEN));
    }
}

/// Every span payload the engine kept, as text.
/// A key connection, a selected environment with a workspace, and one check.
/// A key connection, production selected and a ready check, in a real git
/// checkout (a hand-off makes its worktree from it). Keep the repo alive.
fn seed_selected(ctx: &Ctx) -> support::repo::Repo {
    use iii_cloud::types::*;
    let repo = support::repo::repo();
    let ws = repo.checkout.clone();
    let (plan, _) = iii_cloud::check::plan_part(&json!({"can_deploy": true, "plan": {}}));
    ctx.store.update(|s| {
        let mut conn = Connection::signed_out("http://cloud.test");
        conn.status = ConnStatus::Connected;
        conn.method = Some(Method::ApiKey);
        conn.api_key = Some(ApiKeyRef {
            secret: "K".into(),
            team_id: "t".into(),
            environment_id: "e".into(),
        });
        s.connection = Some(conn);
        s.project = Some(Project {
            path: ws.to_string_lossy().into(),
            repository: "a/b".into(),
            checkout: Checkout {
                branch: None,
                head: "h".into(),
                changed_files: vec![],
                ahead: 0,
                not_served: None,
                branches: vec![],
            },
            team: None,
            teams: vec![],
            cloud_project: None,
            environment: Some(EnvironmentInfo {
                id: "e".into(),
                name: "production".into(),
                production: true,
                branch: "main".into(),
                manifest: None,
                url: None,
            }),
            environments: vec![],
            importable: None,
            github_access: None,
            cloud_projects: vec![],
            other_cloud_projects: vec![],
            default_branch: None,
            workspace: Some(Workspace {
                mode: WorkspaceMode::Checkout,
                path: ws.to_string_lossy().into(),
                branch: "main".into(),
                from_head: None,
                not_brought: vec![],
                source: None,
                not_brought_reasons: Default::default(),
                source_sha: None,
            }),
            fell_back_from: None,
        });
        s.put_check(Check {
            id: "chk".into(),
            environment_id: "e".into(),
            commit: "abc".into(),
            base: String::new(),
            checked_at: "t".into(),
            duration_ms: 0,
            plan,
            here: HerePart {
                problems: 0,
                iii_version: None,
                accepts_cloud_block: true,
            },
            problems: vec![],
            run: None,
            ready: false,
            report: String::new(),
            manifest: None,
            handoff_id: None,
            commits: vec![],
        });
    });
    repo
}

fn fake_fn(iii: &iii_sdk::IIIClient, id: &str, f: impl Fn(Value) -> Value + Send + Sync + 'static) {
    let f = Arc::new(f);
    iii.register_function(
        id,
        RegisterFunction::new_async(move |req: Value| {
            let f = f.clone();
            async move { Ok::<Value, Error>(f(req)) }
        }),
    );
}

/// A hand-off follows a session through a real engine: the worker calls the
/// session and harness functions by id, `status.agent` shows the work, and stop
/// reaches `harness::stop`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_handoff_follows_a_session_through_a_real_engine() {
    let Some(engine) = RealEngine::start().await else {
        eprintln!("skipped: III_ENGINE_BIN unset (the real-engine test did not run)");
        return;
    };
    let marker: Arc<Mutex<String>> = Arc::default();
    let stopped: Arc<Mutex<Vec<Value>>> = Arc::default();
    let sessions = engine.client("fake-sessions").await;
    let created = chrono::Utc::now().timestamp_millis() + 60_000;
    fake_fn(
        &sessions,
        "session::list",
        move |_| json!({"sessions": [{"session_id": "s1", "status": "working", "created_at": created}]}),
    );
    let m = marker.clone();
    fake_fn(&sessions, "session::messages", move |req| {
        if req["roles"] == json!(["user"]) {
            let text = format!("{}\nTake this to iii cloud.", m.lock().unwrap());
            return json!({"messages": [{"entry_id": "u", "message": {"role": "user", "content": [{"type": "text", "text": text}], "timestamp": 1}}]});
        }
        if req.get("after_entry_id").is_some_and(|v| !v.is_null()) {
            return json!({"messages": []});
        }
        json!({"messages": [{"entry_id": "a1", "message": {"role": "assistant", "content": [{"type": "function_call", "id": "c", "function_id": "f", "arguments": null}], "timestamp": 2}}]})
    });
    fake_fn(
        &sessions,
        "session::get",
        |_| json!({"meta": {"session_id": "s1", "status": "working", "updated_at": chrono::Utc::now().timestamp_millis()}}),
    );
    fake_fn(
        &sessions,
        "session::messages-tail",
        |_| json!({"messages": [{"entry_id": "a1", "message": {"role": "assistant", "content": [{"type": "function_call", "id": "c", "function_id": "f", "arguments": {"description": "Reading the compose file"}}], "timestamp": chrono::Utc::now().timestamp_millis()}}], "has_more": false}),
    );
    fake_fn(&sessions, "harness::ask::list", |_| json!({"entries": []}));
    let s = stopped.clone();
    fake_fn(&sessions, "harness::stop", move |req| {
        s.lock().unwrap().push(req);
        json!({"stopping": true})
    });

    let iii = engine.client("iii-cloud").await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        cloud_url: "http://cloud.test".into(),
        data_dir: dir.path().join("data").to_string_lossy().into_owned(),
        ..Config::default()
    };
    let store = Arc::new(Store::open(&cfg.data_path()));
    let ctx = Ctx::with_tick(
        Arc::new(RwLock::new(Arc::new(cfg))),
        store,
        Arc::new(IiiPlatform { iii: iii.clone() }),
        Some(Duration::from_millis(20)),
    );
    let _repo = seed_selected(&ctx);
    register_all(&iii, &ctx);
    let probe = engine.client("probe").await;
    engine.function(&probe, "iii-cloud::handoff").await;
    for id in ["session::list", "harness::stop"] {
        engine.function(&probe, id).await;
    }
    let out = call(
        &probe,
        "iii-cloud::handoff",
        json!({"check_id": "chk", "kind": "ready"}),
    )
    .await
    .unwrap();
    *marker.lock().unwrap() = out["marker"].as_str().unwrap().to_string();
    let mut agent = Value::Null;
    for _ in 0..200 {
        agent = call(&probe, "iii-cloud::status", json!({})).await.unwrap()["agent"].clone();
        if agent["state"] == "working" && agent["steps"] == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(agent["state"], "working", "{agent}");
    assert_eq!(
        (agent["session_id"].as_str(), agent["current"].as_str()),
        (Some("s1"), Some("Reading the compose file"))
    );
    let stop = call(&probe, "iii-cloud::handoff::stop", json!({"id": out["id"]}))
        .await
        .unwrap();
    assert_eq!(stop["stopping"], true);
    assert_eq!(stopped.lock().unwrap()[0]["session_id"], "s1");
}

/// Read-only probe of a real session on a real engine. Prints only the derived
/// `status.agent`, never a message. Ignored: it connects to the engine named by
/// IIICLOUD_PROBE_URL (and namespace IIICLOUD_PROBE_NAMESPACE) and reads the
/// session IIICLOUD_PROBE_SESSION; it sends nothing to the session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "reads a live engine: set IIICLOUD_PROBE_URL and IIICLOUD_PROBE_SESSION"]
async fn probe_a_live_session_prints_only_the_derived_agent_state() {
    let (Ok(url), Ok(session)) = (
        std::env::var("IIICLOUD_PROBE_URL"),
        std::env::var("IIICLOUD_PROBE_SESSION"),
    ) else {
        eprintln!("skipped: IIICLOUD_PROBE_URL / IIICLOUD_PROBE_SESSION unset");
        return;
    };
    let iii = Arc::new(iii_sdk::register_worker(
        &url,
        iii_sdk::InitOptions {
            metadata: Some(iii_sdk::runtime::WorkerMetadata {
                name: "iii-cloud-probe".into(),
                ..Default::default()
            }),
            namespace: std::env::var("IIICLOUD_PROBE_NAMESPACE").ok(),
            ..Default::default()
        },
    ));
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        data_dir: dir.path().join("data").to_string_lossy().into_owned(),
        ..Config::default()
    };
    let store = Arc::new(Store::open(&cfg.data_path()));
    let ctx = Ctx::new(
        Arc::new(RwLock::new(Arc::new(cfg))),
        store,
        Arc::new(IiiPlatform { iii: iii.clone() }),
    );
    use iii_cloud::types::*;
    let agent = AgentState {
        handoff_id: "probe".into(),
        kind: None,
        session_id: Some(session),
        state: AgentPhase::Starting,
        started_at: "t".into(),
        last_activity_at: None,
        steps: 0,
        current: None,
        question: None,
        working_on: vec![],
        note: None,
        waiting_for: None,
    };
    ctx.store.update(|s| {
        s.handoffs.push(Handoff {
            id: "probe".into(),
            marker: "probe".into(),
            environment_id: "e".into(),
            check_id: Some("c".into()),
            kind: HandoffKind::Ready,
            deployment_id: None,
            asked_at_ms: 0,
            agent,
            checked: true,
            pending_text: None,
            workspace_path: String::new(),
            branch: String::new(),
            team_id: String::new(),
            env: None,
            manifest: None,
            candidate: false,
            worktree_note: None,
            continues: 0,
        });
    });
    let state = iii_cloud::handoff::poll_now(&ctx, "probe")
        .await
        .expect("the probe hand-off");
    println!(
        "status.agent = {}",
        serde_json::to_string_pretty(&state).unwrap()
    );
}

/// A follow-up hand-off is sent into the existing chat with `harness::send`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_follow_up_handoff_is_delivered_with_harness_send() {
    let Some(engine) = RealEngine::start().await else {
        eprintln!("skipped: III_ENGINE_BIN unset (the real-engine test did not run)");
        return;
    };
    let sent: Arc<Mutex<Vec<Value>>> = Arc::default();
    let fake = engine.client("fake-chat").await;
    fake_fn(
        &fake,
        "session::get",
        |_| json!({"meta": {"session_id": "s1", "status": "idle", "updated_at": 1, "metadata": {"model": "openai::gpt-x"}}}),
    );
    fake_fn(&fake, "session::list", |_| json!({"sessions": []}));
    fake_fn(&fake, "harness::ask::list", |_| json!({"entries": []}));
    let log = sent.clone();
    fake_fn(&fake, "harness::send", move |req| {
        log.lock().unwrap().push(req);
        json!({"session_id": "s1", "turn_id": "t1"})
    });
    let iii = engine.client("iii-cloud").await;
    let dir = tempfile::tempdir().unwrap();
    let cfg = Config {
        cloud_url: "http://cloud.test".into(),
        data_dir: dir.path().join("data").to_string_lossy().into_owned(),
        ..Config::default()
    };
    let store = Arc::new(Store::open(&cfg.data_path()));
    let ctx = Ctx::with_tick(
        Arc::new(RwLock::new(Arc::new(cfg))),
        store,
        Arc::new(IiiPlatform { iii: iii.clone() }),
        Some(Duration::from_millis(20)),
    );
    let _repo = seed_selected(&ctx);
    // The previous hand-off already has its chat.
    ctx.store.update(|s| {
        use iii_cloud::types::*;
        s.handoffs.push(Handoff {
            id: "prev".into(),
            marker: "m".into(),
            environment_id: "e".into(),
            check_id: Some("chk".into()),
            kind: HandoffKind::Ready,
            deployment_id: None,
            asked_at_ms: 0,
            agent: AgentState {
                handoff_id: "prev".into(),
                kind: Some(HandoffKind::Ready),
                session_id: Some("s1".into()),
                state: AgentPhase::Done,
                started_at: "t".into(),
                last_activity_at: None,
                steps: 0,
                current: None,
                question: None,
                working_on: vec![],
                note: None,
                waiting_for: None,
            },
            checked: true,
            pending_text: None,
            workspace_path: String::new(),
            branch: String::new(),
            team_id: String::new(),
            env: None,
            manifest: None,
            candidate: false,
            worktree_note: None,
            continues: 0,
        });
    });
    register_all(&iii, &ctx);
    let probe = engine.client("probe").await;
    for id in ["iii-cloud::handoff", "harness::send", "session::get"] {
        engine.function(&probe, id).await;
    }
    let out = call(
        &probe,
        "iii-cloud::handoff",
        json!({"check_id": "chk", "kind": "pull_request", "text": "Open the pull request."}),
    )
    .await
    .unwrap();
    assert_eq!(
        (out["delivered"].as_bool(), out["session_id"].as_str()),
        (Some(true), Some("s1"))
    );
    let req = sent.lock().unwrap()[0].clone();
    assert_eq!(req["session_id"], "s1");
    assert_eq!(
        (req["model"].as_str(), req["provider"].as_str()),
        (Some("gpt-x"), Some("openai"))
    );
    assert!(
        req["message"]
            .as_str()
            .unwrap()
            .ends_with("Open the pull request.")
    );
    assert!(req["options"].get("agent").is_none());
    let status = call(&probe, "iii-cloud::status", json!({})).await.unwrap();
    assert_eq!(status["agent"]["kind"], "pull_request");
}

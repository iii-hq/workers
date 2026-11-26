//! Hand-offs to the agent: the marker, finding the session, following it, and
//! what `status.agent` says. The session and harness functions are fakes.

mod support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_cloud::config::Config;
use iii_cloud::handoff::{Observed, create, derive_phase, stop};
use iii_cloud::project::{resolve, use_project};
use iii_cloud::types::*;
use serde_json::{Value, json};
use support::repo::*;
use support::{Fake, Harness};
use wiremock::matchers::{method, path, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const BODY: &str = "BODYTEXT-the-agents-own-words-xyz";
const NOW_FN: fn() -> i64 = || chrono::Utc::now().timestamp_millis();

fn obs(status: &str, question: Option<&str>, last: Option<i64>, assistant: bool) -> Observed {
    Observed {
        status: status.into(),
        has_assistant: assistant,
        last_activity_ms: last,
        question: question.map(str::to_owned),
    }
}

#[test]
fn the_phase_follows_the_session_the_question_and_the_quiet() {
    let now = 10_000_000;
    let stall = 180_000;
    assert_eq!(
        derive_phase(
            &obs("working", None, Some(now - 1_000), true),
            now,
            stall,
            true
        ),
        AgentPhase::Working
    );
    assert_eq!(
        derive_phase(
            &obs("working", None, Some(now - 181_000), true),
            now,
            stall,
            true
        ),
        AgentPhase::Stalled
    );
    assert_eq!(
        derive_phase(
            &obs("working", Some("Which one?"), Some(now - 500_000), true),
            now,
            stall,
            true
        ),
        AgentPhase::WaitingForYou,
        "a question beats the quiet"
    );
    assert_eq!(
        derive_phase(&obs("idle", None, Some(now), true), now, stall, true),
        AgentPhase::Done
    );
    assert_eq!(
        derive_phase(&obs("done", None, Some(now), false), now, stall, true),
        AgentPhase::Done
    );
    assert_eq!(
        derive_phase(&obs("idle", None, Some(now), false), now, stall, false),
        AgentPhase::Starting,
        "an idle session that has not begun is not done"
    );
}

async fn setup() -> (Harness, MockServer, Repo, std::path::PathBuf) {
    let server = MockServer::start().await;
    let repo = repo();
    let h = connected(&server, "worker-compose.yaml").await;
    Mock::given(method("GET"))
        .and(path("/api/v1/support-matrix"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"engine": "0.27.0", "minimum_sdk": "0.26"})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path_regex(r"/plan$")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"can_deploy": true, "fingerprint": "fp", "plan": {"settings": {}, "resources": [], "issues": [], "deploy_issues": []}}))).mount(&server).await;
    let p = use_project(
        &h.ctx,
        UseRequest {
            path: repo.checkout.to_string_lossy().into(),
            environment_id: Some(ENV.into()),
            mode: WorkspaceMode::Worktree,
            bring_changes: None,
            cloud_project_id: None,
            from: None,
            source: None,
        },
    )
    .await
    .unwrap();
    let ws = std::path::PathBuf::from(p.workspace.unwrap().path);
    let (plan, _) = iii_cloud::check::plan_part(&json!({"can_deploy": true, "plan": {}}));
    h.ctx.store.update(|s| {
        s.put_check(Check {
            id: "chk".into(),
            environment_id: ENV.into(),
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
        })
    });
    (h, server, repo, ws)
}

fn ready_req() -> HandoffRequest {
    HandoffRequest {
        check_id: Some("chk".into()),
        kind: HandoffKind::Ready,
        deployment_id: None,
        text: None,
        candidate: None,
    }
}

/// The state of a fake session the test moves around.
#[derive(Clone)]
struct World {
    status: Arc<Mutex<String>>,
    question: Arc<Mutex<Option<String>>>,
    last_ms: Arc<Mutex<i64>>,
    /// When set, `session::messages-tail` answers with these items.
    tail: Arc<Mutex<Option<Vec<Value>>>>,
}

fn world() -> World {
    World {
        status: Arc::new(Mutex::new("working".into())),
        question: Arc::new(Mutex::new(None)),
        last_ms: Arc::new(Mutex::new(NOW_FN())),
        tail: Arc::new(Mutex::new(None)),
    }
}

/// Fake `session::*` and `harness::*`: an older session, and ours, which starts with `marker`.
fn install(fake: &Fake, marker: &str, w: &World, ours_created: i64) {
    let marker = marker.to_string();
    fake.on("session::list", move |_| {
        Ok(json!({"sessions": [
            {"session_id": "s-ours", "status": "working", "created_at": ours_created},
            {"session_id": "s-old", "status": "done", "created_at": 1}
        ]}))
    });
    let assistant_call = |id: &str, desc: &str| {
        json!({"entry_id": id, "message": {"role": "assistant", "content": [
        {"type": "text", "text": BODY},
        {"type": "function_call", "id": format!("c-{id}"), "function_id": "shell::exec", "arguments": {"description": desc, "command": BODY}}
    ], "timestamp": 5}})
    };
    let mk = marker.clone();
    fake.on("session::messages", move |req| {
        let session = req["session_id"].as_str().unwrap_or_default();
        if req["roles"] == json!(["user"]) {
            let text = if session == "s-ours" { format!("{mk}\\nPlease take this to iii cloud. {BODY}") } else { "something else".to_string() };
            return Ok(json!({"messages": [{"entry_id": "u1", "message": {"role": "user", "content": [{"type": "text", "text": text}], "timestamp": 1}}]}));
        }
        // The assistant's calls, counted once: nothing new after the watermark.
        if req.get("after_entry_id").is_some_and(|v| !v.is_null()) {
            return Ok(json!({"messages": []}));
        }
        Ok(json!({"messages": [assistant_call("a1", "x"), assistant_call("a2", "y"), assistant_call("a3", "z")]}))
    });
    let (status, last) = (w.status.clone(), w.last_ms.clone());
    fake.on("session::get", move |_| Ok(json!({"meta": {"session_id": "s-ours", "status": status.lock().unwrap().clone(), "updated_at": *last.lock().unwrap()}})));
    let last2 = w.last_ms.clone();
    let tail_override = w.tail.clone();
    fake.on("session::messages-tail", move |_| {
        if let Some(items) = tail_override.lock().unwrap().clone() {
            return Ok(json!({"messages": items, "has_more": false}));
        }
        Ok(json!({"messages": [
            {"entry_id": "a3", "message": {"role": "assistant", "content": [{"type": "function_call", "id": "c3", "function_id": "shell::exec", "arguments": {"description": "Upgrading the iii-sdk to 0.27 (token hunter2hunter2)", "command": BODY}}], "timestamp": *last2.lock().unwrap()}},
        ], "has_more": true}))
    });
    let q = w.question.clone();
    fake.on("harness::ask::list", move |_| {
        Ok(match q.lock().unwrap().clone() {
            Some(text) => json!({"entries": [{"session_id": "s-ours", "question_id": "q1", "questions": [{"header": "Pick", "question": text}]}]}),
            None => json!({"entries": []}),
        })
    });
}

/// A compose file with nothing for the agent to fix (a healthcheck), committed.
fn clean_manifest(ws: &std::path::Path) {
    std::fs::write(
        ws.join("worker-compose.yaml"),
        "containers: {}\ncloud:\n  healthcheck:\n    path: /health\n",
    )
    .unwrap();
    // The commit is local only, so the plan is "not pushed" and the lock is looked for here.
    std::fs::write(ws.join("worker-compose.lock"), "").unwrap();
    git(ws, &["add", "-A"]);
    git(ws, &["commit", "-m", "healthcheck"]);
}

/// The agent ran `check {run: true}` for the worktree's head and it passed.
fn seed_passed_run(h: &Harness, ws: &std::path::Path) {
    let head = git(ws, &["rev-parse", "HEAD"]);
    let (part, _) =
        iii_cloud::check::plan_part(&json!({"can_deploy": true, "fingerprint": "f", "plan": {}}));
    let run = Run {
        id: "r".into(),
        check_id: "agent-check".into(),
        commit: head.clone(),
        engine: "0.27.0".into(),
        status: RunStatus::Passed,
        started_at: "t".into(),
        finished_at: None,
        stages: vec![],
        workers: WorkerCount { ready: 1, total: 1 },
        routes: RouteCount {
            count: 1,
            prefix: None,
        },
        healthcheck: None,
        not_covered: vec![],
        log_tail: vec![],
        error: None,
        reused: false,
        waiting_on: None,
    };
    h.ctx.store.update(|s| {
        s.put_check_for(
            Check {
                id: "agent-check".into(),
                environment_id: ENV.into(),
                commit: head,
                base: String::new(),
                checked_at: "t".into(),
                duration_ms: 1,
                plan: part,
                here: HerePart {
                    problems: 0,
                    iii_version: None,
                    accepts_cloud_block: true,
                },
                problems: vec![],
                run: Some(run),
                ready: true,
                report: String::new(),
                manifest: None,
                handoff_id: None,
                commits: vec![],
            },
            false,
        )
    });
}

async fn until(h: &Harness, what: &str, f: impl Fn(&AgentState) -> bool) -> AgentState {
    for _ in 0..400 {
        let agent = iii_cloud::status::status(&h.ctx, StatusRequest::default())
            .await
            .unwrap()
            .agent;
        if let Some(a) = agent.filter(|a| f(a)) {
            return a;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!(
        "never reached {what}: {:?}",
        iii_cloud::status::status(&h.ctx, StatusRequest::default())
            .await
            .unwrap()
            .agent
    );
}

#[tokio::test]
async fn a_handoff_has_a_marker_and_a_new_one_replaces_the_open_one() {
    let (h, _s, _r, _ws) = setup().await;
    let a = create(&h.ctx, ready_req()).await.unwrap();
    assert!(a.marker.starts_with("iii-cloud hand-off ") && a.marker.ends_with(&a.id[..8]));
    let status = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap();
    let agent = status.agent.unwrap();
    assert_eq!(
        (agent.handoff_id.as_str(), agent.state, agent.session_id),
        (a.id.as_str(), AgentPhase::Starting, None)
    );
    // The same hand-off again (a double click) is the one already open.
    let twice = create(&h.ctx, ready_req()).await.unwrap();
    assert!(
        (twice.id.as_str(), twice.marker.as_str(), twice.reused)
            == (a.id.as_str(), a.marker.as_str(), true)
            && !a.reused
    );
    assert_eq!(h.ctx.store.read(|s| s.handoffs.len()), 1);
    // Another kind while one is starting is a follow-up for that one, not a second agent.
    let other_kind = HandoffRequest {
        check_id: Some("chk".into()),
        kind: HandoffKind::PullRequest,
        deployment_id: None,
        text: None,
        candidate: None,
    };
    let follow_up = create(&h.ctx, other_kind.clone()).await.unwrap();
    assert_eq!(
        (follow_up.id.as_str(), follow_up.reused),
        (a.id.as_str(), true)
    );
    assert_eq!(h.ctx.store.read(|s| s.handoffs.len()), 1);
    // Once that one is over, another kind replaces it.
    h.ctx
        .store
        .update(|s| s.handoffs[0].agent.state = AgentPhase::Done);
    let b = create(&h.ctx, other_kind).await.unwrap();
    assert_ne!(a.id, b.id);
    assert!(!b.reused);
    assert_eq!(
        h.ctx.store.read(|s| s.handoffs.len()),
        1,
        "one open hand-off per environment"
    );
    assert_eq!(
        iii_cloud::status::status(&h.ctx, StatusRequest::default())
            .await
            .unwrap()
            .agent
            .unwrap()
            .handoff_id,
        b.id
    );
    // Bad input.
    let bad = HandoffRequest {
        check_id: Some("nope".into()),
        kind: HandoffKind::Ready,
        deployment_id: None,
        text: None,
        candidate: None,
    };
    assert_eq!(
        create(&h.ctx, bad).await.unwrap_err().code,
        "INVALID_REQUEST"
    );
    let fix = HandoffRequest {
        check_id: Some("chk".into()),
        kind: HandoffKind::DeployFix,
        deployment_id: None,
        text: None,
        candidate: None,
    };
    assert_eq!(
        create(&h.ctx, fix).await.unwrap_err().code,
        "INVALID_REQUEST"
    );
}

#[tokio::test]
async fn the_worker_finds_the_session_by_its_marker_and_reports_only_descriptions() {
    let (h, _s, _r, ws) = setup().await;
    std::fs::write(ws.join(".env"), "TOKEN=hunter2hunter2\n").unwrap();
    let w = world();
    let created = create(&h.ctx, ready_req()).await.unwrap();
    install(&h.fake, &created.marker, &w, NOW_FN() + 1_000);
    let a = until(&h, "working", |a| {
        a.state == AgentPhase::Working && a.steps == 3
    })
    .await;
    assert_eq!(
        a.session_id.as_deref(),
        Some("s-ours"),
        "not the older session"
    );
    assert_eq!(
        a.current.as_deref(),
        Some("Upgrading the iii-sdk to 0.27 (token [redacted])")
    );
    assert!(a.last_activity_at.is_some() && a.question.is_none());
    // The worker never read more than it needed: the first user message, one at a time.
    assert!(
        h.fake
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(f, p)| f == "session::messages" && p["roles"] == json!(["user"]))
            .all(|(_, p)| p["limit"] == 1)
    );
    // No body, tool input or secret is in the state, the events or the status.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let state = std::fs::read_to_string(h.ctx.store.dir().join("state.json")).unwrap();
    let events = serde_json::to_string(&*h.fake.events.lock().unwrap()).unwrap();
    let status = serde_json::to_string(
        &iii_cloud::status::status(&h.ctx, StatusRequest::default())
            .await
            .unwrap(),
    )
    .unwrap();
    for text in [&state, &events, &status] {
        assert!(
            !text.contains("BODYTEXT") && !text.contains("hunter2hunter2"),
            "{text}"
        );
    }
    assert!(
        h.fake.kinds().iter().any(|k| k == "agent:false"),
        "{:?}",
        h.fake.kinds()
    );
}

#[tokio::test]
async fn a_question_for_the_person_is_waiting_for_you_and_quiet_is_stalled() {
    let (h, _s, _r, _ws) = setup().await;
    let w = world();
    let created = create(&h.ctx, ready_req()).await.unwrap();
    install(&h.fake, &created.marker, &w, NOW_FN() + 1_000);
    until(&h, "working", |a| a.state == AgentPhase::Working).await;
    *w.question.lock().unwrap() = Some("Which migration tool do you want me to keep?".into());
    let a = until(&h, "waiting_for_you", |a| {
        a.state == AgentPhase::WaitingForYou
    })
    .await;
    assert_eq!(
        a.question.as_deref(),
        Some("Which migration tool do you want me to keep?")
    );
    *w.question.lock().unwrap() = None;
    until(&h, "working again", |a| {
        a.state == AgentPhase::Working && a.question.is_none()
    })
    .await;
    // No new message for longer than the stall window (a test second is 10 ms: 1.8 s).
    *w.last_ms.lock().unwrap() = NOW_FN() - 3_600_000;
    until(&h, "stalled", |a| a.state == AgentPhase::Stalled).await;
    // It picks up again when a message comes in.
    *w.last_ms.lock().unwrap() = NOW_FN() + 60_000;
    until(&h, "working after the stall", |a| {
        a.state == AgentPhase::Working
    })
    .await;
}

#[tokio::test]
async fn when_the_agent_finishes_one_fast_check_runs_and_it_is_done() {
    let (h, _s, _r, ws) = setup().await;
    clean_manifest(&ws);
    seed_passed_run(&h, &ws);
    let w = world();
    let created = create(&h.ctx, ready_req()).await.unwrap();
    install(&h.fake, &created.marker, &w, NOW_FN() + 1_000);
    until(&h, "working", |a| a.state == AgentPhase::Working).await;
    let before = h.ctx.store.read(|s| s.check.as_ref().map(|c| c.id.clone()));
    *w.status.lock().unwrap() = "idle".into();
    let a = until(&h, "done", |a| a.state == AgentPhase::Done).await;
    assert_eq!(a.steps, 3);
    // The fast check (no run) came out of it, once.
    for _ in 0..200 {
        if h.ctx.store.read(|s| s.check.as_ref().map(|c| c.id.clone())) != before {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let (check, count) = h
        .ctx
        .store
        .read(|s| (s.check.clone().unwrap(), s.checks.len()));
    assert_ne!(Some(check.id.clone()), before);
    assert!(check.run.is_none(), "a fast check, no run");
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(h.ctx.store.read(|s| s.checks.len()), count, "only once");
    let kinds = h.fake.kinds();
    assert!(kinds.contains(&"agent:true".to_string()), "{kinds:?}");
}

#[tokio::test]
async fn stop_uses_the_harnesss_own_stop_and_marks_the_hand_off_stopped() {
    let (h, _s, _r, _ws) = setup().await;
    let w = world();
    let created = create(&h.ctx, ready_req()).await.unwrap();
    // No session yet: nothing to stop.
    assert_eq!(
        stop(
            &h.ctx,
            HandoffStopRequest {
                id: created.id.clone()
            }
        )
        .await
        .unwrap_err()
        .code,
        "INVALID_REQUEST"
    );
    assert_eq!(
        stop(&h.ctx, HandoffStopRequest { id: "nope".into() })
            .await
            .unwrap_err()
            .code,
        "INVALID_REQUEST"
    );
    install(&h.fake, &created.marker, &w, NOW_FN() + 1_000);
    h.fake.on("harness::stop", |req| {
        assert_eq!(req["session_id"], "s-ours");
        Ok(json!({"stopping": true}))
    });
    until(&h, "working", |a| a.state == AgentPhase::Working).await;
    let out = stop(
        &h.ctx,
        HandoffStopRequest {
            id: created.id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(out.stopping);
    assert_eq!(
        iii_cloud::status::status(&h.ctx, StatusRequest::default())
            .await
            .unwrap()
            .agent
            .unwrap()
            .state,
        AgentPhase::Stopped
    );
    assert!(
        h.fake
            .calls
            .lock()
            .unwrap()
            .iter()
            .any(|(f, _)| f == "harness::stop")
    );
    // A poll that lands after the stop does not bring it back to working.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        iii_cloud::status::status(&h.ctx, StatusRequest::default())
            .await
            .unwrap()
            .agent
            .unwrap()
            .state,
        AgentPhase::Stopped
    );
    assert!(h.fake.kinds().contains(&"agent:true".to_string()));
    let _: Option<Value> = None;
}

#[tokio::test]
async fn an_import_hand_off_needs_no_check_and_the_brief_has_what_the_agent_needs() {
    let (h, _s, repo, ws) = setup().await;
    // Import: no check_id; every other kind still needs one.
    let imp = create(
        &h.ctx,
        HandoffRequest {
            check_id: None,
            kind: HandoffKind::Import,
            deployment_id: None,
            text: None,
            candidate: None,
        },
    )
    .await
    .unwrap();
    assert!(!imp.id.is_empty());
    // The import is over: a new kind starts its own hand-off below.
    h.ctx
        .store
        .update(|s| s.handoffs[0].agent.state = AgentPhase::Done);
    assert_eq!(
        create(
            &h.ctx,
            HandoffRequest {
                check_id: None,
                kind: HandoffKind::Ready,
                deployment_id: None,
                text: None,
                candidate: None
            }
        )
        .await
        .unwrap_err()
        .code,
        "INVALID_REQUEST"
    );
    let status = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap();
    let brief = status.handoff_brief.unwrap();
    assert_eq!(
        (brief.kind, brief.check_id.clone()),
        (HandoffKind::Import, None)
    );
    assert!(brief.next_step.contains("project::resolve"));
    // A ready hand-off over a check with problems for both owners, and a secret in the .env.
    std::fs::write(ws.join(".env"), "TOKEN=hunter2hunter2\n").unwrap();
    let problem = |owner, title: &str, detail: &str| Problem {
        skipped: false,
        id: title.into(),
        source: Source::CheckHere,
        code: "X".into(),
        cloud_code: None,
        severity: Severity::Block,
        owner,
        title: title.into(),
        detail: Some(detail.into()),
        file: Some("a.yaml".into()),
        line: Some(3),
        excerpt: None,
        fix: Some("Do the thing.".into()),
    };
    h.ctx.store.update(|s| {
        let c = s.check.as_mut().unwrap();
        c.problems = vec![
            problem(Owner::Agent, "Mine", "uses hunter2hunter2"),
            problem(Owner::Person, "Yours", "pick one"),
            problem(Owner::None, "Info", "x"),
        ];
        let c = c.clone();
        s.put_check(c);
    });
    create(&h.ctx, ready_req()).await.unwrap();
    let brief = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .handoff_brief
        .unwrap();
    assert_eq!(brief.kind, HandoffKind::Ready);
    assert_eq!(brief.repository.as_deref(), Some("ada/hello-iii"));
    assert!(brief.project.as_deref().unwrap().ends_with("production"));
    let w = brief.worktree.clone().unwrap();
    assert_eq!(
        (w.path.as_str(), w.branch.as_str()),
        (ws.to_str().unwrap(), "iii-cloud/ready-production")
    );
    assert_eq!(
        std::path::PathBuf::from(brief.checkout_path.clone().unwrap()),
        repo.checkout
    );
    assert_eq!(
        (brief.check_id.as_deref(), brief.commit.as_deref()),
        (Some("chk"), Some("abc"))
    );
    assert_eq!(brief.agent_problems.len(), 1);
    assert_eq!(brief.person_problems.len(), 1);
    assert_eq!(
        brief.agent_problems[0].detail.as_deref(),
        Some("uses [redacted]"),
        "no secret value in the brief"
    );
    assert_eq!(
        (
            brief.agent_problems[0].file.as_deref(),
            brief.agent_problems[0].line,
            brief.agent_problems[0].fix.as_deref()
        ),
        (Some("a.yaml"), Some(3), Some("Do the thing."))
    );
    assert!(brief.next_step.contains("check {run: true}"));
    assert!(!serde_json::to_string(&brief).unwrap().contains("hunter2"));
    // deploy_fix: the failed deployment's stage, worker, code and log tail.
    let mut dep = iii_cloud::watch::to_deploy("http://x", (None, None, "production"), &json!({"id": "44444444-4444-4444-8444-444444444444", "number": 7, "environment_id": ENV, "type": "push", "status": "failed", "commit": {"sha": "abc", "branch": "main"}, "stages": [], "error": {"stage": "workers", "worker": "api", "code": "WORKERS_NOT_READY", "hint": "OOMKilled", "action": null}, "created_at": "2026-11-25T10:00:00Z"}), None, false).unwrap();
    dep.log_tail = vec!["Cannot find package 'express'".into()];
    h.ctx.store.update(|s| s.put_deploy(dep));
    h.ctx
        .store
        .update(|s| s.handoffs[0].agent.state = AgentPhase::Done);
    create(
        &h.ctx,
        HandoffRequest {
            check_id: Some("chk".into()),
            kind: HandoffKind::DeployFix,
            deployment_id: Some("44444444-4444-4444-8444-444444444444".into()),
            text: None,
            candidate: None,
        },
    )
    .await
    .unwrap();
    let d = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .handoff_brief
        .unwrap()
        .deploy
        .unwrap();
    assert_eq!(
        (
            d.stage.as_deref(),
            d.worker.as_deref(),
            d.code.as_deref(),
            d.hint.as_deref()
        ),
        (
            Some("workers"),
            Some("api"),
            Some("WORKERS_NOT_READY"),
            Some("OOMKilled")
        )
    );
    assert_eq!(d.log_tail, vec!["Cannot find package 'express'"]);
}

#[tokio::test]
async fn an_import_from_the_add_project_steps_briefs_the_folder_being_added_and_only_then() {
    let other_repo = repo();
    let (h, _s, _selected_repo, _ws) = setup().await;
    resolve(
        &h.ctx,
        ResolveRequest {
            path: other_repo.checkout.to_string_lossy().into(),
            team_id: None,
            cloud_project_id: None,
        },
    )
    .await
    .unwrap();
    let added = h.ctx.store.read(|s| s.candidate.clone()).unwrap();
    let selected = h.ctx.store.read(|s| s.project.clone()).unwrap();
    let import = |candidate| HandoffRequest {
        check_id: None,
        kind: HandoffKind::Import,
        deployment_id: None,
        text: None,
        candidate,
    };
    let brief_path = || async {
        iii_cloud::status::status(&h.ctx, StatusRequest::default())
            .await
            .unwrap()
            .handoff_brief
            .unwrap()
            .checkout_path
    };
    // Sent from the add-project steps: the folder being added.
    create(&h.ctx, import(Some(true))).await.unwrap();
    assert_eq!(brief_path().await, Some(added.path.clone()));
    // Sent from anywhere else, a candidate left from a cancelled add is not used.
    h.ctx
        .store
        .update(|s| s.handoffs[0].agent.state = AgentPhase::Done);
    create(&h.ctx, import(None)).await.unwrap();
    assert_eq!(brief_path().await, Some(selected.path.clone()));
    // Any other kind is of the selected project.
    h.ctx
        .store
        .update(|s| s.handoffs[0].agent.state = AgentPhase::Done);
    create(&h.ctx, ready_req()).await.unwrap();
    assert_eq!(brief_path().await, Some(selected.path));
}

#[tokio::test]
async fn the_brief_names_the_environments_compose_file_and_the_others() {
    let (h, _s, _r, ws) = setup().await;
    // The registry's layout: a dev file and a production file; plus one that is not a compose file.
    std::fs::create_dir_all(ws.join("api")).unwrap();
    std::fs::write(ws.join("api/worker-compose.yaml"), "containers:\n  a: {}\n").unwrap();
    std::fs::write(
        ws.join("api/worker-compose-prod.yaml"),
        "containers:\n  a: {}\n",
    )
    .unwrap();
    std::fs::write(ws.join("api/docker-compose.yml"), "services:\n  db: {}\n").unwrap();
    for i in 0..12 {
        std::fs::write(
            ws.join(format!("extra-compose-{i}.yaml")),
            "containers: {}\n",
        )
        .unwrap();
    }
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-m", "composes"]);
    h.ctx.store.update(|s| {
        s.project
            .as_mut()
            .unwrap()
            .environment
            .as_mut()
            .unwrap()
            .manifest = Some("api/worker-compose-prod.yaml".into())
    });
    create(&h.ctx, ready_req()).await.unwrap();
    let brief = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .handoff_brief
        .unwrap();
    assert_eq!(brief.manifest, "api/worker-compose-prod.yaml");
    assert!(
        brief
            .other_compose_files
            .contains(&"api/worker-compose.yaml".to_string())
    );
    assert!(
        !brief
            .other_compose_files
            .contains(&"api/worker-compose-prod.yaml".to_string()),
        "its own file is not an other"
    );
    assert!(
        !brief
            .other_compose_files
            .iter()
            .any(|f| f.contains("docker-compose")),
        "no `containers:` in it"
    );
    assert_eq!(brief.other_compose_files.len(), 10, "capped");
    assert!(
        brief
            .next_step
            .ends_with("Only api/worker-compose-prod.yaml is checked and deployed."),
        "{}",
        brief.next_step
    );
    // No manifest on the environment: the root file.
    h.ctx.store.update(|s| {
        s.project
            .as_mut()
            .unwrap()
            .environment
            .as_mut()
            .unwrap()
            .manifest = None
    });
    let brief = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .handoff_brief
        .unwrap();
    assert_eq!(brief.manifest, "worker-compose.yaml");
}

fn propose_tail(user_after: bool) -> Vec<Value> {
    let mut items = vec![
        json!({"entry_id": "t1", "message": {"role": "assistant", "content": [{"type": "function_call", "id": "c9", "function_id": "ade::working-directory::propose", "arguments": {"description": "Move the chat into the worktree"}}], "timestamp": 10}}),
        // The result names the function only through its call id, and says so in its text.
        json!({"entry_id": "t2", "message": {"role": "function_result", "function_call_id": "c9", "content": [{"type": "text", "text": "{\"requires_confirmation\": true, \"path\": \"/w\"}"}], "details": null, "is_error": false, "timestamp": 11}}),
        json!({"entry_id": "t3", "message": {"role": "assistant", "content": [{"type": "text", "text": "Please confirm the folder."}], "timestamp": 12}}),
    ];
    if user_after {
        items.push(json!({"entry_id": "t4", "message": {"role": "user", "content": [{"type": "text", "text": "Confirmed."}], "timestamp": 13}}));
    }
    items
}

#[test]
fn a_proposal_waiting_for_confirmation_is_found_in_the_tail() {
    use iii_cloud::handoff::awaits_confirmation;
    assert!(awaits_confirmation(&propose_tail(false)));
    assert!(
        !awaits_confirmation(&propose_tail(true)),
        "the person wrote since"
    );
    // Confirmed already: the result says it does not require confirmation.
    let mut done = propose_tail(false);
    done[1] = json!({"entry_id": "t2", "message": {"role": "function_result", "function_id": "ade::working-directory::propose", "content": [], "details": {"requires_confirmation": false}, "timestamp": 11}});
    assert!(!awaits_confirmation(&done));
    // Another function's result with the same words is not it.
    let mut other = propose_tail(false);
    other[0]["message"]["content"][0]["function_id"] = json!("shell::exec");
    assert!(!awaits_confirmation(&other));
}

#[tokio::test]
async fn a_turn_that_ends_on_a_folder_proposal_is_waiting_for_the_person_not_done() {
    let (h, _s, _r, ws) = setup().await;
    clean_manifest(&ws);
    seed_passed_run(&h, &ws);
    let w = world();
    let created = create(&h.ctx, ready_req()).await.unwrap();
    install(&h.fake, &created.marker, &w, NOW_FN() + 1_000);
    until(&h, "working", |a| a.state == AgentPhase::Working).await;
    *w.tail.lock().unwrap() = Some(propose_tail(false));
    *w.status.lock().unwrap() = "idle".into();
    let a = until(&h, "waiting for the confirmation", |a| {
        a.state == AgentPhase::WaitingForYou
    })
    .await;
    assert_eq!(
        a.question.as_deref(),
        Some("Confirm the worktree folder in the chat (Use for chat), then the agent continues.")
    );
    assert!(
        h.fake.kinds().iter().all(|k| k != "agent:true"),
        "not a terminal event: {:?}",
        h.fake.kinds()
    );
    // It keeps following: the person confirms, the agent has nothing left to fix, and it is done.
    *w.tail.lock().unwrap() = Some(propose_tail(true));
    let a = until(&h, "done after the confirmation", |a| {
        a.state == AgentPhase::Done
    })
    .await;
    assert_eq!(a.question, None);
}

/// The messages the worker posted into the chat with `harness::send`.
fn sent(h: &Harness) -> Vec<String> {
    h.fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|(f, _)| f == "harness::send")
        .map(|(_, p)| p["message"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// The chat accepts what the worker sends.
fn accept_sends(h: &Harness) {
    h.fake.on("harness::send", |_| {
        Ok(json!({"session_id": "s-ours", "turn_id": "t"}))
    });
}

async fn idle_for(w: &World, ms: u64) {
    *w.status.lock().unwrap() = "idle".into();
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test]
async fn a_turn_that_ends_with_problems_left_gets_one_continue_per_idle_stretch_then_stalls() {
    let (h, _s, _r, _ws) = setup().await;
    accept_sends(&h);
    // The repo has no healthcheck: a problem that is the agent's.
    let w = world();
    let created = create(&h.ctx, ready_req()).await.unwrap();
    install(&h.fake, &created.marker, &w, NOW_FN() + 1_000);
    until(&h, "working", |a| a.state == AgentPhase::Working).await;
    let id8 = &created.id[..8];
    for round in 1..=3usize {
        *w.status.lock().unwrap() = "idle".into();
        for _ in 0..200 {
            if sent(&h).len() >= round {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let checks = h.ctx.store.read(|s| s.checks.len());
        // The same idle stretch, however long it lasts: no second message, no second check.
        tokio::time::sleep(Duration::from_millis(500)).await;
        assert_eq!(sent(&h).len(), round, "one continue per idle stretch");
        assert_eq!(h.ctx.store.read(|s| s.checks.len()), checks);
        let state = iii_cloud::status::status(&h.ctx, StatusRequest::default())
            .await
            .unwrap()
            .agent
            .unwrap();
        assert_eq!(
            state.state,
            AgentPhase::Working,
            "the agent is told to go on"
        );
        *w.status.lock().unwrap() = "working".into();
        *w.last_ms.lock().unwrap() = NOW_FN();
        until(&h, "working again", |a| a.state == AgentPhase::Working).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let messages = sent(&h);
    let lines: Vec<&str> = messages[0].lines().collect();
    assert_eq!(lines[0], format!("iii-cloud hand-off {id8} · continue"));
    assert_eq!(
        *lines.last().unwrap(),
        "Keep going until the check is ready or you need the person."
    );
    assert!(
        lines.len() >= 3 && lines[1].contains(": "),
        "one `<CODE>: <title>` line per problem: {lines:?}"
    );
    // The fourth idle stretch: no more messages, and the state is stalled.
    idle_for(&w, 0).await;
    let a = until(&h, "stalled", |a| {
        a.state == AgentPhase::Stalled && a.question.is_some()
    })
    .await;
    assert_eq!(
        a.question.as_deref(),
        Some("Your agent stopped; open the chat to see what it needs.")
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(sent(&h).len(), 3, "at most three");
    assert_eq!(h.ctx.store.read(|s| s.handoffs[0].continues), 3);
}

#[tokio::test]
async fn no_continue_while_the_agent_works_waits_on_a_wake_or_asks_or_after_a_stop() {
    let (h, _s, _r, _ws) = setup().await;
    accept_sends(&h);
    h.fake
        .on("harness::stop", |_| Ok(json!({"stopping": true})));
    let w = world();
    let created = create(&h.ctx, ready_req()).await.unwrap();
    install(&h.fake, &created.marker, &w, NOW_FN() + 1_000);
    // Still working.
    until(&h, "working", |a| a.state == AgentPhase::Working).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(sent(&h).is_empty(), "the agent is working");
    // A question is open.
    *w.question.lock().unwrap() = Some("Which one?".into());
    idle_for(&w, 0).await;
    until(&h, "asked", |a| a.state == AgentPhase::WaitingForYou).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(sent(&h).is_empty(), "an ask is open");
    *w.question.lock().unwrap() = None;
    // A wake is armed.
    let rows = Arc::new(Mutex::new(vec![wake_row(0, NOW_FN() + 3_600_000)]));
    let r = rows.clone();
    h.fake.on("harness::observers::list", move |_| {
        Ok(json!({"observers": r.lock().unwrap().clone()}))
    });
    *w.status.lock().unwrap() = "working".into();
    until(&h, "working", |a| a.state == AgentPhase::Working).await;
    idle_for(&w, 0).await;
    until(&h, "waiting", |a| a.state == AgentPhase::Waiting).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(sent(&h).is_empty(), "a wake is armed");
    // The person stopped it.
    *rows.lock().unwrap() = vec![];
    *w.status.lock().unwrap() = "working".into();
    until(&h, "working", |a| a.state == AgentPhase::Working).await;
    stop(&h.ctx, HandoffStopRequest { id: created.id })
        .await
        .unwrap();
    idle_for(&w, 400).await;
    assert!(sent(&h).is_empty(), "the hand-off was stopped");
}

#[tokio::test]
async fn only_problems_for_the_person_left_waits_for_you_and_names_them() {
    let (h, _s, _r, ws) = setup().await;
    accept_sends(&h);
    // A name with no value anywhere is the person's; nothing is the agent's.
    std::fs::write(
        ws.join("worker-compose.yaml"),
        "containers:\n  api:\n    worker: package://http\n    environment:\n      NO_SOURCE: ${NO_SOURCE}\ncloud:\n  healthcheck:\n    path: /health\n",
    )
    .unwrap();
    std::fs::write(ws.join("worker-compose.lock"), "").unwrap();
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-m", "a name"]);
    let w = world();
    let created = create(&h.ctx, ready_req()).await.unwrap();
    install(&h.fake, &created.marker, &w, NOW_FN() + 1_000);
    until(&h, "working", |a| a.state == AgentPhase::Working).await;
    idle_for(&w, 0).await;
    let a = until(&h, "waiting for you", |a| {
        a.state == AgentPhase::WaitingForYou
    })
    .await;
    let theirs = h.ctx.store.read(|s| {
        s.check
            .as_ref()
            .unwrap()
            .problems
            .iter()
            .filter(|p| p.owner == Owner::Person && p.severity != Severity::Info)
            .map(|p| format!("{}: {}", p.code, p.title))
            .collect::<Vec<_>>()
            .join("\n")
    });
    assert!(theirs.starts_with("LOCAL_VARIABLE_UNSET: "), "{theirs}");
    assert_eq!(a.question.as_deref(), Some(theirs.as_str()));
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(sent(&h).is_empty(), "nothing for the agent to continue on");
}

#[tokio::test]
async fn the_brief_function_answers_the_brief_alone_by_short_or_full_id_with_the_failed_run() {
    use iii_cloud::handoff::brief_for;
    let (h, _s, _r, _ws) = setup().await;
    let created = create(&h.ctx, ready_req()).await.unwrap();
    let by = |id: &str| HandoffBriefRequest {
        handoff_id: id.into(),
    };
    let short = brief_for(&h.ctx, by(&created.id[..8])).await.unwrap();
    let full = brief_for(&h.ctx, by(&created.id)).await.unwrap();
    assert_eq!(short, full);
    assert_eq!(short.brief.handoff_id, created.id);
    assert_eq!(short.run_failure, None);
    let v = serde_json::to_value(&short).unwrap();
    assert!(v.get("run_failure").is_some_and(Value::is_null));
    for absent in ["checks", "deploys", "variables", "connection", "agent"] {
        assert!(v.get(absent).is_none(), "{absent} is not part of the brief");
    }
    assert!(v["next_step"].is_string() && v["manifest"].is_string());
    assert_eq!(
        brief_for(&h.ctx, by("nope")).await.unwrap_err().code,
        "INVALID_REQUEST"
    );
    // A failed run: its stage, code, message and the last lines.
    h.ctx.store.update(|s| {
        let mut c = s.check.clone().unwrap();
        c.run = Some(Run {
            id: "r".into(),
            check_id: c.id.clone(),
            commit: "abc".into(),
            engine: "0.27.0".into(),
            status: RunStatus::Failed,
            started_at: "t".into(),
            finished_at: None,
            stages: vec![],
            workers: WorkerCount { ready: 0, total: 1 },
            routes: RouteCount {
                count: 0,
                prefix: None,
            },
            healthcheck: None,
            not_covered: vec![],
            log_tail: (0..60).map(|i| format!("line {i}")).collect(),
            error: Some(RunError {
                stage: StageName::Workers,
                code: "WORKERS_NOT_READY".into(),
                message: "api did not connect".into(),
            }),
            reused: false,
            waiting_on: None,
        });
        s.put_check(c);
    });
    let failed = brief_for(&h.ctx, by(&created.id[..8])).await.unwrap();
    let f = failed.run_failure.unwrap();
    assert_eq!(
        (f.stage, f.code.as_str(), f.message.as_str()),
        (
            StageName::Workers,
            "WORKERS_NOT_READY",
            "api did not connect"
        )
    );
    assert_eq!(f.log_tail.len(), 40);
    assert_eq!(f.log_tail.last().map(String::as_str), Some("line 59"));
}

#[tokio::test]
async fn progress_and_check_take_the_short_id_from_the_marker() {
    let (h, _s, _r, _ws) = setup().await;
    let created = create(&h.ctx, ready_req()).await.unwrap();
    let out = iii_cloud::handoff::progress(
        &h.ctx,
        HandoffProgressRequest {
            handoff_id: Some(created.id[..8].to_string()),
            working_on: vec![],
            note: Some("starting".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(out.note.as_deref(), Some("starting"));
}

#[tokio::test]
async fn auto_fix_is_on_by_default_and_the_status_follows_the_config() {
    let (h, _s, _r, _ws) = setup().await;
    let status = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap();
    assert!(status.auto_fix, "on by default");
    let cfg = Config {
        auto_fix: false,
        ..(**h.ctx.cfg.read().await).clone()
    };
    iii_cloud::configuration::apply_config(&h.ctx.cfg, cfg).await;
    let status = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap();
    assert!(!status.auto_fix);
}

fn prob(id: &str, code: &str) -> Problem {
    Problem {
        skipped: false,
        id: id.into(),
        source: Source::CheckHere,
        code: code.into(),
        cloud_code: None,
        severity: Severity::Block,
        owner: Owner::Agent,
        title: id.into(),
        detail: None,
        file: None,
        line: None,
        excerpt: None,
        fix: None,
    }
}

fn set_problems(h: &Harness, problems: Vec<Problem>) {
    h.ctx.store.update(|s| {
        let mut c = s.check.clone().unwrap();
        c.problems = problems;
        s.put_check(c);
    });
}

#[tokio::test]
async fn the_agent_says_what_it_works_on_and_it_clears_when_the_problems_go() {
    use iii_cloud::handoff::progress;
    let (h, _s, _r, ws) = setup().await;
    std::fs::write(ws.join(".env"), "TOKEN=hunter2hunter2\n").unwrap();
    set_problems(
        &h,
        vec![
            prob("p1", "LOCK_MISSING"),
            prob("p2", "LOCK_MISSING"),
            prob("p3", "HEALTHCHECK_MISSING"),
        ],
    );
    // No open hand-off: nothing to report progress on.
    let say = |names: &[&str], note: Option<&str>| HandoffProgressRequest {
        handoff_id: None,
        working_on: names.iter().map(|n| n.to_string()).collect(),
        note: note.map(str::to_owned),
    };
    assert_eq!(
        progress(&h.ctx, say(&["p1"], None)).await.unwrap_err().code,
        "INVALID_REQUEST"
    );
    create(&h.ctx, ready_req()).await.unwrap();
    // A code stands for every open problem of it; an id for itself; an unknown name for nothing.
    let long = format!("Fixing the lock with hunter2hunter2 {}", "x".repeat(200));
    let out = progress(&h.ctx, say(&["LOCK_MISSING", "p3", "NOPE"], Some(&long)))
        .await
        .unwrap();
    assert_eq!(out.working_on, vec!["p1", "p2", "p3"]);
    let note = out.note.unwrap();
    assert!(
        note.chars().count() <= 123 && note.starts_with("Fixing the lock with [redacted]"),
        "{note}"
    );
    let agent = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .agent
        .unwrap();
    assert_eq!(agent.working_on, vec!["p1", "p2", "p3"]);
    assert_eq!(agent.note.as_deref(), Some(note.as_str()));
    assert!(
        h.fake
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.kind == EventKind::Agent
                && e.record["working_on"] == json!(["p1", "p2", "p3"])),
        "the page is told"
    );
    // A newer check without p1 and p3: only p2 is still being worked on.
    set_problems(&h, vec![prob("p2", "LOCK_MISSING")]);
    let agent = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .agent
        .unwrap();
    assert_eq!(agent.working_on, vec!["p2"]);
    // The follower persists that too.
    let w = world();
    let id = h.ctx.store.read(|s| s.handoffs[0].id.clone());
    let marker = h.ctx.store.read(|s| s.handoffs[0].marker.clone());
    install(&h.fake, &marker, &w, NOW_FN() + 1_000);
    until(&h, "working", |a| a.state == AgentPhase::Working).await;
    assert_eq!(
        h.ctx.store.read(|s| s.handoffs[0].agent.working_on.clone()),
        vec!["p2"]
    );
    // The progress call survives a poll; the end of the hand-off clears it.
    assert_eq!(
        h.ctx
            .store
            .read(|s| s.handoffs[0].agent.note.clone())
            .as_deref(),
        Some(note.as_str())
    );
    h.fake
        .on("harness::stop", |_| Ok(json!({"stopping": true})));
    stop(&h.ctx, HandoffStopRequest { id }).await.unwrap();
    let agent = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .agent
        .unwrap();
    assert_eq!(
        (agent.state, agent.working_on.is_empty(), agent.note),
        (AgentPhase::Stopped, true, None)
    );
}

#[tokio::test]
async fn a_hand_off_that_finds_no_chat_in_90_seconds_waits_for_you_and_still_picks_one_up() {
    let (h, _s, _r, _ws) = setup().await;
    let created = create(&h.ctx, ready_req()).await.unwrap();
    // No session ever shows up (a test second is 10 ms: 90 s is 0.9 s).
    let a = until(&h, "waiting for you", |a| {
        a.state == AgentPhase::WaitingForYou
    })
    .await;
    assert_eq!(
        a.question.as_deref(),
        Some("The chat didn't start. Open the chat and send the message, or try again.")
    );
    assert_eq!((a.session_id, a.kind), (None, Some(HandoffKind::Ready)));
    // The person sends the message after all: the worker finds the chat and follows it.
    let w = world();
    install(&h.fake, &created.marker, &w, NOW_FN() + 1_000);
    until(&h, "working", |a| {
        a.state == AgentPhase::Working && a.session_id.as_deref() == Some("s-ours")
    })
    .await;
}

#[tokio::test]
async fn a_follow_up_is_sent_into_the_existing_chat_like_the_console_would() {
    let (h, _s, _r, _ws) = setup().await;
    let w = world();
    let first = create(&h.ctx, ready_req()).await.unwrap();
    assert!(
        !first.delivered && first.session_id.is_none(),
        "no chat yet: nothing to send into"
    );
    install(&h.fake, &first.marker, &w, NOW_FN() + 1_000);
    until(&h, "working", |a| a.session_id.as_deref() == Some("s-ours")).await;
    // The chat's own model and folder come from its metadata.
    let (status, last) = (w.status.clone(), w.last_ms.clone());
    h.fake.on("session::get", move |_| Ok(json!({"meta": {"session_id": "s-ours", "status": status.lock().unwrap().clone(), "updated_at": *last.lock().unwrap(), "metadata": {"model": "anthropic::claude-x", "fs_scope": {"root": "/work/tree"}}}})));
    h.fake.on("harness::send", |_| {
        Ok(json!({"session_id": "s-ours", "turn_id": "t"}))
    });
    *w.status.lock().unwrap() = "idle".into();
    *w.last_ms.lock().unwrap() = NOW_FN() - 3_600_000;
    let req = HandoffRequest {
        check_id: Some("chk".into()),
        kind: HandoffKind::PullRequest,
        deployment_id: None,
        text: Some("Please open the pull request.".into()),
        candidate: None,
    };
    // The first one is over; the follow-up goes into its chat.
    h.ctx
        .store
        .update(|s| s.handoffs[0].agent.state = AgentPhase::Done);
    let out = create(&h.ctx, req).await.unwrap();
    assert!(out.delivered);
    assert_eq!(out.session_id.as_deref(), Some("s-ours"));
    let sent = h
        .fake
        .calls
        .lock()
        .unwrap()
        .iter()
        .find(|(f, _)| f == "harness::send")
        .map(|(_, p)| p.clone())
        .unwrap();
    assert_eq!(sent["session_id"], "s-ours");
    assert_eq!(
        sent["message"],
        format!("{}\nPlease open the pull request.", out.marker)
    );
    assert_eq!(
        (sent["model"].as_str(), sent["provider"].as_str()),
        (Some("claude-x"), Some("anthropic"))
    );
    assert_eq!(
        sent["options"]["functions"],
        json!({"allow": ["*"], "deny": ["approval::*", "configuration::register", "configuration::ensure", "shell::workspace::*"], "expose": "agent_call"})
    );
    assert_eq!(
        sent["options"]["metadata"]["fs_scope"]["root"],
        "/work/tree"
    );
    assert!(
        sent["options"].get("agent").is_none(),
        "never an agent on an existing session"
    );
    assert!(
        sent["idempotency_key"]
            .as_str()
            .unwrap()
            .starts_with("iii-cloud-handoff-")
    );
    // The old chat is idle and quiet from before: that is not "done", the turn has not begun.
    let a = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .agent
        .unwrap();
    assert_eq!(
        (a.kind, a.session_id.as_deref()),
        (Some(HandoffKind::PullRequest), Some("s-ours"))
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let a = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .agent
        .unwrap();
    assert_eq!(a.state, AgentPhase::Starting);
    assert_eq!(
        a.current, None,
        "the old chat's last call is not what the agent does now"
    );
    // The turn begins: it is followed.
    *w.status.lock().unwrap() = "working".into();
    *w.last_ms.lock().unwrap() = NOW_FN() + 5_000;
    until(&h, "working", |a| a.state == AgentPhase::Working).await;
    // The chat is gone, or the send fails: not delivered, the page starts a new chat.
    h.fake.on("session::get", |_| Ok(json!({"meta": null})));
    let again = HandoffRequest {
        check_id: Some("chk".into()),
        kind: HandoffKind::DeployFix,
        deployment_id: Some("d1".into()),
        text: Some("Fix it.".into()),
        candidate: None,
    };
    h.ctx
        .store
        .update(|s| s.handoffs[0].agent.state = AgentPhase::Done);
    let out = create(&h.ctx, again).await.unwrap();
    assert!(!out.delivered && out.session_id.is_none());
}

#[tokio::test]
async fn a_double_click_is_one_hand_off_until_it_ends() {
    let (h, _s, _r, _ws) = setup().await;
    let w = world();
    let a = create(&h.ctx, ready_req()).await.unwrap();
    install(&h.fake, &a.marker, &w, NOW_FN() + 1_000);
    until(&h, "working", |x| x.state == AgentPhase::Working).await;
    // Working: still the same one, with its session.
    let again = create(&h.ctx, ready_req()).await.unwrap();
    assert_eq!(
        (
            again.id.as_str(),
            again.reused,
            again.session_id.as_deref(),
            again.delivered
        ),
        (a.id.as_str(), true, Some("s-ours"), true)
    );
    // Another kind while it works goes into the same agent as a follow-up.
    let fix = HandoffRequest {
        check_id: Some("chk".into()),
        kind: HandoffKind::DeployFix,
        deployment_id: Some("d1".into()),
        text: None,
        candidate: None,
    };
    let followed = create(&h.ctx, fix.clone()).await.unwrap();
    assert_eq!(
        (followed.id.as_str(), followed.reused),
        (a.id.as_str(), true)
    );
    // A different deployment is a different hand-off once the first is over.
    h.ctx
        .store
        .update(|s| s.handoffs[0].agent.state = AgentPhase::Done);
    let b = create(&h.ctx, fix.clone()).await.unwrap();
    assert_ne!(b.id, a.id);
    h.ctx
        .store
        .update(|s| s.handoffs[0].agent.state = AgentPhase::Done);
    let other = HandoffRequest {
        deployment_id: Some("d2".into()),
        ..fix
    };
    assert_ne!(create(&h.ctx, other).await.unwrap().id, b.id);
    // Once it is over, the same request starts a fresh one.
    h.fake
        .on("harness::stop", |_| Ok(json!({"stopping": true})));
    let id = h.ctx.store.read(|s| s.handoffs[0].id.clone());
    let _ = stop(&h.ctx, HandoffStopRequest { id }).await;
    h.ctx
        .store
        .update(|s| s.handoffs[0].agent.state = AgentPhase::Done);
    let fresh = create(
        &h.ctx,
        h.ctx.store.read(|s| {
            let x = &s.handoffs[0];
            HandoffRequest {
                check_id: x.check_id.clone(),
                kind: x.kind,
                deployment_id: x.deployment_id.clone(),
                text: None,
                candidate: None,
            }
        }),
    )
    .await
    .unwrap();
    assert!(!fresh.reused);
}

#[tokio::test]
async fn a_hand_off_with_no_chat_is_reused_for_90_seconds_then_a_new_one_starts() {
    let (h, _s, _r, _ws) = setup().await;
    let a = create(&h.ctx, ready_req()).await.unwrap();
    // Waiting with no chat, but young: still the same click.
    h.ctx
        .store
        .update(|s| s.handoffs[0].agent.state = AgentPhase::WaitingForYou);
    assert!(create(&h.ctx, ready_req()).await.unwrap().reused);
    // Old enough (a test second is 10 ms: 90 s is 0.9 s) and still no chat: a new try.
    tokio::time::sleep(Duration::from_millis(1_000)).await;
    h.ctx
        .store
        .update(|s| s.handoffs[0].agent.state = AgentPhase::WaitingForYou);
    let b = create(&h.ctx, ready_req()).await.unwrap();
    assert!(!b.reused && b.id != a.id);
}

#[tokio::test]
async fn the_brief_says_when_an_earlier_try_left_the_fix_branch_on_origin() {
    let (h, _s, repo, ws) = setup().await;
    create(&h.ctx, ready_req()).await.unwrap();
    let note = |h: &Harness| h.ctx.store.read(|s| s.handoffs[0].clone());
    let brief =
        |h: &Harness| iii_cloud::handoff::brief(&h.ctx.store.read(|s| s.clone()), &note(h), &[]);
    assert_eq!(brief(&h).remote_branch_note, None, "nothing on origin");
    // An earlier try's branch on origin, with a commit this worktree does not have.
    let t = tempfile::tempdir().unwrap();
    git(
        t.path(),
        &["clone", "-q", repo.origin.to_str().unwrap(), "c"],
    );
    let c = t.path().join("c");
    git(&c, &["checkout", "-b", "iii-cloud/ready-production"]);
    std::fs::write(c.join("earlier.txt"), "e").unwrap();
    git(&c, &["add", "."]);
    git(&c, &["commit", "-m", "earlier try"]);
    git(&c, &["push", "-q", "origin", "iii-cloud/ready-production"]);
    git(&ws, &["fetch", "origin"]);
    assert_eq!(
        brief(&h).remote_branch_note.as_deref(),
        Some(
            "an earlier try left origin/iii-cloud/ready-production; your first push replaces it with --force-with-lease"
        )
    );
    // When the remote branch is part of this worktree's own history, it is this try's: no note.
    git(
        &ws,
        &[
            "merge",
            "-q",
            "--no-edit",
            "origin/iii-cloud/ready-production",
        ],
    );
    assert_eq!(brief(&h).remote_branch_note, None);
}

#[tokio::test]
async fn done_is_provisional_the_chat_is_followed_on_and_a_pending_ask_wins() {
    let (h, _s, _r, ws) = setup().await;
    clean_manifest(&ws);
    seed_passed_run(&h, &ws);
    let w = world();
    let created = create(&h.ctx, ready_req()).await.unwrap();
    install(&h.fake, &created.marker, &w, NOW_FN() + 1_000);
    let a = until(&h, "working", |a| {
        a.state == AgentPhase::Working && a.current.is_some()
    })
    .await;
    let first_activity = a.last_activity_at.clone().unwrap();
    *w.status.lock().unwrap() = "idle".into();
    let a = until(&h, "done", |a| a.state == AgentPhase::Done).await;
    assert_eq!(a.current, None, "nothing is being done once it is done");
    // The agent's question registers after the turn ended: waiting for you, with the ask's text.
    *w.question.lock().unwrap() = Some("Which database should I use?".into());
    let a = until(&h, "waiting for you", |a| {
        a.state == AgentPhase::WaitingForYou
    })
    .await;
    assert_eq!(a.question.as_deref(), Some("Which database should I use?"));
    assert_eq!(a.session_id.as_deref(), Some("s-ours"));
    // Answered; the person later starts a run from the chat: the hand-off follows it, not "finished".
    *w.question.lock().unwrap() = None;
    until(&h, "done again", |a| {
        a.state == AgentPhase::Done && a.question.is_none()
    })
    .await;
    *w.status.lock().unwrap() = "working".into();
    *w.last_ms.lock().unwrap() = NOW_FN() + 30_000;
    let a = until(&h, "working again", |a| a.state == AgentPhase::Working).await;
    assert!(a.current.is_some(), "the new call is shown");
    // last_activity_at follows the newest message.
    assert!(a.last_activity_at.unwrap() > first_activity);
}

#[tokio::test]
async fn agent_updates_are_events_of_kind_agent_with_the_status_record() {
    let (h, _s, _r, _ws) = setup().await;
    let a = create(&h.ctx, ready_req()).await.unwrap();
    let events: Vec<ChangeEvent> = h
        .fake
        .events
        .lock()
        .unwrap()
        .iter()
        .filter(|e| e.kind == EventKind::Agent)
        .cloned()
        .collect();
    let e = events.first().expect("an agent event");
    // The page applies `record` as `status.agent`, by `id`.
    assert_eq!(e.id, a.id);
    assert_eq!(
        (
            e.record["handoff_id"].as_str(),
            e.record["state"].as_str(),
            e.record["kind"].as_str()
        ),
        (Some(a.id.as_str()), Some("starting"), Some("ready"))
    );
    assert!(!e.terminal);
}

#[tokio::test]
async fn a_hand_off_that_never_finds_its_chat_is_stalled_after_30_minutes_not_dropped() {
    let (h, _s, _r, _ws) = setup().await;
    let created = create(&h.ctx, ready_req()).await.unwrap();
    // A test second is 10 ms: 30 minutes is 18 s.
    let mut a = None;
    for _ in 0..1_000 {
        a = iii_cloud::status::status(&h.ctx, StatusRequest::default())
            .await
            .unwrap()
            .agent;
        if a.as_ref().is_some_and(|a| a.state == AgentPhase::Stalled) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let a = a.unwrap();
    assert_eq!(a.state, AgentPhase::Stalled);
    assert!(a.question.unwrap().starts_with("No progress for 30 min"));
    assert_eq!(a.handoff_id, created.id);
    assert!(
        h.fake
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.kind == EventKind::Agent && e.record["state"] == "stalled")
    );
}

#[tokio::test]
async fn done_also_needs_a_run_here_that_passed_for_the_commit() {
    let (h, _s, _r, ws) = setup().await;
    // Nothing left to fix, but the agent never ran it here like the cloud.
    clean_manifest(&ws);
    let w = world();
    let created = create(&h.ctx, ready_req()).await.unwrap();
    install(&h.fake, &created.marker, &w, NOW_FN() + 1_000);
    until(&h, "working", |a| a.state == AgentPhase::Working).await;
    *w.status.lock().unwrap() = "idle".into();
    let a = until(&h, "waiting", |a| a.state == AgentPhase::WaitingForYou).await;
    assert_eq!(
        a.question.as_deref(),
        Some("Your agent stopped before running it here like the cloud.")
    );
    // The run passes for that commit: done.
    seed_passed_run(&h, &ws);
    until(&h, "done", |a| a.state == AgentPhase::Done).await;
}

/// An observer row as `harness::observers::list` returns it (live shape): a
/// wake on the run here ending, no `target`.
fn wake_row(fires: u64, expires_at: i64) -> Value {
    json!({
        "observer_id": "sub_db901f",
        "function_id": "iii-cloud::on-change",
        "namespace": "my-project",
        "filters": {"input.kind": {"$eq": "run"}, "input.id": {"$eq": "0c87581a"}, "input.terminal": {"$eq": true}},
        "once": true,
        "fires": fires,
        "expires_at": expires_at,
        "created_at": 1
    })
}

#[tokio::test]
async fn a_turn_that_ended_on_an_armed_wake_waits_for_the_event_not_the_person() {
    let (h, _s, _r, ws) = setup().await;
    // Nothing left to fix and no run here yet: without the wake this is "waiting for you".
    clean_manifest(&ws);
    let w = world();
    let created = create(&h.ctx, ready_req()).await.unwrap();
    install(&h.fake, &created.marker, &w, NOW_FN() + 1_000);
    let rows = Arc::new(Mutex::new(vec![wake_row(0, NOW_FN() + 3_600_000)]));
    let r = rows.clone();
    h.fake.on("harness::observers::list", move |req| {
        assert_eq!(req["session_id"], "s-ours");
        Ok(json!({"observers": r.lock().unwrap().clone()}))
    });
    until(&h, "working", |a| a.state == AgentPhase::Working).await;
    *w.status.lock().unwrap() = "idle".into();
    let a = until(&h, "waiting", |a| a.state == AgentPhase::Waiting).await;
    assert_eq!(
        (a.waiting_for.as_deref(), a.question.as_deref()),
        (Some("the run here to end"), None)
    );
    assert!(
        h.fake
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.kind == EventKind::Agent
                && e.summary == "the agent waits for the run here to end"
                && e.record["state"] == "waiting"
                && !e.terminal)
    );
    // Quiet for longer than the stall time while the wake is armed: still waiting.
    *w.status.lock().unwrap() = "working".into();
    *w.last_ms.lock().unwrap() = NOW_FN() - 3_600_000;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let a = until(&h, "still waiting", |a| a.state == AgentPhase::Waiting).await;
    assert_eq!(a.waiting_for.as_deref(), Some("the run here to end"));
    // The wake fired: today's rules again.
    *w.status.lock().unwrap() = "idle".into();
    *w.last_ms.lock().unwrap() = NOW_FN();
    *rows.lock().unwrap() = vec![wake_row(1, NOW_FN() + 3_600_000)];
    let a = until(&h, "waiting for you", |a| {
        a.state == AgentPhase::WaitingForYou
    })
    .await;
    assert_eq!(
        (a.question.as_deref(), a.waiting_for.as_deref()),
        (
            Some("Your agent stopped before running it here like the cloud."),
            None
        )
    );
}

#[tokio::test]
async fn an_expired_wake_or_a_failing_list_keeps_waiting_for_you() {
    for list in ["expired", "error"] {
        let (h, _s, _r, ws) = setup().await;
        clean_manifest(&ws);
        let w = world();
        let created = create(&h.ctx, ready_req()).await.unwrap();
        install(&h.fake, &created.marker, &w, NOW_FN() + 1_000);
        if list == "expired" {
            let row = wake_row(0, NOW_FN() - 1);
            h.fake.on("harness::observers::list", move |_| {
                Ok(json!({"observers": [row.clone()]}))
            });
        } else {
            h.fake.on("harness::observers::list", |_| {
                Err(iii_cloud::error::CloudError::new(
                    "BOOM",
                    "store unreadable",
                ))
            });
        }
        until(&h, "working", |a| a.state == AgentPhase::Working).await;
        *w.status.lock().unwrap() = "idle".into();
        let a = until(&h, "waiting for you", |a| {
            a.state == AgentPhase::WaitingForYou
        })
        .await;
        assert_eq!(
            (a.question.as_deref(), a.waiting_for.as_deref()),
            (
                Some("Your agent stopped before running it here like the cloud."),
                None
            ),
            "{list}"
        );
        assert!(
            h.fake
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|(f, _)| f == "harness::observers::list"),
            "{list}"
        );
    }
}

#[test]
fn what_a_wake_waits_for_reads_from_its_observer() {
    use iii_cloud::handoff::waiting_for;
    let now = 1_000;
    let on = |kind: Value| json!({"observers": [{"function_id": "iii-cloud::on-change", "filters": {"input.kind": kind}, "once": true, "fires": 0}]});
    assert_eq!(
        waiting_for(&on(json!({"$eq": "run"})), now).as_deref(),
        Some("the run here to end")
    );
    assert_eq!(
        waiting_for(&on(json!({"$eq": "check"})), now).as_deref(),
        Some("the check to end")
    );
    assert_eq!(
        waiting_for(&on(json!("deploy")), now).as_deref(),
        Some("the deploy to end")
    );
    let row = |extra: Value| {
        let mut o = json!({"function_id": "state::on-change", "once": true, "fires": 0});
        o.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"observers": [o]})
    };
    assert_eq!(
        waiting_for(&row(json!({"label": "the build", "action": "built"})), now).as_deref(),
        Some("the build")
    );
    assert_eq!(
        waiting_for(&row(json!({"action": "the build to finish"})), now).as_deref(),
        Some("the build to finish")
    );
    assert_eq!(
        waiting_for(&row(json!({})), now).as_deref(),
        Some("state::on-change to fire")
    );
    // Not a wake on this session: a target, fired, past its deadline, or out of fires.
    assert_eq!(waiting_for(&row(json!({"target": "my::fn"})), now), None);
    assert_eq!(waiting_for(&row(json!({"fires": 1})), now), None);
    assert_eq!(waiting_for(&row(json!({"expires_at": now})), now), None);
    assert_eq!(
        waiting_for(&row(json!({"expires_at": now + 1})), now).as_deref(),
        Some("state::on-change to fire")
    );
    assert_eq!(
        waiting_for(
            &row(json!({"once": false, "max_fires": 3, "fires": 2})),
            now
        )
        .as_deref(),
        Some("state::on-change to fire")
    );
    assert_eq!(
        waiting_for(
            &row(json!({"once": false, "max_fires": 3, "fires": 3})),
            now
        ),
        None
    );
    assert_eq!(waiting_for(&json!({"observers": []}), now), None);
}

#[tokio::test]
async fn a_reused_handoff_delivers_its_text_or_queues_it_until_the_chat_is_found() {
    let (h, _s, _r, _ws) = setup().await;
    let w = world();
    let mut req = ready_req();
    req.text = Some("first".into());
    let a = create(&h.ctx, req.clone()).await.unwrap();
    // No chat yet: the text of the second click is queued, nothing is launched twice.
    req.text = Some("second".into());
    let b = create(&h.ctx, req.clone()).await.unwrap();
    assert_eq!(
        (b.id.as_str(), b.reused, b.delivered),
        (a.id.as_str(), true, false)
    );
    assert_eq!(
        h.ctx.store.read(|s| s.handoffs[0].pending_text.clone()),
        Some("second".into())
    );
    // The chat appears: the queued text goes in, with the marker.
    h.fake.on("harness::send", |_| {
        Ok(json!({"session_id": "s-ours", "turn_id": "t"}))
    });
    install(&h.fake, &a.marker, &w, NOW_FN() + 1_000);
    let sent = |h: &Harness| -> Vec<Value> {
        h.fake
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|(f, _)| f == "harness::send")
            .map(|(_, p)| p.clone())
            .collect()
    };
    for _ in 0..200 {
        if !sent(&h).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let first = sent(&h);
    assert_eq!(first.len(), 1);
    assert_eq!(first[0]["message"], format!("{}\nsecond", a.marker));
    assert_eq!(
        h.ctx.store.read(|s| s.handoffs[0].pending_text.clone()),
        None
    );
    // With a chat, the text goes straight in and says so.
    req.text = Some("third".into());
    let c = create(&h.ctx, req).await.unwrap();
    assert_eq!(
        (
            c.id.as_str(),
            c.reused,
            c.delivered,
            c.session_id.as_deref()
        ),
        (a.id.as_str(), true, true, Some("s-ours"))
    );
    assert_eq!(
        sent(&h).last().unwrap()["message"],
        format!("{}\nthird", a.marker)
    );
}

#[tokio::test]
async fn a_hand_off_from_a_check_of_another_file_keeps_the_agents_checks_on_it() {
    let (h, server, _r, ws) = setup().await;
    std::fs::write(
        ws.join("worker-compose-next.yaml"),
        "containers: {}\ncloud: {}\n",
    )
    .unwrap();
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-m", "next"]);
    let head = git(&ws, &["rev-parse", "HEAD"]);
    h.ctx.store.update(|s| {
        s.check.as_mut().unwrap().manifest = Some("worker-compose-next.yaml".into());
        s.checks[0].manifest = Some("worker-compose-next.yaml".into());
    });
    // The cloud plans the file it is asked for.
    Mock::given(method("GET"))
        .and(path_regex(r"/plan$"))
        .respond_with(|r: &wiremock::Request| {
            let m = r
                .url
                .query_pairs()
                .find(|(k, _)| k == "manifest")
                .map(|(_, v)| v.into_owned());
            ResponseTemplate::new(200).set_body_json(json!({"can_deploy": true, "fingerprint": "fp", "plan": {"source": {"manifest": m}, "settings": {}, "resources": [], "issues": [], "deploy_issues": []}}))
        })
        .with_priority(1)
        .mount(&server)
        .await;
    let a = create(&h.ctx, ready_req()).await.unwrap();
    let stored = h
        .ctx
        .store
        .read(|s| s.handoffs.iter().find(|x| x.id == a.id).cloned())
        .unwrap();
    assert_eq!(stored.manifest.as_deref(), Some("worker-compose-next.yaml"));
    // The brief names the chosen file, and the environment's is one of the others.
    let brief = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .handoff_brief
        .unwrap();
    assert_eq!(brief.manifest, "worker-compose-next.yaml");
    assert!(
        brief
            .other_compose_files
            .contains(&"worker-compose.yaml".to_string()),
        "{:?}",
        brief.other_compose_files
    );
    assert!(
        brief.next_step.contains(
            "Only worker-compose-next.yaml is checked and deployed: your checks on this hand-off use it, and production switches from worker-compose.yaml to it when the person confirms the deploy."
        ),
        "{}",
        brief.next_step
    );
    // The agent's check names no file: the hand-off's goes to the cloud.
    let c = iii_cloud::check::check(
        &h.ctx,
        CheckRequest {
            handoff_id: Some(a.id.clone()),
            commit: Some(head),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(c.manifest.as_deref(), Some("worker-compose-next.yaml"));
    assert_eq!(c.plan.status, PlanStatus::Clean);
    let asked: Vec<String> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|r| r.url.path().ends_with("/plan"))
        .filter_map(|r| r.url.query().map(str::to_owned))
        .collect();
    assert!(
        asked
            .last()
            .is_some_and(|q| q.contains("manifest=worker-compose-next.yaml")),
        "{asked:?}"
    );
}

#[tokio::test]
async fn a_hand_off_from_a_check_of_the_environments_file_carries_none() {
    let (h, _s, _r, _ws) = setup().await;
    h.ctx.store.update(|s| {
        s.check.as_mut().unwrap().manifest = Some("worker-compose.yaml".into());
        s.checks[0].manifest = Some("worker-compose.yaml".into());
        s.project
            .as_mut()
            .unwrap()
            .environment
            .as_mut()
            .unwrap()
            .manifest = Some("worker-compose.yaml".into());
    });
    let a = create(&h.ctx, ready_req()).await.unwrap();
    let stored = h
        .ctx
        .store
        .read(|s| s.handoffs.iter().find(|x| x.id == a.id).cloned())
        .unwrap();
    assert_eq!(stored.manifest, None);
}

#[tokio::test]
async fn status_lists_the_compose_files_the_page_can_pick() {
    let (h, _s, _r, ws) = setup().await;
    std::fs::create_dir_all(ws.join("api")).unwrap();
    for (file, text) in [
        (
            "worker-compose-next.yaml",
            "containers:\n  api:\n    worker: path://./api\n  http:\n    worker: package://http\ncloud: {}\n",
        ),
        ("worker-compose-dev.yaml", "containers: {}\n"),
        ("api/worker-compose-é.yaml", "containers: {}\n"),
        ("my-compose.yaml", "containers: {}\ncloud: {}\n"),
        ("docker-compose.yml", "services: {}\n"),
    ] {
        std::fs::write(ws.join(file), text).unwrap();
    }
    git(&ws, &["add", "-A"]);
    git(&ws, &["commit", "-m", "composes"]);
    let files = iii_cloud::status::status(&h.ctx, StatusRequest::default())
        .await
        .unwrap()
        .compose_files;
    let row = |p: &str| {
        files
            .iter()
            .find(|f| f.path == p)
            .map(|f| (f.choosable, f.issue.clone()))
    };
    // Like the cloud's own picker, a `cloud:` section is not needed.
    assert_eq!(row("worker-compose.yaml"), Some((true, None)));
    assert_eq!(row("worker-compose-next.yaml"), Some((true, None)));
    assert_eq!(row("worker-compose-dev.yaml"), Some((true, None)));
    // A non-ASCII name is listed as it is, not as git quotes it.
    assert_eq!(row("api/worker-compose-é.yaml"), Some((true, None)));
    assert_eq!(
        row("my-compose.yaml"),
        Some((false, Some("NOT_WORKER_COMPOSE".into())))
    );
    assert_eq!(row("docker-compose.yml"), None, "no `containers:`");
    // Each one's containers and whether it has a `cloud:` section.
    let facts = |p: &str| {
        files
            .iter()
            .find(|f| f.path == p)
            .map(|f| (f.workers, f.has_cloud_block))
    };
    assert_eq!(facts("worker-compose-next.yaml"), Some((Some(2), true)));
    assert_eq!(facts("worker-compose-dev.yaml"), Some((Some(0), false)));
}

/// "Edits this folder": the selected workspace is the person's checkout on `main`,
/// with a commit of theirs and changed files, and production's check `chk`.
async fn here() -> (Harness, MockServer, Repo) {
    let (h, server, repo, _ws) = setup().await;
    let ws = repo.checkout.parent().unwrap().join("hello-iii-iii-cloud");
    git(
        &repo.checkout,
        &["worktree", "remove", "--force", ws.to_str().unwrap()],
    );
    git(
        &repo.checkout,
        &["branch", "-q", "-D", "iii-cloud/ready-production"],
    );
    for (k, v) in [("user.name", "Ada"), ("user.email", "ada@example.test")] {
        git(&repo.checkout, &["config", k, v]);
    }
    use_project(
        &h.ctx,
        UseRequest {
            path: repo.checkout.to_string_lossy().into(),
            environment_id: Some(ENV.into()),
            mode: WorkspaceMode::Checkout,
            bring_changes: None,
            cloud_project_id: None,
            from: None,
            source: None,
        },
    )
    .await
    .unwrap();
    std::fs::write(repo.checkout.join("mine.txt"), "mine").unwrap();
    git(&repo.checkout, &["add", "mine.txt"]);
    git(&repo.checkout, &["commit", "-q", "-m", "mine"]);
    std::fs::write(repo.checkout.join("README.md"), "changed\n").unwrap();
    std::fs::write(repo.checkout.join("new.txt"), "new").unwrap();
    (h, server, repo)
}

fn brief_of(h: &Harness) -> HandoffBrief {
    let s = h.ctx.store.read(|s| s.clone());
    iii_cloud::handoff::brief(&s, &s.handoffs[0], &[])
}

#[tokio::test]
async fn a_hand_off_from_the_persons_folder_works_in_a_worker_worktree_and_the_folder_stays() {
    let (h, _s, repo) = here().await;
    let selected = h.ctx.store.read(|s| s.project.clone());
    let before = (
        git(&repo.checkout, &["rev-parse", "HEAD"]),
        git(&repo.checkout, &["status", "--porcelain"]),
    );
    create(&h.ctx, ready_req()).await.unwrap();
    let wt = repo.checkout.parent().unwrap().join("hello-iii-iii-cloud");
    let (path, branch) = h.ctx.store.read(|s| {
        (
            s.handoffs[0].workspace_path.clone(),
            s.handoffs[0].branch.clone(),
        )
    });
    assert_eq!(std::path::PathBuf::from(&path), wt);
    assert_eq!(branch, "iii-cloud/ready-production");
    // What the check saw: the person's commit, and their changed files on top.
    assert_eq!(git(&wt, &["log", "-1", "--format=%s", "HEAD~1"]), "mine");
    assert_eq!(
        git(&wt, &["log", "-1", "--format=%s"]),
        "Uncommitted changes from main"
    );
    assert_eq!(std::fs::read_to_string(wt.join("new.txt")).unwrap(), "new");
    // The person's folder and the page's selection are as they were.
    assert_eq!(
        (
            git(&repo.checkout, &["rev-parse", "HEAD"]),
            git(&repo.checkout, &["status", "--porcelain"]),
        ),
        before
    );
    assert_eq!(h.ctx.store.read(|s| s.project.clone()), selected);
    let brief = brief_of(&h);
    assert_eq!(brief.worktree.as_ref().unwrap().path, path);
    assert!(
        brief.next_step.contains(&format!(
            "Work only in {path} on iii-cloud/ready-production. If this hand-off points at the person's folder, call `iii-cloud::handoff::worktree` first."
        )),
        "{}",
        brief.next_step
    );
    // The agent's tools act in that worktree.
    let id = h.ctx.store.read(|s| s.handoffs[0].id.clone());
    assert_eq!(
        iii_cloud::check::selected_for(&h.ctx, Some(&id))
            .await
            .unwrap()
            .workspace,
        std::path::PathBuf::from(&path)
    );
}

#[tokio::test]
async fn an_older_hand_off_on_the_persons_folder_gets_its_worktree_and_never_a_push_note_for_main()
{
    let (h, _s, repo) = here().await;
    create(&h.ctx, ready_req()).await.unwrap();
    // As an older worker left it: the person's folder on main, and no worktree.
    let wt = repo.checkout.parent().unwrap().join("hello-iii-iii-cloud");
    git(
        &repo.checkout,
        &["worktree", "remove", "--force", wt.to_str().unwrap()],
    );
    git(
        &repo.checkout,
        &["branch", "-q", "-D", "iii-cloud/ready-production"],
    );
    let checkout = repo.checkout.canonicalize().unwrap();
    h.ctx.store.update(|s| {
        s.handoffs[0].workspace_path = checkout.to_string_lossy().into();
        s.handoffs[0].branch = "main".into();
    });
    // origin's main moved on, so it is not in the person's history.
    let other = repo.root.path().join("other");
    git(
        repo.root.path(),
        &["clone", "-q", repo.origin.to_str().unwrap(), "other"],
    );
    std::fs::write(other.join("theirs.txt"), "theirs").unwrap();
    git(&other, &["add", "."]);
    git(&other, &["commit", "-q", "-m", "theirs"]);
    git(&other, &["push", "-q", "origin", "main"]);
    git(&repo.checkout, &["fetch", "-q", "origin"]);
    let brief = brief_of(&h);
    assert_eq!(brief.remote_branch_note, None, "never about main");
    assert!(!brief.next_step.contains("force"), "{}", brief.next_step);
    assert!(
        brief
            .next_step
            .contains("call `iii-cloud::handoff::worktree` first")
    );
    let id = h.ctx.store.read(|s| s.handoffs[0].id.clone());
    let first = iii_cloud::handoff::worktree(
        &h.ctx,
        HandoffWorktreeRequest {
            handoff_id: Some(id.clone()),
        },
    )
    .await
    .unwrap();
    assert!(first.created);
    assert_eq!(std::path::PathBuf::from(&first.path), wt);
    assert_eq!(first.branch, "iii-cloud/ready-production");
    assert_eq!(first.merged_from.as_deref(), Some("main"));
    assert_eq!(first.conflict, None);
    assert_eq!(
        git(&wt, &["log", "-1", "--format=%s", "HEAD~1"]),
        "Merge main into main for the check"
    );
    assert_eq!(
        h.ctx.store.read(|s| (
            s.handoffs[0].workspace_path.clone(),
            s.handoffs[0].branch.clone()
        )),
        (first.path.clone(), first.branch.clone()),
        "rebound"
    );
    assert_eq!(
        iii_cloud::check::selected_for(&h.ctx, Some(&id))
            .await
            .unwrap()
            .workspace,
        wt
    );
    // Again, with the default hand-off: the same worktree, not made again.
    let again = iii_cloud::handoff::worktree(&h.ctx, HandoffWorktreeRequest::default())
        .await
        .unwrap();
    assert_eq!((again.path, again.created), (first.path, false));
    assert_eq!(
        h.ctx
            .store
            .read(|s| s.project.as_ref().unwrap().workspace.clone().unwrap().mode),
        WorkspaceMode::Checkout,
        "the selection keeps its mode"
    );
}

/// A hand-off from the folder resumes the agent's pushed fix branch: its
/// commits stay, the person's commit and changes come on top.
#[tokio::test]
async fn a_hand_off_resumes_the_agents_earlier_fix_branch() {
    let (h, _s, repo) = here().await;
    let t = repo.root.path().join("agent");
    git(
        repo.root.path(),
        &["clone", "-q", repo.origin.to_str().unwrap(), "agent"],
    );
    git(&t, &["checkout", "-q", "-b", "iii-cloud/ready-production"]);
    std::fs::write(t.join("fix.txt"), "fix").unwrap();
    git(&t, &["add", "."]);
    git(
        &t,
        &["commit", "-q", "-m", "Add isolated iii cloud test manifest"],
    );
    git(&t, &["push", "-q", "origin", "iii-cloud/ready-production"]);
    create(&h.ctx, ready_req()).await.unwrap();
    let (path, branch) = h.ctx.store.read(|s| {
        (
            s.handoffs[0].workspace_path.clone(),
            s.handoffs[0].branch.clone(),
        )
    });
    assert_eq!(branch, "iii-cloud/ready-production");
    let wt = std::path::Path::new(&path);
    for f in ["fix.txt", "mine.txt", "new.txt"] {
        assert!(wt.join(f).exists(), "{f}");
    }
    assert!(git(wt, &["log", "--format=%s"]).contains("Add isolated iii cloud test manifest"));
}

//! The worker's four observable functions against a live engine.
//!
//! Each test hosts a function on one connection, observes it from another
//! with args, filters and both `deliver` shapes, produces the event and
//! checks what each target received, and that a disposed observer receives
//! nothing more. The engine is `III_ENGINE_WS_URL` (default
//! `ws://127.0.0.1:49134`); without one the tests skip, and with
//! `III_E2E_REQUIRE` set they fail instead.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ide::code::config::CoderConfig;
use ide::code::path::PathResolver;
use ide::code::state::ResolverCell;
use ide::config::ShellConfig;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, Deliver, Error, IIIClient, InitOptions, Observed, ObserverHandle,
    RegisterFunction, Source,
};
use serde_json::{json, Value};

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:49134";

/// How long a target is given to receive what it is owed.
const DELIVERY: Duration = Duration::from_secs(15);
/// How long a target that is owed nothing more is watched for.
const QUIET: Duration = Duration::from_millis(1500);

fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| DEFAULT_WS_URL.to_string())
}

/// A connection under its own worker name, once the engine answers; `None`
/// (skip) without an engine unless `III_E2E_REQUIRE` is set.
async fn connect(name: &str) -> Option<Arc<IIIClient>> {
    let iii = Arc::new(register_worker(
        &ws_url(),
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: format!("{name}-{}", uuid::Uuid::new_v4()),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let probe = iii
            .call(CallRequest {
                function_id: "engine::workers::list".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(800),
            })
            .await;
        if probe.is_ok() {
            return Some(iii);
        }
    }
    iii.shutdown_async().await;
    assert!(
        std::env::var("III_E2E_REQUIRE").is_err(),
        "no iii engine reachable at {}: start one or set III_ENGINE_WS_URL",
        ws_url()
    );
    eprintln!(
        "[skip] no iii engine reachable at {}; set III_E2E_REQUIRE=1 to fail instead",
        ws_url()
    );
    None
}

/// What one target received, in order.
#[derive(Default)]
struct Received(Mutex<Vec<Value>>);

impl Received {
    fn all(&self) -> Vec<Value> {
        self.0.lock().unwrap().clone()
    }

    /// Waits until `done` holds for what arrived so far.
    async fn wait_until(&self, what: &str, done: impl Fn(&[Value]) -> bool) -> Vec<Value> {
        let deadline = tokio::time::Instant::now() + DELIVERY;
        loop {
            let all = self.all();
            if done(&all) {
                return all;
            }
            assert!(
                tokio::time::Instant::now() < deadline,
                "{what}: still waiting, received {all:?}"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// Registers `id` on `iii`; every payload it is called with is recorded.
fn recorder(iii: &IIIClient, id: &str) -> Arc<Received> {
    let received = Arc::new(Received::default());
    let sink = received.clone();
    iii.register_function(
        id,
        RegisterFunction::new(move |payload: Value| {
            sink.0.lock().unwrap().push(payload);
            Ok::<_, Error>(json!({ "ok": true }))
        }),
    );
    received
}

/// Observes `source` into `target` and waits until the observer is active.
async fn observe(iii: &IIIClient, source: Source, target: &str) -> ObserverHandle {
    let handle = iii.observe(source, target);
    tokio::time::timeout(Duration::from_secs(10), handle.ready())
        .await
        .unwrap_or_else(|_| panic!("observer of {target} did not settle"))
        .unwrap_or_else(|e| panic!("observer of {target} failed: {e}"));
    handle
}

/// The event an `Observed` envelope carries, failing unless the payload is
/// the envelope a target typed `Observed<Value, Value>` reads.
fn envelope_output(payload: &Value) -> Value {
    let envelope: Observed<Value, Value> = serde_json::from_value(payload.clone())
        .unwrap_or_else(|e| panic!("not an Observed envelope ({e}): {payload}"));
    envelope.output.expect("the envelope carries the output")
}

fn resolver_rooted_at(root: &Path) -> ResolverCell {
    let cfg = CoderConfig {
        base_paths: vec![root.to_path_buf()],
        ..Default::default()
    };
    Arc::new(tokio::sync::RwLock::new(Arc::new(
        PathResolver::new(&cfg).unwrap(),
    )))
}

fn path_str(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

// --- shell::on-job-finished ------------------------------------------------

/// Starts `command` as a host background job named `job_id` and waits until
/// its record is final.
async fn run_job(iii: &IIIClient, job_id: &str, command: &str) {
    let cfg = Arc::new(ShellConfig::default());
    let request = serde_json::from_value(json!({ "command": command, "job_id": job_id })).unwrap();
    ide::functions::exec_bg::handle(cfg, iii.clone(), request)
        .await
        .unwrap();
    let handle = ide::jobs::get(job_id).await.expect("the job is retained");
    tokio::time::timeout(Duration::from_secs(10), async {
        while !handle.lock().await.finalized {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("job {job_id} did not finish"));
}

fn job(job_id: &str) -> Source {
    Source::function("shell::on-job-finished").args(json!({ "job_id": job_id }))
}

fn failed_only(source: Source) -> Source {
    source
        .filters(json!({ "input.status": { "$eq": "failed" } }))
        .deliver(Deliver::Output)
}

fn job_ids(events: &[Value]) -> Vec<String> {
    let mut ids: Vec<String> = events
        .iter()
        .map(|event| event["job_id"].as_str().unwrap().to_string())
        .collect();
    ids.sort();
    ids
}

/// The keys of an `Observed` envelope that every delivery carries, whatever
/// call it came from.
fn envelope_keys(payload: &Value) -> Vec<String> {
    let mut keys: Vec<String> = payload
        .as_object()
        .unwrap_or_else(|| panic!("not an object: {payload}"))
        .keys()
        .filter(|key| !matches!(key.as_str(), "metadata" | "caller"))
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// A job's completion reaches each observer of it once, whether the
/// observer was added before the job ended (the live call) or after (the
/// replay: a call with metadata `{ for_observer }`), in the shape its
/// `deliver` asks for and only when its filters match. A replay is
/// delivered like the live call and reaches its observer alone: neither an
/// observer of the job that took the live call nor an observer without a
/// `job_id` receives it. A disposed observer receives nothing.
#[tokio::test]
async fn on_job_finished_reaches_each_observer_of_a_job_once_live_or_replayed() {
    let Some(owner) = connect("ide-e2e-jobs-owner").await else {
        return;
    };
    ide::job_events::register_job_finished(&owner);
    let Some(consumer) = connect("ide-e2e-jobs-consumer").await else {
        return;
    };
    let run = uuid::Uuid::new_v4();
    let id = |name: &str| format!("e2e-{name}-{run}");
    let (live, live_failed) = (id("live"), id("live-failed"));
    let (late, late_failed) = (id("late"), id("late-failed"));
    let dropped = id("dropped");

    let every = recorder(&consumer, "e2e::jobs::every");
    let every_envelope = recorder(&consumer, "e2e::jobs::every-envelope");
    let live_envelope = recorder(&consumer, "e2e::jobs::live-envelope");
    let live_output = recorder(&consumer, "e2e::jobs::live-output");
    let live_failures = recorder(&consumer, "e2e::jobs::live-failures");
    let late_envelope = recorder(&consumer, "e2e::jobs::late-envelope");
    let late_output = recorder(&consumer, "e2e::jobs::late-output");
    let late_failures = recorder(&consumer, "e2e::jobs::late-failures");
    let early_envelope = recorder(&consumer, "e2e::jobs::early-envelope");
    let early_output = recorder(&consumer, "e2e::jobs::early-output");
    let dropped_output = recorder(&consumer, "e2e::jobs::dropped");

    // Observed before the jobs start: the live call.
    let every_handle = observe(
        &consumer,
        Source::function("shell::on-job-finished").deliver(Deliver::Output),
        "e2e::jobs::every",
    )
    .await;
    let every_envelope_handle = observe(
        &consumer,
        Source::function("shell::on-job-finished"),
        "e2e::jobs::every-envelope",
    )
    .await;
    observe(&consumer, job(&live), "e2e::jobs::live-envelope").await;
    observe(
        &consumer,
        job(&live).deliver(Deliver::Output),
        "e2e::jobs::live-output",
    )
    .await;
    observe(
        &consumer,
        failed_only(job(&live)),
        "e2e::jobs::live-failures",
    )
    .await;
    observe(
        &consumer,
        failed_only(job(&live_failed)),
        "e2e::jobs::live-failures",
    )
    .await;
    let dropped_handle = observe(
        &consumer,
        job(&dropped).deliver(Deliver::Output),
        "e2e::jobs::dropped",
    )
    .await;
    // Observers of `late` that take its live call, before the replays
    // made for the observers added after it ended.
    observe(&consumer, job(&late), "e2e::jobs::early-envelope").await;
    observe(
        &consumer,
        job(&late).deliver(Deliver::Output),
        "e2e::jobs::early-output",
    )
    .await;
    run_job(&owner, &live, "true").await;
    run_job(&owner, &live_failed, "false").await;

    // Observed after the jobs finished: the replay.
    run_job(&owner, &late, "true").await;
    run_job(&owner, &late_failed, "false").await;
    every
        .wait_until("every live completion", |all| all.len() >= 4)
        .await;
    every_envelope
        .wait_until("every live completion, enveloped", |all| all.len() >= 4)
        .await;
    let late_envelope_handle = observe(&consumer, job(&late), "e2e::jobs::late-envelope").await;
    let late_output_handle = observe(
        &consumer,
        job(&late).deliver(Deliver::Output),
        "e2e::jobs::late-output",
    )
    .await;
    observe(
        &consumer,
        failed_only(job(&late)),
        "e2e::jobs::late-failures",
    )
    .await;
    observe(
        &consumer,
        failed_only(job(&late_failed)),
        "e2e::jobs::late-failures",
    )
    .await;

    for (what, target) in [
        ("live envelope", &live_envelope),
        ("live output", &live_output),
        ("live failures", &live_failures),
        ("early envelope", &early_envelope),
        ("early output", &early_output),
        ("replayed envelope", &late_envelope),
        ("replayed output", &late_output),
        ("replayed failures", &late_failures),
    ] {
        target.wait_until(what, |all| !all.is_empty()).await;
    }

    // Disposed before its job runs: nothing.
    dropped_handle.dispose().await.unwrap();
    every_handle.dispose().await.unwrap();
    every_envelope_handle.dispose().await.unwrap();
    run_job(&owner, &dropped, "true").await;
    tokio::time::sleep(QUIET).await;

    let envelope = live_envelope.all();
    assert_eq!(envelope.len(), 1, "live envelope: {envelope:?}");
    let event = envelope_output(&envelope[0]);
    assert_eq!(event["job_id"], live.as_str());
    assert_eq!(event["status"], "finished");
    assert_eq!(event["exit_code"], 0);
    assert_eq!(live_output.all(), [event], "live output is the event");

    let live_delivery = envelope[0].clone();
    let envelope = late_envelope.all();
    assert_eq!(envelope.len(), 1, "replayed envelope: {envelope:?}");
    let replayed = &envelope[0];
    let event = envelope_output(replayed);
    assert_eq!(event["job_id"], late.as_str());
    assert_eq!(event["status"], "finished");
    assert!(
        event.get("_caller_worker_id").is_none(),
        "the replay is delivered as iii delivers: {event}"
    );
    // The replay is an envelope like the live call's, of the same function,
    // for the observer it was made for.
    assert_eq!(envelope_keys(replayed), envelope_keys(&live_delivery));
    assert_eq!(replayed["function_id"], "shell::on-job-finished");
    assert_eq!(replayed["observer_id"], late_envelope_handle.id());
    assert_eq!(replayed["input"], event, "the call's input is the event");
    assert_eq!(
        late_output.all(),
        std::slice::from_ref(&event),
        "replayed output is the event"
    );
    assert_ne!(late_envelope_handle.id(), late_output_handle.id());

    // The observers of `late` that took its live call received it once,
    // in their own shape, and neither replay.
    let envelope = early_envelope.all();
    assert_eq!(envelope.len(), 1, "early envelope: {envelope:?}");
    assert_eq!(envelope_output(&envelope[0]), event);
    assert_eq!(envelope_keys(&envelope[0]), envelope_keys(&live_delivery));
    assert_eq!(early_output.all(), [event], "early output is the event");

    // The filters select the failed job of each pair, live and replayed.
    let failures = live_failures.all();
    assert_eq!(job_ids(&failures), std::slice::from_ref(&live_failed));
    assert_eq!(failures[0]["status"], "failed");
    let failures = late_failures.all();
    assert_eq!(job_ids(&failures), std::slice::from_ref(&late_failed));
    assert_eq!(failures[0]["status"], "failed");

    // The catch-all heard the four completions live and no replay; neither
    // disposed observer heard the last job.
    let mut expected = vec![live, live_failed, late, late_failed];
    expected.sort();
    assert_eq!(job_ids(&every.all()), expected);
    let enveloped: Vec<Value> = every_envelope.all().iter().map(envelope_output).collect();
    assert_eq!(job_ids(&enveloped), expected);
    assert!(
        dropped_output.all().is_empty(),
        "{:?}",
        dropped_output.all()
    );

    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}

// --- shell::on-changed -----------------------------------------------------

fn changed(dir: &Path) -> Source {
    Source::function("shell::on-changed").args(json!({ "path": path_str(dir) }))
}

fn paths(events: &[Value]) -> Vec<String> {
    events
        .iter()
        .map(|event| event["path"].as_str().unwrap().to_string())
        .collect()
}

/// A change reaches the observers of the directory it happened in, in the
/// shape each one's `deliver` asks for and through its filters, and stops
/// reaching an observer once it is disposed.
#[tokio::test]
async fn on_changed_reaches_the_observers_of_its_directory_until_disposed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let (a, b) = (root.join("a"), root.join("b"));
    std::fs::create_dir(&a).unwrap();
    std::fs::create_dir(&b).unwrap();
    let Some(owner) = connect("ide-e2e-changed-owner").await else {
        return;
    };
    ide::events::register_changed(&owner, resolver_rooted_at(&root));
    let Some(consumer) = connect("ide-e2e-changed-consumer").await else {
        return;
    };
    let a_output = recorder(&consumer, "e2e::changed::a-output");
    let a_kept = recorder(&consumer, "e2e::changed::a-kept");
    let b_envelope = recorder(&consumer, "e2e::changed::b-envelope");
    let kept = ["keep.txt", "keep-after.txt"];
    let a_handle = observe(
        &consumer,
        changed(&a).deliver(Deliver::Output),
        "e2e::changed::a-output",
    )
    .await;
    observe(
        &consumer,
        changed(&a)
            .filters(json!({ "input.path": { "$in": kept } }))
            .deliver(Deliver::Output),
        "e2e::changed::a-kept",
    )
    .await;
    observe(&consumer, changed(&b), "e2e::changed::b-envelope").await;

    std::fs::write(a.join("one.txt"), "1").unwrap();
    std::fs::write(a.join("keep.txt"), "k").unwrap();
    let events = a_output
        .wait_until("both changes in a", |all| {
            let seen = paths(all);
            ["one.txt", "keep.txt"]
                .iter()
                .all(|p| seen.iter().any(|s| s == p))
        })
        .await;
    let event = events.iter().find(|e| e["path"] == "one.txt").unwrap();
    assert_eq!(event["root"], path_str(&a));
    assert_eq!(event["dir"], false);
    a_kept
        .wait_until("the kept change in a", |all| {
            paths(all).iter().any(|p| p == "keep.txt")
        })
        .await;

    std::fs::write(b.join("two.txt"), "2").unwrap();
    let envelopes = b_envelope
        .wait_until("a change in b", |all| {
            all.iter()
                .any(|payload| envelope_output(payload)["path"] == "two.txt")
        })
        .await;
    for payload in &envelopes {
        assert_eq!(envelope_output(payload)["root"], path_str(&b));
    }

    a_handle.dispose().await.unwrap();
    std::fs::write(a.join("three.txt"), "3").unwrap();
    std::fs::write(a.join("keep-after.txt"), "k").unwrap();
    a_kept
        .wait_until("the kept change in a after the dispose", |all| {
            paths(all).iter().any(|p| p == "keep-after.txt")
        })
        .await;
    tokio::time::sleep(QUIET).await;

    assert!(
        !paths(&a_output.all())
            .iter()
            .any(|p| p == "three.txt" || p == "keep-after.txt"),
        "a disposed observer heard a change: {:?}",
        a_output.all()
    );
    assert!(
        paths(&a_kept.all())
            .iter()
            .all(|p| kept.contains(&p.as_str())),
        "the path filter let a change through: {:?}",
        a_kept.all()
    );
    assert!(
        b_envelope
            .all()
            .iter()
            .all(|payload| envelope_output(payload)["root"] == path_str(&b)),
        "b heard a change in a: {:?}",
        b_envelope.all()
    );
    assert!(
        a_output.all().iter().all(|e| e["root"] == path_str(&a)),
        "a heard a change in b: {:?}",
        a_output.all()
    );

    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}

// --- shell::on-git-changed -------------------------------------------------

fn git(repo: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("git runs");
    assert!(
        status.success(),
        "git {args:?} failed in {}",
        repo.display()
    );
}

fn git_changed(repo: &Path) -> Source {
    Source::function("shell::on-git-changed").args(json!({ "path": path_str(repo) }))
}

fn mentions_index(event: &Value) -> bool {
    event["changes"]
        .as_array()
        .is_some_and(|changes| changes.iter().any(|c| c == "index"))
}

/// A repository's state change reaches the observers of that repository,
/// in the shape each one's `deliver` asks for, and stops reaching an
/// observer once it is disposed.
#[tokio::test]
async fn on_git_changed_reaches_the_observers_of_its_repository_until_disposed() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let (one, two): (PathBuf, PathBuf) = (root.join("one"), root.join("two"));
    for repo in [&one, &two] {
        std::fs::create_dir(repo).unwrap();
        git(repo, &["init", "-q"]);
    }
    let Some(owner) = connect("ide-e2e-git-owner").await else {
        return;
    };
    ide::git_events::register_git_changed(&owner, resolver_rooted_at(&root));
    let Some(consumer) = connect("ide-e2e-git-consumer").await else {
        return;
    };
    let one_output = recorder(&consumer, "e2e::git::one-output");
    let two_envelope = recorder(&consumer, "e2e::git::two-envelope");
    let one_handle = observe(
        &consumer,
        git_changed(&one).deliver(Deliver::Output),
        "e2e::git::one-output",
    )
    .await;
    observe(&consumer, git_changed(&two), "e2e::git::two-envelope").await;

    std::fs::write(one.join("a.txt"), "a").unwrap();
    git(&one, &["add", "a.txt"]);
    let events = one_output
        .wait_until("an index change in one", |all| {
            all.iter().any(mentions_index)
        })
        .await;
    assert!(
        events.iter().all(|e| e["path"] == path_str(&one)),
        "{events:?}"
    );

    std::fs::write(two.join("b.txt"), "b").unwrap();
    git(&two, &["add", "b.txt"]);
    let envelopes = two_envelope
        .wait_until("an index change in two", |all| {
            all.iter()
                .any(|payload| mentions_index(&envelope_output(payload)))
        })
        .await;
    for payload in &envelopes {
        assert_eq!(envelope_output(payload)["path"], path_str(&two));
    }

    one_handle.dispose().await.unwrap();
    let before = one_output.all().len();
    std::fs::write(one.join("c.txt"), "c").unwrap();
    git(&one, &["add", "c.txt"]);
    tokio::time::sleep(QUIET).await;
    assert_eq!(
        one_output.all().len(),
        before,
        "a disposed observer heard a change: {:?}",
        one_output.all()
    );
    assert!(
        two_envelope
            .all()
            .iter()
            .all(|payload| envelope_output(payload)["path"] == path_str(&two)),
        "two heard a change in one: {:?}",
        two_envelope.all()
    );

    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}

// --- shell::turns::on-changed ----------------------------------------------

/// A stored session record reaches observers whose `input.session_id`
/// filter it matches, once per coalescing window, in the shape each one's
/// `deliver` asks for, and nothing reaches an observer once it is disposed.
#[tokio::test]
async fn turns_on_changed_reaches_the_observers_of_its_session_until_disposed() {
    let Some(owner) = connect("ide-e2e-turns-owner").await else {
        return;
    };
    let feed = ide::turn_events::register_turns_changed(&owner);
    let Some(consumer) = connect("ide-e2e-turns-consumer").await else {
        return;
    };
    let mine = recorder(&consumer, "e2e::turns::mine");
    let every = recorder(&consumer, "e2e::turns::every");
    let mine_handle = observe(
        &consumer,
        Source::function("shell::turns::on-changed")
            .filters(json!({ "input.session_id": { "$eq": "s1" } }))
            .deliver(Deliver::Output),
        "e2e::turns::mine",
    )
    .await;
    let every_handle = observe(
        &consumer,
        Source::function("shell::turns::on-changed"),
        "e2e::turns::every",
    )
    .await;

    for _ in 0..5 {
        feed.notify("s1");
    }
    feed.notify("s2");
    let envelopes = every
        .wait_until("both sessions", |all| all.len() >= 2)
        .await;
    tokio::time::sleep(QUIET).await;

    assert_eq!(mine.all(), [json!({ "session_id": "s1" })]);
    let mut sessions: HashMap<String, usize> = HashMap::new();
    for payload in every.all() {
        let session = envelope_output(&payload)["session_id"]
            .as_str()
            .unwrap()
            .to_string();
        *sessions.entry(session).or_default() += 1;
    }
    assert_eq!(
        sessions,
        HashMap::from([("s1".to_string(), 1), ("s2".to_string(), 1)]),
        "{envelopes:?}"
    );

    mine_handle.dispose().await.unwrap();
    every_handle.dispose().await.unwrap();
    feed.notify("s1");
    tokio::time::sleep(QUIET).await;
    assert_eq!(mine.all().len(), 1);
    assert_eq!(every.all().len(), 2);

    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}

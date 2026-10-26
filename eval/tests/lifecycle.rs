//! The whole cycle driven through the controller entrypoints (`eval::start`,
//! `eval::step`, `eval::sweep`, `eval::execution-changed`, `eval::cancel`)
//! with the real candidate module against local, native-shaped mocks: the
//! E2E and Sentinel on this worker's Engine, the Harness actor sessions, a
//! git repository with a bare remote, and a tiny HTTP server standing in for
//! GitHub. No paid call and no live service.

#[path = "../../judge-typesafe/tests/support/fake_engine.rs"]
mod fake_engine;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eval::contract::*;
use eval::runtime::{self, Deps};
use eval::state;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const OBJECTIVE: &str = "Reduzir o consumo de contexto mantendo a corre\u{e7}\u{e3}o das tarefas";
const CONTEXT: &str = "context_pressure";
const CONTROL: &str = "minimal_path";
const CONTEXT_CRITERIA: [&str; 4] = [
    "charter_preserved",
    "needles_recovered",
    "ingestion_discipline",
    "concise_report",
];
const CONTROL_CRITERIA: [&str; 1] = ["path_completed"];
const SOURCE_TEMPLATE: &str = "iii: 0.22.0\ncontainers:\n  harness:\n    worker: package://harness\n    commit: \"{commit}\"\n  context-manager:\n    worker: package://context-manager\n    commit: \"{commit}\"\n";

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

// ---------------------------------------------------------------------------
// Native-shaped E2E bundles
// ---------------------------------------------------------------------------

/// What a side's reports state about the system under test.
#[derive(Clone)]
struct System {
    workers_revision: String,
    harness_version: String,
    binary: Option<String>,
}

fn run(run_id: &str, status: &str, tokens: f64, awards: &[Option<u8>], criteria: &[&str]) -> Value {
    json!({"run_id": run_id, "attempt_number": 1, "session_id": format!("s-{run_id}"),
        "status": status, "completion": "completed", "technical": "valid", "wall_time_ms": 1000,
        "cost": {"total_usd": 0.2}, "efficiency": {"total_tokens": tokens, "function_calls": 50},
        "criteria": criteria.iter().zip(awards).map(|(id, awarded)| json!({
            "id": id, "description": format!("Asks for {id}?"), "possible": 10,
            "awarded": awarded, "gate": false, "reason": "r"}))
            .collect::<Vec<_>>(),
        "transcript": {"messages": []}})
}

/// The runs of one side: context_pressure at `tokens` with `context`
/// awards, and the control fully held.
/// The defect: `needles_recovered` loses half its points; a fix restores
/// it. A native, independently assessed E2E criterion, not a counter the
/// Harness reports about itself.
const LOST: [Option<u8>; 4] = [Some(10), Some(5), Some(10), Some(10)];
const FIXED: [Option<u8>; 4] = [Some(10); 4];
/// Tokens are the Harness's own report: kept equal, never the criterion.
const TOKENS: f64 = 90_000.0;

/// Runs per side the plan freezes: five same-case pairs are the fewest a
/// one-sided exact sign test at 0.05 can accept.
const RUNS: usize = 5;

fn side(tokens: f64, context: [Option<u8>; 4]) -> Vec<(&'static str, Vec<Value>)> {
    vec![
        (
            CONTEXT,
            (0..RUNS)
                .map(|at| {
                    run(
                        &format!("c{at}"),
                        "passed",
                        tokens,
                        &context,
                        &CONTEXT_CRITERIA,
                    )
                })
                .collect(),
        ),
        (
            CONTROL,
            (0..RUNS)
                .map(|at| {
                    run(
                        &format!("m{at}"),
                        "passed",
                        500.0,
                        &[Some(10)],
                        &CONTROL_CRITERIA,
                    )
                })
                .collect(),
        ),
    ]
}

/// The native case and execution policy of a scenario, as a report carries
/// them; the catalog's `contract_sha256` is recomputed from them.
fn native_scenario(id: &str) -> Value {
    let (behavior, case, inputs) = scenario_identity(id);
    json!({"scenario_id": id, "case_id": case, "behavior_sha256": behavior,
        "case": {"scenario_id": id, "case_id": case, "behavior_sha256": behavior,
            "inputs_sha256": inputs, "seed": 1, "inputs": {}, "deliverable_contract": {"invariants": []}},
        "execution_policy": {"max_turns": 60, "stuck_timeout_seconds": 600}})
}

fn contract_of(id: &str) -> String {
    eval::evidence::scenario_contract_sha256(&native_scenario(id)).expect("a whole native case")
}

fn scenario_identity(id: &str) -> (&'static str, String, String) {
    match id {
        CONTEXT => (
            "sha256:ctx",
            "context_pressure:seed-1".into(),
            "inputs-context_pressure".into(),
        ),
        _ => (
            "sha256:min",
            "minimal_path:seed-1".into(),
            "inputs-minimal_path".into(),
        ),
    }
}

fn bundle(
    id: &str,
    label: &str,
    status: &str,
    parameters: &Value,
    runs: Vec<(&str, Vec<Value>)>,
    system: &System,
) -> Value {
    let reports: Vec<Value> = if runs.is_empty() {
        vec![
            json!({"scenario_id": CONTEXT, "available": false, "report": null, "state": "running"}),
        ]
    } else {
        runs.into_iter()
            .map(|(scenario_id, runs)| {
                let mut scenario = native_scenario(scenario_id);
                scenario["summary"] = json!(format!("What {scenario_id} expects."));
                scenario["runs"] = json!(runs);
                json!({"scenario_id": scenario_id, "available": true, "report": {
                    "result_contract_sha256": "rc-1",
                    "system_under_test": {
                        // Registry stack versions prove every non-Harness
                        // component. `workers_revision` stands in for the
                        // native full-commit proof of the source-built
                        // Harness that is still being located (open gap).
                        "stack": {"mode": "registry", "workers_repository": "iii-hq/workers",
                                  "workers_revision": system.workers_revision,
                                  "stack_versions": {"harness": system.harness_version,
                                      "engine": "0.22.0", "state": "1.0.0"}},
                        "engine_version": "0.22.0", "harness_version": system.harness_version,
                        "harness_binary_sha256": system.binary,
                        // A source-built Harness: the validated build links
                        // the full commit to the executable measured.
                        "harness_source": if system.workers_revision.len() == 40 {
                            json!({"repository": "iii-hq/workers", "path": "harness",
                                "revision": system.workers_revision, "binary_sha256": system.binary})
                        } else { Value::Null },
                        "e2e_repository": "iii-hq/harness-e2e", "e2e_revision": "e-1",
                        "contract_hashes": {"harness::status": "x"}},
                    "subject": {"model": "m", "provider": "p", "context_window": 200000,
                        "max_output_tokens": 16384, "supports_tools": true, "supports_vision": null},
                    "scenarios": [scenario]}})
            })
            .collect()
    };
    json!({"manifest": {"executions": [{"id": id, "label": label, "status": status,
            "run_id": id, "attempt": 1, "started_at": "2099-01-01T00:00:00Z",
            "completed_at": if status == "running" { "" } else { "2099-01-01T00:10:00Z" },
            "parameters": parameters}]},
        "detail": {"reports": reports}})
}

/// An E2E dashboard: dispatches are recorded with their parameters and
/// stay running until the test finishes them.
#[derive(Default)]
struct Dashboard {
    prefix: &'static str,
    executions: BTreeMap<String, Value>,
    dispatched: Vec<(String, String, Value)>,
}

impl Dashboard {
    fn respond(&mut self, function: &str, data: &Value) -> Option<Result<Value, String>> {
        Some(Ok(match function {
            "e2e::dashboard::execution-start" => {
                let id = format!("{}-{}", self.prefix, self.dispatched.len() + 1);
                let label = data["label"].as_str().unwrap_or_default().to_string();
                let parameters = data["parameters"].clone();
                let system = System {
                    workers_revision: String::new(),
                    harness_version: String::new(),
                    binary: None,
                };
                let mut execution =
                    bundle(&id, &label, "running", &parameters, Vec::new(), &system);
                execution["manifest"]["executions"][0]["idempotency_key"] =
                    data["idempotency_key"].clone();
                self.executions.insert(id.clone(), execution);
                self.dispatched.push((id.clone(), label, parameters));
                json!({"execution_id": id})
            }
            "e2e::dashboard::execution-get" => {
                match self.executions.get(data["execution_id"].as_str()?) {
                    Some(bundle) => bundle.clone(),
                    None => return Some(Err("execution not found".into())),
                }
            }
            "e2e::dashboard::executions-list" => {
                let query = data["query"].as_str().unwrap_or_default();
                let executions: Vec<Value> = self
                    .executions
                    .values()
                    .map(|bundle| bundle["manifest"]["executions"][0].clone())
                    .filter(|summary| {
                        summary["label"]
                            .as_str()
                            .unwrap_or_default()
                            .contains(query)
                            || summary["idempotency_key"]
                                .as_str()
                                .unwrap_or_default()
                                .contains(query)
                    })
                    .collect();
                json!({"executions": executions, "next_cursor": null})
            }
            "e2e::dashboard::execution-cancel" => json!({}),
            _ => return None,
        }))
    }

    fn finish(&mut self, id: &str, status: &str, runs: Vec<(&str, Vec<Value>)>, system: &System) {
        let idempotency_key =
            self.executions[id]["manifest"]["executions"][0]["idempotency_key"].clone();
        let (_, label, parameters) = self
            .dispatched
            .iter()
            .find(|(dispatched, _, _)| dispatched == id)
            .cloned()
            .expect("dispatched");
        let mut execution = bundle(id, &label, status, &parameters, runs, system);
        execution["manifest"]["executions"][0]["idempotency_key"] = idempotency_key;
        self.executions.insert(id.into(), execution);
    }
}

// ---------------------------------------------------------------------------
// This worker's Engine: state, queue, E2E, Sentinel, Harness sessions, judge
// ---------------------------------------------------------------------------

struct World {
    state: BTreeMap<(String, String), Value>,
    steps: VecDeque<Value>,
    calls: Vec<(String, Value)>,
    e2e: Dashboard,
    tests_pages: Vec<Value>,
    catalog: Value,
    boundary: &'static str,
    /// Harness sessions: `(turn_id, status, result)`.
    sessions: HashMap<String, (String, String, Value)>,
    judge_choice: &'static str,
    /// `judge::evaluate` answers `http` this many times before it answers.
    judge_failures: u32,
}

fn row(
    test_id: &str,
    behavior: &str,
    criteria: &[(&str, &str)],
    summary: &str,
    prompt: &str,
) -> Value {
    json!({"test_id": test_id, "lifecycle": "active", "current_version": behavior,
        "selected_version": behavior, "runs_current": 8, "runs_total": 8,
        "spec": {"summary": summary, "prompt": prompt,
            "criteria": criteria.iter().map(|(id, description)| json!({"id": id,
                "description": description, "gate": false})).collect::<Vec<_>>()}})
}

fn scenario(id: &str) -> Value {
    let (behavior, case, inputs) = scenario_identity(id);
    json!({"scenario_id": id, "behavior_sha256": behavior, "case_id": case, "seed": 1,
        "contract_sha256": contract_of(id), "inputs_sha256": inputs, "summary": "",
        "deliverable_contract": {"invariants": []}})
}

impl World {
    fn new() -> Self {
        Self {
            state: BTreeMap::new(),
            steps: VecDeque::new(),
            calls: Vec::new(),
            e2e: Dashboard {
                prefix: "plan",
                ..Dashboard::default()
            },
            tests_pages: vec![json!({"revision": "rev-1", "next_cursor": null, "rows": [
                row(CONTEXT, "sha256:ctx", &[
                    ("charter_preserved", "Every charter fact after the full distractor load."),
                    ("needles_recovered", "Every needle in order."),
                    ("ingestion_discipline", "Each segment read once."),
                    ("concise_report", "No echoed document text.")],
                    "Recovers early charter facts after a large distractor corpus fills the context.",
                    "Complete a context-endurance run over 48 large documents."),
                row(CONTROL, "sha256:min", &[("path_completed", "The minimal path completed.")],
                    "The smallest end-to-end task.", "Do the minimal task."),
            ]})],
            catalog: json!({"catalog_sha256": "sha256:cat", "scenarios": [scenario(CONTEXT), scenario(CONTROL)]}),
            boundary: "workspace",
            sessions: HashMap::new(),
            judge_choice: "relevant_opportunity",
            judge_failures: 0,
        }
    }

    fn calls_to(&self, function: &str) -> Vec<Value> {
        self.calls
            .iter()
            .filter(|(id, _)| id == function)
            .map(|(_, payload)| payload.clone())
            .collect()
    }

    fn respond(&mut self, function: &str, data: &Value) -> Option<Result<Value, String>> {
        if let Some(answer) = self.e2e.respond(function, data) {
            return Some(answer);
        }
        let key = || {
            (
                data["scope"].as_str().unwrap_or_default().to_string(),
                data["key"].as_str().unwrap_or_default().to_string(),
            )
        };
        Some(Ok(match function {
            "state::get" => self.state.get(&key()).cloned().unwrap_or(Value::Null),
            "state::set" => {
                self.state.insert(key(), data["value"].clone());
                json!({})
            }
            "state::compare-and-set" => {
                let current = self.state.get(&key()).cloned();
                let matches = match data.get("expected") {
                    None | Some(Value::Null) => current.is_none(),
                    Some(expected) => current.as_ref() == Some(expected),
                };
                if matches {
                    self.state.insert(key(), data["value"].clone());
                }
                json!({"swapped": matches})
            }
            "state::delete" => {
                self.state.remove(&key());
                json!({})
            }
            "state::list" => Value::Array(
                self.state
                    .iter()
                    .filter(|((scope, _), _)| scope == data["scope"].as_str().unwrap())
                    .map(|(_, value)| value.clone())
                    .collect(),
            ),
            "eval::step" => {
                self.steps.push_back(data.clone());
                json!({"message_id": "queued"})
            }
            "e2e::dashboard::tests-list" => self.tests_pages[0].clone(),
            "e2e::scenarios-list" => self.catalog.clone(),
            "sentinel::reports::ingest" => {
                json!({"recorded": 0, "deduped": 0, "skipped": 0, "results": []})
            }
            "harness::filesystem::info" => json!({"default_root": null, "boundary": self.boundary}),
            "harness::status" => match self
                .sessions
                .get(data["session_id"].as_str().unwrap_or_default())
            {
                None => Value::Null,
                Some((turn, status, result)) => json!({"session_id": data["session_id"],
                    "turn_id": turn, "status": status, "step": 3, "turn_count": 1, "children": [],
                    "expects_wake": false, "result": result}),
            },
            "harness::send" => {
                let session = data["session_id"].as_str().unwrap_or_default().to_string();
                let turn = format!("turn-{session}");
                self.sessions.entry(session.clone()).or_insert((
                    turn.clone(),
                    "running".into(),
                    Value::Null,
                ));
                json!({"session_id": session, "turn_id": turn, "accepted": true})
            }
            "harness::metrics" => {
                json!({"totals": {"cost_usd": 0.42, "input_tokens": 9000, "output_tokens": 700}})
            }
            "harness::stop" => json!({"stopped": true}),
            "judge::evaluate" if self.judge_failures > 0 => {
                self.judge_failures -= 1;
                json!({"status": "error", "code": "http", "http_status": 402,
                    "stats": {"attempts": 1, "requests": 1, "questions": 1, "input_tokens": 0,
                        "output_tokens": 0, "elapsed_ms": 5, "usage_complete": false}})
            }
            "judge::evaluate" => {
                let mut probabilities = json!({"relevant_opportunity": 0.05,
                    "expected_behavior": 0.05, "insufficient_evidence": 0.05});
                probabilities[self.judge_choice] = json!(0.9);
                json!({"status": "ok", "model": "jev-test",
                    "results": {"findings": {"answers": {"opportunity": {"type": "choice",
                        "choice": self.judge_choice, "confidence": 0.9,
                        "probabilities": probabilities}}}},
                    "stats": {"attempts": 1, "requests": 1, "questions": 1, "input_tokens": 10,
                        "output_tokens": 2, "elapsed_ms": 5, "usage_complete": true}})
            }
            other => return Some(Err(format!("unexpected call to {other}"))),
        }))
    }

    /// Completes a Harness session's turn with `result`.
    fn complete_session(&mut self, prefix: &str, result: Value) {
        let session = self
            .sessions
            .keys()
            .find(|session| session.starts_with(prefix))
            .cloned()
            .expect("session started");
        let entry = self.sessions.get_mut(&session).unwrap();
        entry.1 = "completed".into();
        entry.2 = result;
    }
}

// ---------------------------------------------------------------------------
// GitHub over HTTP
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Services {
    candidate_sha: String,
    /// Every GitHub call answers 401 (missing or expired access).
    deny: bool,
    /// Answers the next pull request creation with 502.
    fail_create: bool,
    /// Opens the next pull request but drops the connection unanswered.
    lose_create: bool,
    /// The pull requests opened so far.
    pulls: Vec<Value>,
    seen: Vec<(String, String)>,
}

impl Services {
    /// `None` drops the connection unanswered.
    fn respond(&mut self, key: &str, body: &str) -> Option<(u16, Value)> {
        self.seen.push((key.into(), body.into()));
        if self.deny {
            return Some((401, json!({"message": "Bad credentials"})));
        }
        let repo = "/repos/iii-hq/workers";
        let body_json: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        Some(match key {
            k if k.starts_with(&format!("GET {repo}/git/ref/heads/feat/")) => {
                (200, json!({"object": {"sha": self.candidate_sha}}))
            }
            k if k.starts_with(&format!("GET {repo}/pulls?")) => (200, json!(self.pulls)),
            k if k == format!("POST {repo}/pulls") => {
                if std::mem::take(&mut self.fail_create) {
                    return Some((502, json!({})));
                }
                let number = 7 + self.pulls.len() as u64;
                let pull = json!({"number": number, "state": "open", "merged_at": null,
                    "html_url": format!("https://github.com/iii-hq/workers/pull/{number}"),
                    "head": {"sha": self.candidate_sha, "ref": body_json["head"]},
                    "base": {"ref": body_json["base"]}});
                self.pulls.push(pull.clone());
                if std::mem::take(&mut self.lose_create) {
                    return None;
                }
                (201, pull)
            }
            other => (404, json!({"error": format!("no route {other}")})),
        })
    }

    fn created(&self) -> Vec<Value> {
        self.seen
            .iter()
            .filter(|(key, _)| key == "POST /repos/iii-hq/workers/pulls")
            .map(|(_, body)| serde_json::from_str(body).unwrap())
            .collect()
    }
}

struct Http {
    url: String,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for Http {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn serve(services: Arc<Mutex<Services>>) -> Http {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let services = services.clone();
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 4096];
                let header_end = loop {
                    let read = socket.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    if let Some(at) = buffer.windows(4).position(|window| window == b"\r\n\r\n") {
                        break at + 4;
                    }
                };
                let head = String::from_utf8_lossy(&buffer[..header_end]).to_string();
                let length: usize = head
                    .lines()
                    .filter_map(|line| line.split_once(':'))
                    .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .and_then(|(_, value)| value.trim().parse().ok())
                    .unwrap_or(0);
                while buffer.len() < header_end + length {
                    let read = socket.read(&mut chunk).await.unwrap_or(0);
                    if read == 0 {
                        break;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                }
                let body = String::from_utf8_lossy(&buffer[header_end..]).to_string();
                let mut parts = head.lines().next().unwrap_or_default().split(' ');
                let key = format!(
                    "{} {}",
                    parts.next().unwrap_or_default(),
                    parts.next().unwrap_or_default()
                );
                let answer = services.lock().unwrap().respond(&key, &body);
                let Some((status, value)) = answer else {
                    return;
                };
                let text = value.to_string();
                let response = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    Http { url, task }
}

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// The rig
// ---------------------------------------------------------------------------

fn roots(repo: &Path) -> runtime::CandidateRoots {
    std::fs::create_dir_all(repo.join(".worktrees")).unwrap();
    runtime::CandidateRoots {
        repository: repo.display().to_string(),
        worktrees_dir: repo.join(".worktrees").display().to_string(),
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

async fn connect(url: &str) -> Arc<IIIClient> {
    let iii = Arc::new(register_worker(
        url,
        InitOptions {
            namespace: Some("my-project".into()),
            ..InitOptions::default()
        },
    ));
    // A local socket, but a loaded machine can starve the connection
    // thread for seconds.
    tokio::time::timeout(Duration::from_secs(30), async {
        while iii.get_connection_state() != iii_sdk::runtime::IIIConnectionState::Connected {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("connects to the fake engine");
    iii
}

fn engine_reply(frame: &Value, reply: Option<Result<Value, String>>) -> Vec<Value> {
    let Some(reply) = reply else { return vec![] };
    let mut response = json!({"type": "invocationresult", "invocation_id": frame["invocation_id"],
        "function_id": frame["function_id"]});
    match reply {
        Ok(result) => response["result"] = result,
        Err(message) => response["error"] = json!({"code": "mock", "message": message}),
    }
    vec![response]
}

struct Rig {
    world: Arc<Mutex<World>>,
    services: Arc<Mutex<Services>>,
    deps: Deps,
    iii: Arc<IIIClient>,
    repo: PathBuf,
    base_sha: String,
    http_url: String,
    _http: Http,
    _engine: fake_engine::FakeEngine,
    scratch: PathBuf,
    /// One rig at a time: the credential variable is process-global and
    /// setenv races with every getenv (process spawn included) of a
    /// concurrent test.
    _serial: tokio::sync::MutexGuard<'static, ()>,
}

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
        // A client's connection thread outlives it and keeps redialling its
        // stopped engine: a later rig's one-connection fake engine on the
        // same port would serve it instead of that rig's own client.
        let iii = self.iii.clone();
        std::thread::spawn(move || iii.shutdown());
    }
}

impl Rig {
    async fn start(_tag: &str) -> Option<Self> {
        let serial = SERIAL.lock().await;
        let scratch = std::env::temp_dir().join(format!("eval-lifecycle-{}", uuid::Uuid::new_v4()));
        let repo = scratch.join("repo");
        std::fs::create_dir_all(repo.join("harness/src")).unwrap();
        git(&repo, &["init", "-q", "-b", "main"]);
        git(&repo, &["config", "user.name", "fixture"]);
        git(&repo, &["config", "user.email", "fixture@example.invalid"]);
        std::fs::write(repo.join(".gitignore"), "/.worktrees/\n").unwrap();
        std::fs::write(
            repo.join("harness/Cargo.toml"),
            "[package]\nname = \"harness\"\n",
        )
        .unwrap();
        std::fs::write(repo.join("harness/src/lib.rs"), "pub fn window() {}\n").unwrap();
        git(&repo, &["add", "-A"]);
        git(&repo, &["commit", "-q", "-m", "base"]);
        let base_sha = git(&repo, &["rev-parse", "HEAD"]);
        // The official origin the modules require; nothing is ever pushed.
        git(
            &repo,
            &[
                "remote",
                "add",
                "origin",
                "https://github.com/iii-hq/workers.git",
            ],
        );

        let world = Arc::new(Mutex::new(World::new()));
        let shared = world.clone();
        let engine = fake_engine::start(move |frame: Value| {
            if frame["type"] != "invokefunction" || frame["invocation_id"].is_null() {
                return vec![];
            }
            let function = frame["function_id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let mut world = shared.lock().unwrap();
            world.calls.push((function.clone(), frame["data"].clone()));
            let reply = world.respond(&function, &frame["data"]);
            engine_reply(&frame, reply)
        })
        .await;
        let services = Arc::new(Mutex::new(Services::default()));
        let http = serve(services.clone()).await;
        let iii = connect(&engine.url).await;
        let rig = Self {
            world,
            services,
            deps: Deps::new(iii.clone())
                .with_github_api(http.url.clone())
                .with_candidate_roots(roots(&repo)),
            iii,
            repo,
            base_sha,
            http_url: http.url.clone(),
            _http: http,
            _engine: engine,
            scratch,
            _serial: serial,
        };
        rig.credentials(true);
        Some(rig)
    }

    /// The module reads its credential from a fixed variable; access is
    /// denied (401) by the service rather than by racing on it.
    fn credentials(&self, present: bool) {
        // Under SERIAL: no concurrent test reads the environment now.
        std::env::set_var(eval::pull_request::GITHUB_TOKEN_ENV, "secret-github");
        self.services.lock().unwrap().deny = !present;
    }

    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }

    /// A fresh process on the same storage.
    fn restart(&mut self) {
        self.deps = Deps::new(self.iii.clone())
            .with_github_api(self.http_url.clone())
            .with_candidate_roots(roots(&self.repo));
    }

    async fn configure(&self) {
        let config = json!({
            "subject": {"provider": "p", "model": "m"}, "where": "harness",
            "runs_per_side": RUNS, "technical_retries": 0, "max_scenarios": 2,
            "estimated_usd_per_run": 0.5,
            "min_effect": {"tokens": 1000.0, "pass_rate_pp": 10.0},
            "budget": {"max_spend_usd": 100.0, "max_duration_ms": 3_600_000, "max_calls": 50,
                       "max_candidates": 1},
            "controls": [CONTROL],
            "judge": {"provider": "typesafe", "estimated_usd": 0.01},
            "candidate": {"repository": self.repo.display().to_string(), "base_sha": self.base_sha,
                "worktrees_dir": self.repo.join(".worktrees").display().to_string(),
                "model": "actor-model", "provider": "actor-provider", "max_turns": 12,
                "max_output_tokens": 4000, "max_total_tokens": 200000, "max_cost_usd": 1.5,
                "publish_remote": null},
            "validation": {"where": "docker", "stack_name": "loop",
                "source_stack_template": SOURCE_TEMPLATE},
        });
        runtime::configure(
            &self.deps,
            serde_json::from_value(json!({"config": config})).unwrap(),
        )
        .await
        .unwrap();
    }

    async fn drain(&self) {
        loop {
            let next = self.world().steps.pop_front();
            let Some(payload) = next else { break };
            Box::pin(runtime::step(
                &self.deps,
                serde_json::from_value(payload).unwrap(),
            ))
            .await
            .unwrap();
        }
    }

    /// One sweep (the cron) and every step it queued.
    async fn tick(&self) {
        Box::pin(runtime::sweep(&self.deps)).await.unwrap();
        self.drain().await;
    }

    async fn begin(&self, request: Value) -> String {
        let cycle_id = runtime::cycle_start(&self.deps, serde_json::from_value(request).unwrap())
            .await
            .unwrap()
            .cycle_id;
        self.drain().await;
        cycle_id
    }

    async fn cycle(&self, cycle_id: &str) -> CycleRecordV1 {
        state::get_cycle(&self.iii, cycle_id)
            .await
            .unwrap()
            .expect("cycle exists")
    }

    async fn changed(&self, execution_id: &str) {
        Box::pin(runtime::execution_changed(
            &self.deps,
            serde_json::from_value(json!({"kind": "completed", "execution_id": execution_id}))
                .unwrap(),
        ))
        .await
        .unwrap();
        self.drain().await;
    }

    fn base(&self) -> System {
        System {
            workers_revision: self.base_sha.clone(),
            harness_version: "h-base".into(),
            binary: Some(format!("sha256:{}", sha256(b"base harness build"))),
        }
    }

    /// Objective → baseline measured → proposal → candidate committed and
    /// published → its source validation dispatched. Returns the cycle and
    /// the candidate commit.
    /// A finished E2E execution whose needles criterion lost points: the
    /// source entry, covered by its current scenario and judged once.
    async fn begin_source(&self) -> String {
        let parameters = json!({"model": "m", "provider": "p", "runs": RUNS,
            "scenarios": [CONTEXT], "technical_retries": 0});
        let base = self.base();
        let mut runs = side(TOKENS, LOST);
        runs.truncate(1);
        self.world().e2e.executions.insert(
            "source".into(),
            bundle("source", "nightly", "passed", &parameters, runs, &base),
        );
        self.begin(json!({"execution_id": "source"})).await
    }

    async fn reach_validation(&mut self) -> (String, String) {
        Box::pin(self.reach_validation_changing(&["harness/src/lib.rs"])).await
    }

    /// [`Self::reach_validation`] with the actor writing `files`.
    async fn reach_validation_changing(&mut self, files: &[&str]) -> (String, String) {
        let cycle_id = self.begin_source().await;
        let record = self.cycle(&cycle_id).await;
        assert_eq!(
            record.stage,
            CycleStageV1::AwaitingExecution,
            "{:?}",
            record.history
        );
        let baseline_stack = self.world().e2e.dispatched[0].2["stack"]["yaml"].clone();
        assert!(
            baseline_stack.as_str().unwrap().contains(&self.base_sha),
            "the baseline pins the base"
        );
        let base = self.base();
        self.world()
            .e2e
            .finish("plan-1", "passed", side(TOKENS, LOST), &base);
        self.changed("plan-1").await;
        let record = self.cycle(&cycle_id).await;
        assert_eq!(
            (record.status, record.stage),
            (CycleStatusV1::Waiting, CycleStageV1::Proposal),
            "{:?}",
            record.history
        );
        assert_eq!(
            self.world().calls_to("harness::send").len(),
            1,
            "one proposal dispatch"
        );

        self.world().complete_session(
            "loop-proposal-",
            json!({
            "verdict": "proposal", "hypothesis": "window() keeps every distractor document",
            "change": "summarise ingested documents in window()",
            "citations": [{"path": "harness/src/lib.rs", "line_start": 1, "line_end": 1}],
            "insufficient_reason": ""}),
        );
        self.tick().await;
        let record = self.cycle(&cycle_id).await;
        assert_eq!(
            (record.status, record.stage),
            (CycleStatusV1::Waiting, CycleStageV1::Candidate),
            "{:?}",
            record.history
        );
        // A restart while the actor runs reuses its identity: no second send.
        self.restart();
        self.tick().await;
        assert_eq!(
            self.world().calls_to("harness::send").len(),
            2,
            "proposal + one candidate"
        );

        let identity = record.candidate.as_ref().unwrap().identity.clone();
        for file in files {
            let written = Path::new(&identity.root).join(file);
            std::fs::create_dir_all(written.parent().unwrap()).unwrap();
            std::fs::write(written, "pub fn window() { summarise(); }\n").unwrap();
        }
        self.world().complete_session("loop-candidate-", json!({}));
        // The push to the official origin is the one effect an offline test
        // cannot perform (and must never attempt): its recorded result is
        // injected for the commit the module itself makes.
        let candidate_config = record
            .config
            .as_ref()
            .unwrap()
            .config
            .candidate
            .clone()
            .unwrap();
        let commit = eval::candidate::collect_commit(&candidate_config, &identity)
            .unwrap()
            .unwrap();
        state::update_cycle(&self.iii, &cycle_id, |stored| {
            let mut next = stored.clone();
            next.candidate.as_mut().unwrap().publish = Some(eval::candidate::CandidatePublishV1 {
                remote: "origin".into(),
                branch: identity.branch.clone(),
                candidate_sha: commit.candidate_sha.clone(),
                builder_ref: format!("commit:{}", commit.candidate_sha),
            });
            Ok((Some(next), ()))
        })
        .await
        .unwrap();
        self.tick().await;
        let record = self.cycle(&cycle_id).await;
        assert_eq!(
            record.stage,
            CycleStageV1::AwaitingExecution,
            "{:?}",
            record.history
        );
        let state = record.candidate.clone().unwrap();
        let commit = state.candidate.unwrap().commit.unwrap();
        assert_eq!(commit.base_sha, self.base_sha);
        assert_eq!(
            commit
                .paths
                .iter()
                .map(|path| path.path.as_str())
                .collect::<Vec<_>>(),
            files
        );
        assert_eq!(
            state.publish.as_ref().unwrap().candidate_sha,
            commit.candidate_sha
        );
        let validation = self.world().e2e.dispatched[1].clone();
        assert_eq!(validation.1, format!("eval:{cycle_id}:candidate"));
        // The actor and the run validating its commit are two reservations.
        let ledger = state::get_ledger(&self.iii, &cycle_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            ledger.reservations[&format!("{cycle_id}:implementation")]
                .work
                .kind,
            CycleWorkKindV1::Implementation
        );
        assert_eq!(
            ledger.reservations[&format!("{cycle_id}:candidate")]
                .work
                .kind,
            CycleWorkKindV1::E2e
        );
        assert!(validation.2["stack"]["yaml"]
            .as_str()
            .unwrap()
            .contains(&commit.candidate_sha));
        self.services.lock().unwrap().candidate_sha = commit.candidate_sha.clone();
        (cycle_id, commit.candidate_sha)
    }

    /// … then the validation ran the candidate commit and held every
    /// criterion: the source decision is accepted.
    async fn reach_accepted(&mut self, files: &[&str]) -> String {
        let (cycle_id, candidate_sha) = Box::pin(self.reach_validation_changing(files)).await;
        self.world().e2e.finish(
            "plan-2",
            "passed",
            side(TOKENS, FIXED),
            &candidate_system(candidate_sha),
        );
        self.changed("plan-2").await;
        let record = self.cycle(&cycle_id).await;
        assert_eq!(
            record.decision.as_ref().unwrap().outcome,
            DecisionOutcomeV1::Accepted,
            "{:?}",
            record.decision
        );
        cycle_id
    }
}

fn candidate_system(sha: String) -> System {
    System {
        workers_revision: sha,
        harness_version: "h-candidate".into(),
        binary: Some(format!("sha256:{}", sha256(b"candidate harness build"))),
    }
}

// ---------------------------------------------------------------------------
// Trajectories
// ---------------------------------------------------------------------------

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn an_accepted_candidate_is_proposed_in_a_pull_request_ready_for_review() {
    let Some(mut rig) = Rig::start("proposed").await else {
        return;
    };
    rig.configure().await;
    let cycle_id = Box::pin(rig.reach_accepted(&["harness/src/lib.rs"])).await;
    let record = rig.cycle(&cycle_id).await;
    let outcome = record.outcome.clone().unwrap();
    assert_eq!(
        (record.status, outcome.code),
        (CycleStatusV1::Completed, OutcomeCodeV1::Proposed),
        "{} {:?}",
        outcome.detail,
        record.history
    );
    let pull = record.pull_request.clone().unwrap();
    assert_eq!((pull.number, pull.created), (7, true));
    assert!(outcome.detail.contains(&pull.url), "{}", outcome.detail);
    let services = rig.services.lock().unwrap();
    let created = services.created();
    assert_eq!(created.len(), 1);
    let sent = &created[0];
    assert_eq!(sent["draft"], false, "ready for review");
    assert_eq!(sent["base"], "main");
    assert_eq!(
        sent["head"],
        record.candidate.as_ref().unwrap().identity.branch
    );
    assert_eq!(
        sent["title"],
        "eval: summarise ingested documents in window()"
    );
    let body = sent["body"].as_str().unwrap();
    for expected in [
        "Objective: ",
        "Hypothesis: window() keeps every distractor document",
        "### Measured",
        "| goal | needles_recovered | context_pressure | 0.500 | 1.000 | 5 / 5 | ValidatedImprovement |",
        "| kept | path_completed | minimal_path |",
        "### Transcript findings",
        "Touches `harness`. Measured in `harness`",
        "Cycle `",
    ] {
        assert!(body.contains(expected), "{expected}: {body}");
    }
    assert!(
        !services
            .seen
            .iter()
            .any(|(key, _)| key.starts_with("PATCH ") || key.contains("/merge")),
        "nothing is merged: {:?}",
        services.seen
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_change_measured_in_two_workers_names_both_in_its_pull_request() {
    let Some(mut rig) = Rig::start("two-workers").await else {
        return;
    };
    rig.configure().await;
    let cycle_id =
        Box::pin(rig.reach_accepted(&["context-manager/src/lib.rs", "harness/src/lib.rs"])).await;
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(
        record.outcome.as_ref().unwrap().code,
        OutcomeCodeV1::Proposed
    );
    let created = rig.services.lock().unwrap().created();
    let body = created[0]["body"].as_str().unwrap().to_string();
    assert!(
        body.contains(
            "Touches `context-manager`, `harness`. Measured in `context-manager`, `harness`"
        ),
        "{body}"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_lost_pull_request_answer_is_found_on_the_next_sweep_never_opened_twice() {
    let Some(mut rig) = Rig::start("lost-pull").await else {
        return;
    };
    rig.configure().await;
    // GitHub fails once, then opens the pull request and loses the answer.
    {
        let mut services = rig.services.lock().unwrap();
        services.fail_create = true;
        services.lose_create = true;
    }
    let cycle_id = Box::pin(rig.reach_accepted(&["harness/src/lib.rs"])).await;
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(
        (record.status, record.stage),
        (CycleStatusV1::Waiting, CycleStageV1::PullRequest),
        "{:?}",
        record.history.last()
    );
    assert!(
        record
            .history
            .last()
            .unwrap()
            .detail
            .contains("asked again on the next sweep"),
        "{:?}",
        record.history.last()
    );
    rig.tick().await;
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(
        record.outcome.as_ref().map(|outcome| outcome.code),
        Some(OutcomeCodeV1::Proposed),
        "{:?}",
        record.history
    );
    assert_eq!(record.pull_request.as_ref().unwrap().number, 7);
    let services = rig.services.lock().unwrap();
    assert_eq!(
        services.pulls.len(),
        1,
        "one pull request, found after the lost answer"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn without_github_access_an_accepted_candidate_stays_on_its_branch() {
    let Some(mut rig) = Rig::start("nocreds").await else {
        return;
    };
    rig.configure().await;
    rig.credentials(false);
    let cycle_id = Box::pin(rig.reach_accepted(&["harness/src/lib.rs"])).await;
    let record = rig.cycle(&cycle_id).await;
    let outcome = record.outcome.clone().unwrap();
    assert_eq!(outcome.code, OutcomeCodeV1::Decided, "{}", outcome.detail);
    let branch = &record.candidate.as_ref().unwrap().identity.branch;
    assert!(
        outcome.detail.contains("no pull request")
            && outcome.detail.contains("token is missing or expired")
            && outcome.detail.contains(branch.as_str()),
        "{}",
        outcome.detail
    );
    assert!(record.pull_request.is_none());
    assert!(
        !serde_json::to_string(&record).unwrap().contains("secret-"),
        "no credential in the record"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_cheaper_candidate_that_loses_a_criterion_is_rejected_and_never_proposed() {
    let Some(mut rig) = Rig::start("rejected").await else {
        return;
    };
    rig.configure().await;
    let (cycle_id, candidate_sha) = Box::pin(rig.reach_validation()).await;
    rig.world().e2e.finish(
        "plan-2",
        "passed",
        side(1.0, [Some(0), Some(10), Some(10), Some(10)]),
        &candidate_system(candidate_sha),
    );
    rig.changed("plan-2").await;
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(
        record.decision.as_ref().unwrap().outcome,
        DecisionOutcomeV1::Rejected
    );
    assert_eq!(record.outcome.unwrap().code, OutcomeCodeV1::Decided);
    assert!(record.pull_request.is_none());
    assert!(
        rig.services.lock().unwrap().seen.is_empty(),
        "no GitHub call"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_validation_that_did_not_run_the_candidate_commit_is_inconclusive() {
    let Some(mut rig) = Rig::start("wrongsha").await else {
        return;
    };
    rig.configure().await;
    let (cycle_id, _) = Box::pin(rig.reach_validation()).await;
    // The E2E recorded the base again: what the actor committed is not proof.
    let base = rig.base();
    rig.world()
        .e2e
        .finish("plan-2", "passed", side(TOKENS, FIXED), &base);
    rig.changed("plan-2").await;
    let decision = rig.cycle(&cycle_id).await.decision.unwrap();
    assert_eq!(
        decision.outcome,
        DecisionOutcomeV1::Inconclusive,
        "{}",
        decision.reason
    );
    assert!(
        decision.reason.contains("candidate_revision"),
        "{}",
        decision.reason
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_while_the_actor_runs_stops_it_and_later_steps_do_nothing() {
    let Some(rig) = Rig::start("cancel").await else {
        return;
    };
    rig.configure().await;
    let cycle_id = rig.begin_source().await;
    let base = rig.base();
    rig.world()
        .e2e
        .finish("plan-1", "passed", side(TOKENS, LOST), &base);
    rig.changed("plan-1").await;
    assert_eq!(rig.cycle(&cycle_id).await.stage, CycleStageV1::Proposal);
    let cancelled = runtime::cycle_cancel(
        &rig.deps,
        CycleIdRequestV1 {
            cycle_id: cycle_id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(cancelled.cancelled);
    let stops = rig.world().calls_to("harness::stop");
    assert_eq!(stops.len(), 1);
    assert!(stops[0]["session_id"]
        .as_str()
        .unwrap()
        .starts_with("loop-proposal-"));
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(record.status, CycleStatusV1::Cancelled);
    assert!(record.candidate.is_none());
    assert_eq!(rig.world().calls_to("harness::send").len(), 1);
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn an_unisolated_harness_keeps_the_case_inconclusive_before_any_dispatch() {
    let Some(rig) = Rig::start("roots").await else {
        return;
    };
    rig.world().boundary = "configured_roots";
    rig.configure().await;
    let cycle_id = rig.begin_source().await;
    let base = rig.base();
    rig.world()
        .e2e
        .finish("plan-1", "passed", side(TOKENS, LOST), &base);
    rig.changed("plan-1").await;
    let record = rig.cycle(&cycle_id).await;
    let outcome = record.outcome.unwrap();
    assert_eq!(
        outcome.code,
        OutcomeCodeV1::Inconclusive,
        "{}",
        outcome.detail
    );
    assert!(outcome.detail.contains("boundary"), "{}", outcome.detail);
    assert!(
        rig.world().calls_to("harness::send").is_empty(),
        "no model is dispatched"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_passed_execution_with_a_lost_award_is_covered_then_judged_once_then_rebaselined() {
    let Some(rig) = Rig::start("source").await else {
        return;
    };
    rig.configure().await;
    let parameters = json!({"model": "m", "provider": "p", "runs": RUNS,
        "scenarios": [CONTEXT], "technical_retries": 0});
    let base = rig.base();
    rig.world().e2e.executions.insert(
        "nightly".into(),
        bundle(
            "nightly",
            "nightly",
            "passed",
            &parameters,
            vec![(
                CONTEXT,
                vec![run(
                    "r1",
                    "passed",
                    90_000.0,
                    &[Some(10), Some(5), Some(10), Some(10)],
                    &CONTEXT_CRITERIA,
                )],
            )],
            &base,
        ),
    );
    let cycle_id = rig.begin(json!({"execution_id": "nightly"})).await;
    let record = rig.cycle(&cycle_id).await;
    // Covered before anything is paid; then the ambiguous loss is judged once.
    assert!(record.coverage.as_ref().unwrap().covered);
    let triage = record.triage.clone().expect("triaged");
    assert_eq!(triage.source, TriageSourceV1::Jev);
    assert_eq!(triage.outcome, TriageOutcomeV1::Proceed);
    assert!(
        triage.request_id.is_some() && triage.answer.is_some() && triage.evidence_digest.is_some()
    );
    assert_eq!(rig.world().calls_to("judge::evaluate").len(), 1);
    // The judge reads what the scenario expects and what the lost criterion asks.
    let asked = &rig.world().calls_to("judge::evaluate")[0]["evaluations"][0]["state"];
    assert_eq!(
        asked["scenarios"][0]["summary"],
        format!("What {CONTEXT} expects.")
    );
    assert_eq!(asked["scenarios"][0]["runs"], 1);
    assert_eq!(
        asked["findings"][0]["criterion_asks"],
        "Asks for needles_recovered?"
    );
    let plan = record.plan.clone().expect("plan frozen");
    assert_eq!(plan.criterion.metric, CriterionMetricV1::CriterionScore);
    assert_eq!(
        plan.criterion.criterion_id.as_deref(),
        Some("needles_recovered")
    );
    assert_eq!(
        record.stage,
        CycleStageV1::AwaitingExecution,
        "{:?}",
        record.history
    );
    assert_eq!(
        rig.world().e2e.dispatched[0].1,
        format!("eval:{cycle_id}:baseline")
    );
    // Redelivery never opens another cycle or judges twice.
    rig.changed("nightly").await;
    rig.tick().await;
    assert_eq!(
        rig.world()
            .state
            .keys()
            .filter(|(scope, _)| scope == state::CYCLE_SCOPE)
            .count(),
        1
    );
    assert_eq!(rig.world().calls_to("judge::evaluate").len(), 1);
    // ...nor wakes the cycle: it still waits for its baseline.
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(
        (record.stage, record.status),
        (CycleStageV1::AwaitingExecution, CycleStatusV1::Waiting),
        "{:?}",
        record.history
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_judge_that_does_not_answer_is_asked_again_on_the_next_sweep() {
    let Some(rig) = Rig::start("jev-retry").await else {
        return;
    };
    rig.configure().await;
    rig.world().judge_failures = 1;
    let parameters = json!({"model": "m", "provider": "p", "runs": RUNS,
        "scenarios": [CONTEXT], "technical_retries": 0});
    let base = rig.base();
    rig.world().e2e.executions.insert(
        "nightly".into(),
        bundle(
            "nightly",
            "nightly",
            "passed",
            &parameters,
            vec![(
                CONTEXT,
                vec![run(
                    "r1",
                    "passed",
                    90_000.0,
                    &[Some(10), Some(5), Some(10), Some(10)],
                    &CONTEXT_CRITERIA,
                )],
            )],
            &base,
        ),
    );
    let cycle_id = rig.begin(json!({"execution_id": "nightly"})).await;
    // Out of credits: the cycle waits in triage instead of ending.
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(record.stage, CycleStageV1::Triage);
    assert!(record.outcome.is_none());
    let reason = record.triage.clone().expect("asked").reason;
    assert!(reason.contains("did not answer (Http)"), "{reason}");
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    let triage = record.triage.clone().expect("triaged");
    assert_eq!(
        triage.outcome,
        TriageOutcomeV1::Proceed,
        "{}",
        triage.reason
    );
    assert_eq!(record.stage, CycleStageV1::AwaitingExecution);
    let calls = rig.world().calls_to("judge::evaluate");
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0]["request_id"], calls[1]["request_id"]);
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn an_uncovered_execution_spends_nothing() {
    let Some(rig) = Rig::start("uncovered").await else {
        return;
    };
    rig.configure().await;
    // A scenario the catalog does not hold in its executed definition.
    let parameters = json!({"model": "m", "provider": "p", "runs": RUNS,
        "scenarios": ["skills_probe"], "technical_retries": 0});
    let base = rig.base();
    rig.world().e2e.executions.insert(
        "probe".into(),
        bundle(
            "probe",
            "probe",
            "passed",
            &parameters,
            vec![(
                "skills_probe",
                vec![run(
                    "r1",
                    "passed",
                    1.0,
                    &[Some(10), Some(5), Some(10), Some(10)],
                    &CONTEXT_CRITERIA,
                )],
            )],
            &base,
        ),
    );
    let cycle_id = rig.begin(json!({"execution_id": "probe"})).await;
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(record.outcome.unwrap().code, OutcomeCodeV1::NoCoverage);
    assert!(rig.world().calls_to("judge::evaluate").is_empty());
    assert!(rig.world().e2e.dispatched.is_empty());
    assert!(rig.world().calls_to("harness::send").is_empty());
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_forged_candidate_repository_is_refused_before_any_git_or_actor() {
    let Some(rig) = Rig::start("forged").await else {
        return;
    };
    // A repository whose origin claims to be official but whose own config
    // runs a helper: never selectable by a configure caller.
    let forged = rig.scratch.join("forged");
    std::fs::create_dir_all(forged.join(".worktrees")).unwrap();
    git(&forged, &["init", "-q", "-b", "main"]);
    git(
        &forged,
        &[
            "remote",
            "add",
            "origin",
            "https://github.com/iii-hq/workers.git",
        ],
    );
    let marker = rig.scratch.join("helper-ran");
    git(
        &forged,
        &[
            "config",
            "core.sshCommand",
            &format!("touch {}", marker.display()),
        ],
    );
    let config = json!({
        "subject": {"provider": "p", "model": "m"}, "where": "harness",
        "runs_per_side": RUNS, "technical_retries": 0, "max_scenarios": 1,
        "estimated_usd_per_run": 0.5, "min_effect": {"tokens": 1000.0, "pass_rate_pp": 10.0},
        "budget": {"max_spend_usd": 100.0, "max_duration_ms": 3_600_000, "max_calls": 50, "max_candidates": 1},
        "candidate": {"repository": forged.display().to_string(), "base_sha": rig.base_sha,
            "worktrees_dir": forged.join(".worktrees").display().to_string(),
            "model": "actor-model", "provider": "actor-provider", "max_turns": 12,
            "max_output_tokens": 4000, "max_total_tokens": 200000, "max_cost_usd": 1.5}});
    let refused = runtime::configure(
        &rig.deps,
        serde_json::from_value(json!({"config": config})).unwrap(),
    )
    .await;
    assert!(
        matches!(refused, Err(eval::error::EvalError::InvalidRequest(_))),
        "{refused:?}"
    );
    assert!(
        state::get_config(&rig.iii).await.unwrap().is_none(),
        "nothing stored"
    );
    assert!(!marker.exists(), "no git process ran");
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_cycle_resumed_under_other_candidate_roots_stops_before_paid_work() {
    let Some(mut rig) = Rig::start("bootstrap").await else {
        return;
    };
    rig.configure().await;
    let cycle_id = rig.begin_source().await;
    // The deployment now trusts another repository.
    let other = rig.scratch.join("other");
    std::fs::create_dir_all(&other).unwrap();
    rig.deps = Deps::new(rig.iii.clone())
        .with_github_api(rig.http_url.clone())
        .with_candidate_roots(roots(&other));
    let base = rig.base();
    rig.world()
        .e2e
        .finish("plan-1", "passed", side(TOKENS, LOST), &base);
    rig.changed("plan-1").await;
    let record = rig.cycle(&cycle_id).await;
    let outcome = record.outcome.unwrap();
    assert_eq!(
        outcome.code,
        OutcomeCodeV1::Inconclusive,
        "{}",
        outcome.detail
    );
    assert!(
        outcome.detail.contains("candidate authority"),
        "{}",
        outcome.detail
    );
    assert!(rig.world().calls_to("harness::send").is_empty());
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_token_objective_is_planned_and_judged_by_what_the_candidate_changes() {
    let Some(rig) = Rig::start("tokens").await else {
        return;
    };
    rig.configure().await;
    let cycle_id = rig.begin(json!({"objective": OBJECTIVE})).await;
    let record = rig.cycle(&cycle_id).await;
    // Whether the totals are independent depends on the candidate's diff
    // (see the decision tests); the cycle is not stopped before one exists.
    let plan = record.plan.clone().expect("a frozen plan");
    assert_eq!(plan.criterion.metric, CriterionMetricV1::Tokens);
    assert_eq!(record.outcome, None, "{:?}", record.history);
    assert_eq!(rig.world().e2e.dispatched.len(), 1, "the baseline");
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_tampered_token_counter_without_the_criterion_fix_is_not_approved() {
    let Some(mut rig) = Rig::start("tampered").await else {
        return;
    };
    rig.configure().await;
    let (cycle_id, candidate_sha) = Box::pin(rig.reach_validation()).await;
    // The candidate reports almost no tokens but the needles criterion is
    // still lost: the self-reported counter decides nothing.
    rig.world().e2e.finish(
        "plan-2",
        "passed",
        side(1.0, LOST),
        &candidate_system(candidate_sha),
    );
    rig.changed("plan-2").await;
    let record = rig.cycle(&cycle_id).await;
    let decision = record.decision.clone().unwrap();
    assert_ne!(
        decision.outcome,
        DecisionOutcomeV1::Accepted,
        "{}",
        decision.reason
    );
    assert!(record.pull_request.is_none());
    assert!(rig.services.lock().unwrap().seen.is_empty());
}

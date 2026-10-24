//! The whole cycle driven through the controller entrypoints (`eval::start`,
//! `eval::step`, `eval::sweep`, `eval::execution-changed`, `eval::cancel`)
//! with the real candidate and promotion modules against local,
//! native-shaped mocks: the E2E and Sentinel on this worker's Engine, the
//! Harness actor sessions, a git repository with a bare remote, a tiny HTTP
//! server standing in for GitHub, Release Control and the Registry, a
//! second Engine standing in for the explicit activation target (Compose and
//! its own E2E) and real local processes whose `/proc/<pid>/exe` is hashed.
//! No paid call and no live service: this is not release qualification.

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
const RC: &str = "0.9.1-rc.1";
const STABLE: &str = "0.9.1";
const PRIOR: &str = "0.9.0";
const PLAN_HASH: &str = "eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";
const TARGET: &str = "x86_64-unknown-linux-gnu";
const NAMESPACE: &str = "promotion-target";
const SOURCE_TEMPLATE: &str = "workers:\n  harness: commit:{commit}\n  engine: 0.22.0\n";
const ARTIFACT_TEMPLATE: &str = "workers:\n  harness: {version}\n  engine: 0.22.0\n";

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
            "id": id, "possible": 10, "awarded": awarded, "gate": false, "reason": "r"}))
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
                    "Recovers early charter facts after ingesting a large distractor corpus.",
                    "Complete a context-endurance run over 48 large documents."),
                row(CONTROL, "sha256:min", &[("path_completed", "The minimal path completed.")],
                    "The smallest end-to-end task.", "Do the minimal task."),
            ]})],
            catalog: json!({"catalog_sha256": "sha256:cat", "scenarios": [scenario(CONTEXT), scenario(CONTROL)]}),
            boundary: "workspace",
            sessions: HashMap::new(),
            judge_choice: "relevant_opportunity",
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
            "judge::evaluate" => json!({"status": "ok", "model": "jev-test",
                "results": {"findings": {"answers": {"opportunity": {"type": "choice",
                    "choice": self.judge_choice, "confidence": 0.9,
                    "probabilities": {"relevant_opportunity": 0.9, "expected_behavior": 0.05,
                        "insufficient_evidence": 0.05}}}}},
                "stats": {"attempts": 1, "requests": 1, "questions": 1, "input_tokens": 10,
                    "output_tokens": 2, "elapsed_ms": 5, "usage_complete": true}}),
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
// The explicit activation target: Compose plus its own E2E
// ---------------------------------------------------------------------------

struct Target {
    namespace: String,
    file: String,
    harness: Value,
    /// The local process that runs each version once pinned.
    pids: HashMap<String, u32>,
    operations: HashMap<String, Value>,
    calls: Vec<(String, Value)>,
    e2e: Dashboard,
    /// The next activation stays running (the old process keeps running)
    /// until the test lets it land.
    hold: bool,
    fail_after_pin: bool,
}

fn harness_row(pid: u32, version: &str) -> Value {
    json!({"container": "harness", "state": "ready", "version": version, "pid": pid,
        "owned": true, "log_path": "/tmp/harness.log"})
}

impl Target {
    fn respond(&mut self, function: &str, data: &Value) -> Result<Value, String> {
        self.calls.push((function.into(), data.clone()));
        if let Some(answer) = self.e2e.respond(function, data) {
            return answer;
        }
        match function {
            "compose::status" => Ok(json!({"namespace": self.namespace, "file": self.file,
                "daemon_pid": 1, "state_dir": "/tmp/state", "containers": [self.harness.clone()]})),
            "compose::operation" => {
                let id = data["progress_operation_id"].as_str().unwrap_or_default();
                self.operations
                    .get(id)
                    .cloned()
                    .ok_or_else(|| format!("unknown compose operation '{id}'"))
            }
            "compose::add" => {
                // The daemon pins the version and the process behind it runs.
                let id = data["operation_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let version = data["workers"][0]["version"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let pid = self.pids[&version];
                if std::mem::take(&mut self.hold) {
                    self.operations
                        .insert(id.clone(), json!({"operation_id": id, "status": "running"}));
                } else {
                    self.harness = harness_row(pid, &version);
                    self.operations.insert(
                        id.clone(),
                        json!({"operation_id": id, "status": "succeeded"}),
                    );
                }
                if std::mem::take(&mut self.fail_after_pin) {
                    self.operations.get_mut(&id).unwrap()["status"] = json!("failed");
                    return Err("activation failed after changing the target".into());
                }
                Ok(json!({"operation_id": id, "status": "accepted", "requested": 1}))
            }
            other => Err(format!("unexpected {other}")),
        }
    }

    fn adds(&self) -> Vec<Value> {
        self.calls
            .iter()
            .filter(|(function, _)| function == "compose::add")
            .map(|(_, data)| data.clone())
            .collect()
    }
}

// ---------------------------------------------------------------------------
// GitHub, Release Control and the Registry over HTTP
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Services {
    candidate_sha: String,
    candidate_tree: String,
    base_sha: String,
    /// `main` was fast-forwarded to the candidate.
    merged: bool,
    /// Every GitHub and Release Control call answers 401 (missing or
    /// expired access).
    deny: bool,
    archive_new: Vec<u8>,
    archive_prior: Vec<u8>,
    url: String,
    seen: Vec<(String, String)>,
}

impl Services {
    fn respond(&mut self, key: &str, body: &str) -> (u16, Value) {
        self.seen.push((key.into(), body.into()));
        if self.deny && !key.starts_with("GET /w/") {
            return (401, json!({"message": "Bad credentials"}));
        }
        let repo = "/repos/iii-hq/workers";
        let source = self.candidate_sha.clone();
        let body_json: Value = serde_json::from_str(body).unwrap_or(Value::Null);
        let pull = json!({"number": 7, "html_url": "https://github.com/iii-hq/workers/pull/7",
            "state": "open", "merged": false, "merged_at": null, "mergeable_state": "clean",
            "head": {"sha": source, "ref": "feat"}, "base": {"ref": "main"}});
        let main = if self.merged {
            source.clone()
        } else {
            self.base_sha.clone()
        };
        let archive = |name: &str, bytes: &[u8]| json!({(TARGET): {"url": format!("{}/{name}", self.url), "sha256": sha256(bytes)}});
        let versions = json!({"versions": [
            {"version": RC, "tags": ["next"], "binaries": archive("archive-new", &self.archive_new)},
            {"version": STABLE, "tags": ["latest"], "binaries": archive("archive-new", &self.archive_new)},
            {"version": PRIOR, "tags": [], "binaries": archive("archive-prior", &self.archive_prior)}]});
        let deployment = |id: &str, version: &str, channel: &str, from: Value| {
            json!({"operation": {"id": id},
            "deployments": [{"id": format!("dep-{id}"), "worker": "harness", "version": version, "channel": channel,
                "fromVersion": from, "sourceSha": source, "status": "converged", "statusDetail": null,
                "descriptorSha256": PLAN_HASH, "executions": [{"runId": 11, "conclusion": "success", "hasManifest": true}]}]})
        };
        match key {
            k if k.starts_with(&format!("GET {repo}/git/ref/heads/feat/")) => {
                (200, json!({"object": {"sha": source}}))
            }
            k if k == format!("GET {repo}/git/ref/heads/main") => {
                (200, json!({"object": {"sha": main}}))
            }
            k if k.starts_with(&format!("GET {repo}/pulls?")) => (200, json!([pull])),
            k if k == format!("GET {repo}/pulls/7") => (200, pull),
            k if k == format!("GET {repo}/git/commits/{source}") => (
                200,
                json!({"sha": source,
                "parents": [{"sha": self.base_sha}], "tree": {"sha": self.candidate_tree}}),
            ),
            k if k == format!("PATCH {repo}/git/refs/heads/main") => {
                assert_eq!(body_json["force"], false, "never a forced update");
                self.merged = true;
                (200, json!({"object": {"sha": source}}))
            }
            "POST /operations/preview" if body_json["type"] == "deploy" => (
                200,
                json!({
                "planId": "plan-rc", "planHash": PLAN_HASH, "expiresAt": "later",
                "preview": {"sourceSha": source, "blocked": false, "plan": [{"worker": "harness",
                    "channel": "next", "sourceSha": source, "targetVersion": RC, "blocking": null,
                    "snapshots": {"next": {"version": "0.9.0-rc.3"}, "latest": {"version": PRIOR}}}]}}),
            ),
            "POST /operations/preview" => (
                200,
                json!({
                "planId": "plan-final", "planHash": PLAN_HASH, "expiresAt": "later",
                "preview": {"blocked": false, "plan": [{"worker": "harness", "candidateVersion": RC,
                    "stableVersion": STABLE, "channel": "latest", "sourceSha": source,
                    "expectedNextVersion": RC, "expectedLatestVersion": PRIOR, "blocking": null}]}}),
            ),
            "POST /operations" if body_json["planId"] == "plan-rc" => {
                (201, json!({"operation": {"id": "op-rc"}}))
            }
            "POST /operations" => (201, json!({"operation": {"id": "op-final"}})),
            "GET /operations/op-rc" => (200, deployment("op-rc", RC, "next", Value::Null)),
            "GET /operations/op-final" => {
                (200, deployment("op-final", STABLE, "latest", json!(RC)))
            }
            "GET /w/harness/versions" => (200, versions),
            other => (404, json!({"error": format!("no route {other}")})),
        }
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
    services.lock().unwrap().url = url.clone();
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
                let payload: Vec<u8> = {
                    let mut services = services.lock().unwrap();
                    match key.as_str() {
                        "GET /archive-new" => services.archive_new.clone(),
                        "GET /archive-prior" => services.archive_prior.clone(),
                        _ => {
                            let (status, value) = services.respond(&key, &body);
                            let text = value.to_string().into_bytes();
                            let response = format!(
                                "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                                text.len()
                            );
                            let mut out = response.into_bytes();
                            out.extend(text);
                            out
                        }
                    }
                };
                let payload = if key.starts_with("GET /archive-") {
                    let mut out = format!(
                        "HTTP/1.1 200 X\r\ncontent-type: application/octet-stream\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        payload.len()
                    )
                    .into_bytes();
                    out.extend(payload);
                    out
                } else {
                    payload
                };
                let _ = socket.write_all(&payload).await;
                let _ = socket.shutdown().await;
            });
        }
    });
    Http { url, task }
}

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

fn origins(url: &str) -> eval::promotion::PromotionOrigins {
    eval::promotion::PromotionOrigins {
        release_control_url: url.into(),
        github_api_url: url.into(),
        registry_url: url.into(),
    }
}

struct Proc(std::process::Child);

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// A runnable executable named `harness` (the name the target observation
/// requires), made distinct by `salt` appended past its end.
fn harness_executable(dir: &Path, salt: &[u8]) -> PathBuf {
    use std::os::unix::fs::PermissionsExt;
    std::fs::create_dir_all(dir).unwrap();
    let source = std::fs::canonicalize(
        ["/usr/bin/python3", "/bin/python3"]
            .into_iter()
            .find(|path| Path::new(path).exists())
            .expect("python3"),
    )
    .unwrap();
    let path = dir.join("harness");
    let mut bytes = std::fs::read(source).unwrap();
    bytes.extend_from_slice(salt);
    std::fs::write(&path, bytes).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

/// A release archive as the Registry serves it: a tar.gz holding the one
/// regular `harness` executable.
fn archive(executable: &Path) -> Vec<u8> {
    let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::fast(),
    ));
    builder
        .append_path_with_name(executable, "harness")
        .unwrap();
    builder.into_inner().unwrap().finish().unwrap()
}

fn spawn(program: &str, args: &[&str]) -> (Proc, u32, String) {
    let child = Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("spawn a local process");
    let pid = child.id();
    let child = Proc(child);
    let digest = sha256(&std::fs::read(program).unwrap());
    for _ in 0..100 {
        if std::fs::read(format!("/proc/{pid}/exe")).is_ok_and(|bytes| sha256(&bytes) == digest) {
            return (child, pid, digest);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("the fixture child did not exec its expected binary");
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
    tokio::time::timeout(Duration::from_secs(10), async {
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
    target: Arc<Mutex<Target>>,
    services: Arc<Mutex<Services>>,
    deps: Deps,
    iii: Arc<IIIClient>,
    repo: PathBuf,
    base_sha: String,
    candidate_digest: String,
    prior_digest: String,
    target_url: String,
    http_url: String,
    _http: Http,
    _engines: (fake_engine::FakeEngine, fake_engine::FakeEngine),
    _procs: (Proc, Proc),
    scratch: PathBuf,
    /// One rig at a time: the credential variables are process-global and
    /// setenv races with every getenv (process spawn included) of a
    /// concurrent test.
    _serial: tokio::sync::MutexGuard<'static, ()>,
}

static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.scratch);
    }
}

impl Rig {
    async fn start(_tag: &str) -> Option<Self> {
        let serial = SERIAL.lock().await;
        let scratch = std::env::temp_dir().join(format!("eval-lifecycle-{}", uuid::Uuid::new_v4()));
        // Two different executables, each named harness: never a vacuous pass.
        let sleeper = ["-c", "import time; time.sleep(600)"];
        let prior_exe = harness_executable(&scratch.join("prior"), b"");
        let candidate_exe = harness_executable(&scratch.join("candidate"), b"candidate");
        let (prior_proc, prior_pid, prior_digest) = spawn(prior_exe.to_str().unwrap(), &sleeper);
        let (candidate_proc, candidate_pid, candidate_digest) =
            spawn(candidate_exe.to_str().unwrap(), &sleeper);
        assert_ne!(
            prior_digest, candidate_digest,
            "the prior and candidate executables differ"
        );
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
        let ours = fake_engine::start(move |frame: Value| {
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
        let compose_file = scratch.join("worker-compose.yaml");
        std::fs::write(&compose_file, "containers: {}\n").unwrap();
        let target = Arc::new(Mutex::new(Target {
            namespace: NAMESPACE.into(),
            file: compose_file.to_string_lossy().to_string(),
            harness: harness_row(prior_pid, PRIOR),
            pids: HashMap::from([
                (PRIOR.to_string(), prior_pid),
                (STABLE.to_string(), candidate_pid),
            ]),
            operations: HashMap::new(),
            calls: Vec::new(),
            e2e: Dashboard {
                prefix: "canary",
                ..Dashboard::default()
            },
            hold: false,
            fail_after_pin: false,
        }));
        let shared = target.clone();
        let theirs = fake_engine::start(move |frame: Value| {
            if frame["type"] != "invokefunction" || frame["invocation_id"].is_null() {
                return vec![];
            }
            let function = frame["function_id"]
                .as_str()
                .unwrap_or_default()
                .to_string();
            let reply = shared.lock().unwrap().respond(&function, &frame["data"]);
            engine_reply(&frame, Some(reply))
        })
        .await;
        let services = Arc::new(Mutex::new(Services {
            base_sha: base_sha.clone(),
            archive_new: archive(&candidate_exe),
            archive_prior: archive(&prior_exe),
            ..Services::default()
        }));
        let http = serve(services.clone()).await;
        let iii = connect(&ours.url).await;
        let rig = Self {
            world,
            target,
            services,
            deps: Deps::new(iii.clone())
                .with_release_origins(origins(&http.url))
                .with_candidate_roots(roots(&repo)),
            iii,
            repo,
            base_sha,
            candidate_digest,
            prior_digest,
            target_url: theirs.url.clone(),
            http_url: http.url.clone(),
            _http: http,
            _engines: (ours, theirs),
            _procs: (prior_proc, candidate_proc),
            scratch,
            _serial: serial,
        };
        rig.credentials(true);
        Some(rig)
    }

    /// The module reads its credentials from fixed variables; access is
    /// denied (401) by the services rather than by racing on them.
    fn credentials(&self, present: bool) {
        // Under SERIAL: no concurrent test reads the environment now.
        std::env::set_var(eval::promotion::GITHUB_TOKEN_ENV, "secret-github");
        std::env::set_var(
            eval::promotion::RELEASE_CONTROL_SESSION_ENV,
            "secret-session",
        );
        self.services.lock().unwrap().deny = !present;
    }

    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }

    fn target(&self) -> std::sync::MutexGuard<'_, Target> {
        self.target.lock().unwrap()
    }

    /// A fresh process on the same storage.
    fn restart(&mut self) {
        self.deps = Deps::new(self.iii.clone())
            .with_release_origins(origins(&self.http_url))
            .with_candidate_roots(roots(&self.repo));
    }

    async fn configure(&self, promotion: bool) {
        self.configure_with(promotion, 100.0).await
    }

    async fn configure_with(&self, promotion: bool, max_spend_usd: f64) {
        let mut config = json!({
            "subject": {"provider": "p", "model": "m"}, "where": "harness",
            "runs_per_side": RUNS, "technical_retries": 0, "max_scenarios": 2,
            "estimated_usd_per_run": 0.5,
            "min_effect": {"tokens": 1000.0, "pass_rate_pp": 10.0},
            "budget": {"max_spend_usd": max_spend_usd, "max_duration_ms": 3_600_000, "max_calls": 50,
                       "max_candidates": 1},
            "controls": [CONTROL],
            "judge": {"provider": "typesafe", "estimated_usd": 0.01},
            "candidate": {"repository": self.repo.display().to_string(), "base_sha": self.base_sha,
                "worktrees_dir": self.repo.join(".worktrees").display().to_string(),
                "model": "actor-model", "provider": "actor-provider", "max_turns": 12,
                "max_output_tokens": 4000, "max_total_tokens": 200000, "max_cost_usd": 1.5,
                "publish_remote": null},
            "validation": {"where": "docker", "stack_name": "loop",
                "source_stack_template": SOURCE_TEMPLATE, "artifact_stack_template": ARTIFACT_TEMPLATE},
        });
        if promotion {
            config["promotion"] = json!({"artifact_target": TARGET,
                "target": {"engine_url": self.target_url, "namespace": NAMESPACE,
                    "compose_file": self.target().file.clone()}});
            config["release_build_estimate_usd"] = json!(1.0);
        }
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
            "citations": [{"path": "src/lib.rs", "line_start": 1, "line_end": 1}],
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
        std::fs::write(
            Path::new(&identity.harness_root).join("src/lib.rs"),
            "pub fn window() { summarise(); }\n",
        )
        .unwrap();
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
            ["harness/src/lib.rs"]
        );
        assert_eq!(
            state.publish.as_ref().unwrap().candidate_sha,
            commit.candidate_sha
        );
        let validation = self.world().e2e.dispatched[1].clone();
        assert_eq!(validation.1, format!("eval:{cycle_id}:candidate"));
        assert!(validation.2["stack"]["yaml"]
            .as_str()
            .unwrap()
            .contains(&commit.candidate_sha));
        let mut services = self.services.lock().unwrap();
        services.candidate_sha = commit.candidate_sha.clone();
        services.candidate_tree = state.candidate_tree.unwrap();
        drop(services);
        (cycle_id, commit.candidate_sha)
    }

    /// … then the source validation returns with fewer tokens and every
    /// criterion held; the promotion runs to the rc and its artifact run.
    async fn reach_artifact(&mut self) -> (String, String) {
        let (cycle_id, candidate_sha) = Box::pin(self.reach_validation()).await;
        let candidate = candidate_system(candidate_sha.clone());
        self.world()
            .e2e
            .finish("plan-2", "passed", side(TOKENS, FIXED), &candidate);
        self.changed("plan-2").await;
        let record = self.cycle(&cycle_id).await;
        assert_eq!(
            record.decision.as_ref().unwrap().outcome,
            DecisionOutcomeV1::Accepted,
            "{:?}",
            record.decision
        );
        assert_eq!(
            record.stage,
            CycleStageV1::AwaitingExecution,
            "{:?}",
            record.history
        );
        let artifact = self.world().e2e.dispatched[2].clone();
        assert_eq!(artifact.1, format!("eval:{cycle_id}:artifact"));
        assert!(artifact.2["stack"]["yaml"].as_str().unwrap().contains(RC));
        let seen: Vec<String> = self
            .services
            .lock()
            .unwrap()
            .seen
            .iter()
            .map(|(key, _)| key.clone())
            .collect();
        assert_eq!(
            seen.iter()
                .filter(|key| key.as_str() == "PATCH /repos/iii-hq/workers/git/refs/heads/main")
                .count(),
            1,
            "one exact non-forced fast-forward"
        );
        assert!(seen.iter().any(|key| key == "GET /operations/op-rc"));
        (cycle_id, candidate_sha)
    }

    /// … then the rc artifact run measures the same gain on the approved
    /// executable; finalization, activation on the target and the canary
    /// dispatch on the target follow.
    async fn reach_canary(&mut self) -> String {
        let (cycle_id, _) = Box::pin(self.reach_artifact()).await;
        let candidate_sha = self.services.lock().unwrap().candidate_sha.clone();
        let _ = candidate_sha;
        let rc = System {
            workers_revision: String::new(),
            harness_version: RC.into(),
            binary: Some(format!("sha256:{}", self.candidate_digest)),
        };
        self.world()
            .e2e
            .finish("plan-3", "passed", side(TOKENS, FIXED), &rc);
        self.changed("plan-3").await;
        let record = self.cycle(&cycle_id).await;
        assert_eq!(
            record.artifact_decision.as_ref().unwrap().outcome,
            DecisionOutcomeV1::Accepted,
            "{:?}",
            record.artifact_decision
        );
        assert_eq!(
            (record.status, record.stage),
            (CycleStatusV1::Waiting, CycleStageV1::Promotion),
            "{:?}",
            record.history
        );
        let adds = self.target().adds();
        assert_eq!(adds.len(), 1);
        assert_eq!(
            adds[0]["workers"],
            json!([{"worker": "package://harness", "version": STABLE}])
        );
        let phases: Vec<String> = serde_json::to_value(record.promotion.as_ref().unwrap()).unwrap()
            ["phases"]
            .as_array()
            .unwrap()
            .iter()
            .map(|phase| phase["phase"].as_str().unwrap().to_string())
            .collect();
        assert!(
            phases.contains(&"activation_requested".to_string())
                && phases.contains(&"activation_confirmed".to_string()),
            "{phases:?}"
        );
        let canary = self.target().e2e.dispatched[0].clone();
        assert_eq!(canary.1, format!("eval:{cycle_id}:canary"));
        assert_eq!(
            canary.2["where"], "harness",
            "the canary runs on the activated target itself"
        );
        assert!(canary.2["stack"].is_null());
        let entry = record
            .executions
            .iter()
            .find(|entry| entry.role == ExecutionRoleV1::Canary)
            .unwrap();
        assert_eq!(entry.route.as_ref().unwrap().namespace, NAMESPACE);
        cycle_id
    }

    fn stable(&self, binary: &str) -> System {
        System {
            workers_revision: String::new(),
            harness_version: STABLE.into(),
            binary: Some(format!("sha256:{binary}")),
        }
    }
}

// ---------------------------------------------------------------------------
// Trajectories
// ---------------------------------------------------------------------------

fn phases(record: &CycleRecordV1) -> Vec<String> {
    serde_json::to_value(record.promotion.as_ref().unwrap()).unwrap()["phases"]
        .as_array()
        .unwrap()
        .iter()
        .map(|phase| phase["phase"].as_str().unwrap().to_string())
        .collect()
}

fn candidate_system(sha: String) -> System {
    System {
        workers_revision: sha,
        harness_version: "h-candidate".into(),
        binary: Some(format!("sha256:{}", sha256(b"candidate harness build"))),
    }
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn an_accepted_candidate_is_released_activated_and_kept_when_its_canary_holds() {
    let Some(mut rig) = Rig::start("promoted").await else {
        return;
    };
    rig.configure(true).await;
    let cycle_id = Box::pin(rig.reach_canary()).await;
    let stable = rig.stable(&rig.candidate_digest.clone());
    rig.target()
        .e2e
        .finish("canary-1", "passed", side(TOKENS, FIXED), &stable);
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(
        record.status,
        CycleStatusV1::Completed,
        "{:?}",
        record.history
    );
    let outcome = record.outcome.clone().unwrap();
    assert_eq!(outcome.code, OutcomeCodeV1::Promoted, "{}", outcome.detail);
    assert_eq!(
        record.canary.as_ref().unwrap().outcome,
        CanaryOutcomeV1::Held
    );
    let journal = phases(&record);
    for phase in [
        "source_approved",
        "pull_request",
        "merge_requested",
        "merged",
        "rc_planned",
        "rc_confirmed",
        "rc_published",
        "artifact_approved",
        "finalize_planned",
        "finalize_confirmed",
        "stable_published",
        "rollback_destination",
        "activation_requested",
        "activation_confirmed",
    ] {
        assert!(journal.contains(&phase.to_string()), "{phase}: {journal:?}");
    }
    let at = |name: &str| journal.iter().position(|phase| phase == name).unwrap();
    assert!(
        at("rc_planned") < at("rc_confirmed"),
        "the plan is persisted before it is confirmed"
    );
    assert!(at("activation_requested") < at("activation_confirmed"));
    // Official builds stay charged at their frozen estimate: unknown cost.
    let usage = runtime::cycle_result(
        &rig.deps,
        CycleIdRequestV1 {
            cycle_id: cycle_id.clone(),
        },
    )
    .await
    .unwrap()
    .unwrap()
    .usage
    .unwrap();
    assert!(usage.unknown_cost >= 2, "{usage:?}");
    // Redelivery and later sweeps change nothing.
    rig.changed("plan-3").await;
    rig.tick().await;
    assert_eq!(rig.target().adds().len(), 1);
    assert_eq!(
        rig.cycle(&cycle_id).await.outcome.unwrap().code,
        OutcomeCodeV1::Promoted
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_violated_canary_rolls_back_to_the_proved_prior_executable() {
    let Some(mut rig) = Rig::start("rollback").await else {
        return;
    };
    rig.configure(true).await;
    let cycle_id = Box::pin(rig.reach_canary()).await;
    let stable = rig.stable(&rig.candidate_digest.clone());
    rig.target().e2e.finish(
        "canary-1",
        "passed",
        side(TOKENS, [Some(3), Some(10), Some(10), Some(10)]),
        &stable,
    );
    rig.tick().await;
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(
        record.canary.as_ref().unwrap().outcome,
        CanaryOutcomeV1::Violated
    );
    let outcome = record.outcome.clone().unwrap();
    assert_eq!(
        outcome.code,
        OutcomeCodeV1::RolledBack,
        "{} {:?}",
        outcome.detail,
        record.history
    );
    assert!(
        outcome.detail.contains(&rig.prior_digest),
        "{}",
        outcome.detail
    );
    let adds = rig.target().adds();
    assert_eq!(adds.len(), 2);
    assert_eq!(
        adds[1]["workers"],
        json!([{"worker": "package://harness", "version": PRIOR}])
    );
    let journal = phases(&record);
    assert!(
        journal.ends_with(&[
            "rollback_requested".to_string(),
            "rollback_confirmed".to_string()
        ]),
        "{journal:?}"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_canary_of_another_executable_is_unknown_and_never_promoted() {
    let Some(mut rig) = Rig::start("unknown").await else {
        return;
    };
    rig.configure(true).await;
    let cycle_id = Box::pin(rig.reach_canary()).await;
    // The target E2E reports the prior executable, not the activated one.
    let other = rig.stable(&rig.prior_digest.clone());
    rig.target()
        .e2e
        .finish("canary-1", "passed", side(TOKENS, FIXED), &other);
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(
        record.canary.as_ref().unwrap().outcome,
        CanaryOutcomeV1::Unknown
    );
    assert_pending_then_restored_at_a_limit(&mut rig, &cycle_id).await;
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_cheaper_candidate_that_loses_a_criterion_is_rejected_and_nothing_is_released() {
    let Some(mut rig) = Rig::start("rejected").await else {
        return;
    };
    rig.configure(true).await;
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
    assert!(record.promotion.is_none());
    assert!(
        rig.services.lock().unwrap().seen.is_empty(),
        "no GitHub, Release Control or Registry call"
    );
    assert!(rig.target().adds().is_empty());
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_validation_that_did_not_run_the_candidate_commit_is_inconclusive() {
    let Some(mut rig) = Rig::start("wrongsha").await else {
        return;
    };
    rig.configure(true).await;
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
async fn missing_release_credentials_stop_unavailable_before_anything_is_activated() {
    let Some(mut rig) = Rig::start("nocreds").await else {
        return;
    };
    rig.configure(true).await;
    let (cycle_id, candidate_sha) = Box::pin(rig.reach_validation()).await;
    rig.credentials(false);
    rig.world().e2e.finish(
        "plan-2",
        "passed",
        side(TOKENS, FIXED),
        &candidate_system(candidate_sha),
    );
    rig.changed("plan-2").await;
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(
        record.decision.as_ref().unwrap().outcome,
        DecisionOutcomeV1::Accepted
    );
    let outcome = record.outcome.clone().unwrap();
    assert_eq!(
        outcome.code,
        OutcomeCodeV1::Inconclusive,
        "{}",
        outcome.detail
    );
    assert!(
        outcome.detail.contains("nothing was activated"),
        "{}",
        outcome.detail
    );
    assert_eq!(phases(&record).last().unwrap(), "stopped");
    assert!(
        !serde_json::to_string(&record).unwrap().contains("secret-"),
        "no credential in the record"
    );
    assert!(rig.target().adds().is_empty());
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn without_promotion_an_accepted_source_candidate_is_decided_and_never_released() {
    let Some(mut rig) = Rig::start("nopromo").await else {
        return;
    };
    rig.configure(false).await;
    let (cycle_id, candidate_sha) = Box::pin(rig.reach_validation()).await;
    rig.world().e2e.finish(
        "plan-2",
        "passed",
        side(TOKENS, FIXED),
        &candidate_system(candidate_sha),
    );
    rig.changed("plan-2").await;
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(
        record.decision.as_ref().unwrap().outcome,
        DecisionOutcomeV1::Accepted
    );
    let outcome = record.outcome.unwrap();
    assert_eq!(outcome.code, OutcomeCodeV1::Decided);
    assert!(
        outcome.detail.contains("nothing released"),
        "{}",
        outcome.detail
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_while_the_actor_runs_stops_it_and_later_steps_do_nothing() {
    let Some(rig) = Rig::start("cancel").await else {
        return;
    };
    rig.configure(true).await;
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
    rig.configure(true).await;
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
    rig.configure(true).await;
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
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn an_uncovered_execution_spends_nothing() {
    let Some(rig) = Rig::start("uncovered").await else {
        return;
    };
    rig.configure(true).await;
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
async fn a_canary_whose_planned_metric_regresses_is_rolled_back() {
    let Some(mut rig) = Rig::start("primary").await else {
        return;
    };
    rig.configure(true).await;
    let cycle_id = Box::pin(rig.reach_canary()).await;
    let stable = rig.stable(&rig.candidate_digest.clone());
    // The planned criterion falls below the baseline: it regressed.
    rig.target().e2e.finish(
        "canary-1",
        "passed",
        side(TOKENS, [Some(10), Some(0), Some(10), Some(10)]),
        &stable,
    );
    rig.tick().await;
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    let canary = record.canary.clone().unwrap();
    assert_eq!(
        canary.outcome,
        CanaryOutcomeV1::Violated,
        "{}",
        canary.reason
    );
    assert!(
        canary.reason.contains("planned metric regressed"),
        "{}",
        canary.reason
    );
    assert_eq!(record.outcome.unwrap().code, OutcomeCodeV1::RolledBack);
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_canary_with_failures_of_unknown_expectedness_is_unknown_not_held() {
    let Some(mut rig) = Rig::start("failures").await else {
        return;
    };
    rig.configure(true).await;
    let cycle_id = Box::pin(rig.reach_canary()).await;
    let stable = rig.stable(&rig.candidate_digest.clone());
    let mut runs = side(TOKENS, FIXED);
    runs[0].1[0]["failures"] = json!([{"status": "subject_error", "domain": "subject",
        "phase": "act", "code": "tool_failed", "message": "a tool call failed and was retried"}]);
    rig.target().e2e.finish("canary-1", "passed", runs, &stable);
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    let canary = record.canary.clone().unwrap();
    assert_eq!(
        canary.outcome,
        CanaryOutcomeV1::Unknown,
        "{}",
        canary.reason
    );
    assert!(canary.reason.contains("expectedness"), "{}", canary.reason);
    assert_pending_then_restored_at_a_limit(&mut rig, &cycle_id).await;
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_after_activation_restores_and_proves_the_prior_executable() {
    let Some(mut rig) = Rig::start("cancelactive").await else {
        return;
    };
    rig.configure(true).await;
    let cycle_id = Box::pin(rig.reach_canary()).await;
    let cancelled = runtime::cycle_cancel(
        &rig.deps,
        CycleIdRequestV1 {
            cycle_id: cycle_id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(cancelled.cancelled);
    assert_eq!(
        cancelled.status,
        CycleStatusV1::Running,
        "not terminal before the prior is proved"
    );
    // The target canary is stopped through the target Engine itself.
    assert!(rig
        .target()
        .calls
        .iter()
        .any(|(function, _)| function == "e2e::dashboard::execution-cancel"));
    rig.drain().await;
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    let outcome = record.outcome.clone().unwrap();
    assert_eq!(
        outcome.code,
        OutcomeCodeV1::RolledBack,
        "{} {:?}",
        outcome.detail,
        record.history
    );
    assert!(
        outcome.detail.contains("cancelled") && outcome.detail.contains(&rig.prior_digest),
        "{}",
        outcome.detail
    );
    let adds = rig.target().adds();
    assert_eq!(adds.len(), 2);
    assert_eq!(
        adds[1]["workers"],
        json!([{"worker": "package://harness", "version": PRIOR}])
    );
    assert!(phases(&record).ends_with(&[
        "rollback_requested".to_string(),
        "rollback_confirmed".to_string()
    ]));
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_budget_limit_while_the_canary_runs_restores_the_prior_instead_of_abandoning_it() {
    let Some(mut rig) = Rig::start("expiry").await else {
        return;
    };
    rig.configure(true).await;
    let cycle_id = Box::pin(rig.reach_canary()).await;
    state::update_ledger(&rig.iii, &cycle_id, |ledger| {
        let mut ledger = ledger.unwrap();
        ledger.budget.max_spend_usd = 0.01;
        Ok((Some(ledger), ()))
    })
    .await
    .unwrap();
    rig.tick().await;
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    let outcome = record.outcome.clone().unwrap();
    assert_eq!(
        outcome.code,
        OutcomeCodeV1::RolledBack,
        "{} {:?}",
        outcome.detail,
        record.history
    );
    assert!(outcome.detail.contains("budget"), "{}", outcome.detail);
    assert_eq!(rig.target().adds().len(), 2);
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_budget_that_cannot_cover_the_canary_never_activates() {
    let Some(mut rig) = Rig::start("nocanary").await else {
        return;
    };
    rig.configure(true).await;
    let (cycle_id, _) = Box::pin(rig.reach_artifact()).await;
    // Leave one finalize build and less than one canary reservation.
    let usage = runtime::cycle_result(
        &rig.deps,
        CycleIdRequestV1 {
            cycle_id: cycle_id.clone(),
        },
    )
    .await
    .unwrap()
    .unwrap()
    .usage
    .unwrap();
    state::update_ledger(&rig.iii, &cycle_id, |ledger| {
        let mut ledger = ledger.unwrap();
        ledger.budget.max_spend_usd = usage.spend_usd + 1.5;
        Ok((Some(ledger), ()))
    })
    .await
    .unwrap();
    let candidate_sha = rig.services.lock().unwrap().candidate_sha.clone();
    let _ = candidate_sha;
    let rc = System {
        workers_revision: String::new(),
        harness_version: RC.into(),
        binary: Some(format!("sha256:{}", rig.candidate_digest)),
    };
    rig.world()
        .e2e
        .finish("plan-3", "passed", side(TOKENS, FIXED), &rc);
    rig.changed("plan-3").await;
    let record = rig.cycle(&cycle_id).await;
    let outcome = record.outcome.clone().unwrap();
    assert_eq!(
        outcome.code,
        OutcomeCodeV1::Inconclusive,
        "{} {:?}",
        outcome.detail,
        record.history
    );
    assert!(
        outcome.detail.contains("canary") && outcome.detail.contains("nothing was activated"),
        "{}",
        outcome.detail
    );
    assert!(rig.target().adds().is_empty());
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
    rig.configure(true).await;
    let cycle_id = rig.begin_source().await;
    // The deployment now trusts another repository.
    let other = rig.scratch.join("other");
    std::fs::create_dir_all(&other).unwrap();
    rig.deps = Deps::new(rig.iii.clone())
        .with_release_origins(origins(&rig.http_url))
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

/// Lets a held activation land: the candidate process now runs.
fn land_activation(rig: &Rig) {
    let mut target = rig.target();
    let pid = target.pids[STABLE];
    target.harness = harness_row(pid, STABLE);
    let id = target.adds()[0]["operation_id"]
        .as_str()
        .unwrap()
        .to_string();
    target.operations.insert(
        id.clone(),
        json!({"operation_id": id, "status": "succeeded"}),
    );
}

/// To the artifact decision with the activation pin sent and held running.
async fn to_held_activation(rig: &mut Rig) -> String {
    let (cycle_id, _) = Box::pin(rig.reach_artifact()).await;
    rig.target().hold = true;
    let rc = System {
        workers_revision: String::new(),
        harness_version: RC.into(),
        binary: Some(format!("sha256:{}", rig.candidate_digest)),
    };
    rig.world()
        .e2e
        .finish("plan-3", "passed", side(TOKENS, FIXED), &rc);
    rig.changed("plan-3").await;
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(
        phases(&record).last().unwrap(),
        "activation_requested",
        "{:?}",
        record.history
    );
    assert!(
        record.activation_intent.is_some(),
        "the intent was persisted"
    );
    cycle_id
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn round2_failed_or_cancelled_activation_restores_its_partial_change() {
    for (status, stopped) in [("failed", false), ("cancelled", false), ("failed", true)] {
        let Some(mut rig) = Rig::start(status).await else {
            return;
        };
        rig.configure(true).await;
        let cycle_id = Box::pin(to_held_activation(&mut rig)).await;
        land_activation(&rig);
        {
            let mut target = rig.target();
            let operation = target.adds()[0]["operation_id"]
                .as_str()
                .unwrap()
                .to_string();
            target.operations.get_mut(&operation).unwrap()["status"] = json!(status);
            if stopped {
                target.harness["state"] = json!("failed");
                target.harness["pid"] = Value::Null;
            }
        }
        rig.tick().await;
        rig.tick().await;
        let record = rig.cycle(&cycle_id).await;
        assert_eq!(
            record.outcome.as_ref().map(|outcome| outcome.code),
            Some(OutcomeCodeV1::RolledBack),
            "{status}: {:?}",
            record.history
        );
        assert_eq!(rig.target().adds().len(), 2);
        assert!(
            phases(&record).ends_with(&["rollback_requested".into(), "rollback_confirmed".into(),])
        );
        assert!(record.outcome.unwrap().detail.contains(&rig.prior_digest));
    }
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn round2_an_activation_error_after_the_persisted_intent_still_restores() {
    let Some(mut rig) = Rig::start("intenterror").await else {
        return;
    };
    rig.configure(true).await;
    let (cycle_id, _) = Box::pin(rig.reach_artifact()).await;
    rig.target().fail_after_pin = true;
    let rc = System {
        workers_revision: String::new(),
        harness_version: RC.into(),
        binary: Some(format!("sha256:{}", rig.candidate_digest)),
    };
    rig.world()
        .e2e
        .finish("plan-3", "passed", side(TOKENS, FIXED), &rc);
    rig.changed("plan-3").await;
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    assert_eq!(
        record.outcome.as_ref().unwrap().code,
        OutcomeCodeV1::RolledBack
    );
    assert!(record.activation_intent.is_some());
    assert!(!phases(&record).contains(&"activation_requested".into()));
    assert!(phases(&record).contains(&"rollback_confirmed".into()));
    assert_eq!(rig.target().adds().len(), 2);
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_lost_canary_reply_uses_only_its_immutable_key() {
    let Some(mut rig) = Rig::start("canarylabel").await else {
        return;
    };
    rig.configure(true).await;
    let cycle_id = Box::pin(rig.reach_canary()).await;
    state::update_cycle(&rig.iii, &cycle_id, |stored| {
        let mut next = stored.clone();
        let entry = next
            .executions
            .iter_mut()
            .find(|entry| entry.role == ExecutionRoleV1::Canary)
            .unwrap();
        entry.execution_id = None;
        entry.state = ExecutionStateV1::Dispatching;
        Ok((Some(next), ()))
    })
    .await
    .unwrap();
    let stable = rig.stable(&rig.candidate_digest.clone());
    {
        let mut target = rig.target();
        let (_, label, parameters) = target.e2e.dispatched[0].clone();
        target.e2e.executions.remove("canary-1");
        let mut hijack = bundle(
            "aaa-hijack",
            &label,
            "running",
            &parameters,
            Vec::new(),
            &stable,
        );
        hijack["manifest"]["executions"][0]["idempotency_key"] = json!("foreign:canary");
        target.e2e.executions.insert("aaa-hijack".into(), hijack);
    }
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    let entry = record
        .executions
        .iter()
        .find(|entry| entry.role == ExecutionRoleV1::Canary)
        .unwrap();
    assert!(
        entry.execution_id.is_none(),
        "foreign canary was bound to {:?}",
        entry.execution_id
    );
    assert!(record.canary.is_none());
    assert!(!record.status.is_terminal());
    assert_eq!(rig.target().e2e.dispatched.len(), 1);

    {
        let mut target = rig.target();
        let (_, _, parameters) = target.e2e.dispatched[0].clone();
        target.e2e.executions.remove("aaa-hijack");
        let mut recovered = bundle(
            "recovered",
            "renamed canary",
            "running",
            &parameters,
            Vec::new(),
            &stable,
        );
        recovered["manifest"]["executions"][0]["idempotency_key"] =
            json!(format!("{cycle_id}:canary"));
        target.e2e.executions.insert("recovered".into(), recovered);
    }
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    let entry = record
        .executions
        .iter()
        .find(|entry| entry.role == ExecutionRoleV1::Canary)
        .unwrap();
    assert_eq!(entry.execution_id.as_deref(), Some("recovered"));
    assert_eq!(rig.target().e2e.dispatched.len(), 1);
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn round2_a_prior_package_must_match_its_running_executable_before_activation() {
    let Some(mut rig) = Rig::start("priorbytes").await else {
        return;
    };
    rig.configure(true).await;
    let (cycle_id, _) = Box::pin(rig.reach_artifact()).await;
    {
        let mut target = rig.target();
        target.harness = harness_row(target.pids[STABLE], PRIOR);
    }
    let rc = rig.stable(&rig.candidate_digest.clone());
    let rc = System {
        harness_version: RC.into(),
        ..rc
    };
    rig.world()
        .e2e
        .finish("plan-3", "passed", side(TOKENS, FIXED), &rc);
    rig.changed("plan-3").await;
    let record = rig.cycle(&cycle_id).await;
    assert!(
        rig.target().adds().is_empty(),
        "activation must not start with an unproved rollback"
    );
    assert!(record.activation_intent.is_none());
    assert_eq!(
        record.outcome.as_ref().unwrap().code,
        OutcomeCodeV1::Inconclusive
    );
    assert!(!phases(&record).contains(&"rollback_destination".into()));
}

async fn assert_restored_only_after_landing(rig: &Rig, cycle_id: &str) {
    rig.drain().await;
    rig.tick().await;
    let record = rig.cycle(cycle_id).await;
    assert!(!record.status.is_terminal(), "{:?}", record.history);
    assert!(record.outcome.is_none());
    assert_eq!(
        rig.target().adds().len(),
        1,
        "no second pin while the activation may still land"
    );
    land_activation(rig);
    rig.tick().await;
    rig.tick().await;
    let record = rig.cycle(cycle_id).await;
    let outcome = record.outcome.clone().unwrap();
    assert_eq!(
        outcome.code,
        OutcomeCodeV1::RolledBack,
        "{} {:?}",
        outcome.detail,
        record.history
    );
    assert!(
        outcome.detail.contains(&rig.prior_digest),
        "{}",
        outcome.detail
    );
    let journal = phases(&record);
    assert!(
        journal.ends_with(&[
            "activation_confirmed".to_string(),
            "rollback_requested".to_string(),
            "rollback_confirmed".to_string(),
        ]),
        "{journal:?}"
    );
    let adds = rig.target().adds();
    assert_eq!(adds.len(), 2);
    assert_eq!(
        adds[1]["workers"],
        json!([{"worker": "package://harness", "version": PRIOR}])
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn cancelling_while_the_activation_still_runs_waits_for_it_before_restoring() {
    let Some(mut rig) = Rig::start("pending").await else {
        return;
    };
    rig.configure(true).await;
    let cycle_id = to_held_activation(&mut rig).await;
    let cancelled = runtime::cycle_cancel(
        &rig.deps,
        CycleIdRequestV1 {
            cycle_id: cycle_id.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        cancelled.status,
        CycleStatusV1::Running,
        "not terminal before restoration"
    );
    assert_restored_only_after_landing(&rig, &cycle_id).await;
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_lost_activation_record_still_counts_as_in_flight_for_cancel() {
    let Some(mut rig) = Rig::start("lostcas").await else {
        return;
    };
    rig.configure(true).await;
    let cycle_id = to_held_activation(&mut rig).await;
    // The pin reached Compose but its journal write was lost (a crash
    // between the effect and the record): only the persisted intent remains.
    state::update_cycle(&rig.iii, &cycle_id, |stored| {
        let mut next = stored.clone();
        next.promotion.as_mut().unwrap().phases.pop();
        Ok((Some(next), ()))
    })
    .await
    .unwrap();
    assert_eq!(
        phases(&rig.cycle(&cycle_id).await).last().unwrap(),
        "rollback_destination"
    );
    let cancelled = runtime::cycle_cancel(
        &rig.deps,
        CycleIdRequestV1 {
            cycle_id: cycle_id.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(
        cancelled.status,
        CycleStatusV1::Running,
        "never Cancelled while a pin may apply"
    );
    assert_restored_only_after_landing(&rig, &cycle_id).await;
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn an_intent_whose_pin_was_never_sent_never_activates_on_cancel() {
    let Some(mut rig) = Rig::start("nosend").await else {
        return;
    };
    rig.configure(true).await;
    let cycle_id = to_held_activation(&mut rig).await;
    // A crash after the intent was persisted and before the pin was sent:
    // Compose holds no operation and the prior still runs.
    state::update_cycle(&rig.iii, &cycle_id, |stored| {
        let mut next = stored.clone();
        next.promotion.as_mut().unwrap().phases.pop();
        Ok((Some(next), ()))
    })
    .await
    .unwrap();
    {
        let mut target = rig.target();
        target.operations.clear();
        target.calls.clear();
    }
    let cancelled = runtime::cycle_cancel(
        &rig.deps,
        CycleIdRequestV1 {
            cycle_id: cycle_id.clone(),
        },
    )
    .await
    .unwrap();
    assert_eq!(cancelled.status, CycleStatusV1::Running);
    rig.drain().await;
    rig.tick().await;
    rig.tick().await;
    let record = rig.cycle(&cycle_id).await;
    assert!(
        rig.target().adds().is_empty(),
        "recovery never sends an activation (or anything)"
    );
    assert!(
        record
            .outcome
            .as_ref()
            .is_none_or(|outcome| outcome.code != OutcomeCodeV1::RolledBack),
        "never claimed restored while the pin's fate is unknown: {:?}",
        record.history
    );
    assert!(
        !record.status.is_terminal(),
        "pending, explicitly, within the grace: {:?}",
        record.history
    );
}

/// An unknown canary keeps the activated target under pending observation
/// (no claim, no new canary); a time limit then restores the prior.
async fn assert_pending_then_restored_at_a_limit(rig: &mut Rig, cycle_id: &str) {
    rig.tick().await;
    let record = rig.cycle(cycle_id).await;
    assert_eq!(
        (record.status, record.stage),
        (CycleStatusV1::Waiting, CycleStageV1::Promotion),
        "{:?}",
        record.history
    );
    assert!(
        record.outcome.is_none(),
        "never claimed stable or restored yet"
    );
    assert_eq!(rig.target().adds().len(), 1);
    assert_eq!(
        rig.target().e2e.dispatched.len(),
        1,
        "no new canary to try again"
    );
    state::update_ledger(&rig.iii, cycle_id, |ledger| {
        let mut ledger = ledger.unwrap();
        ledger.budget.max_duration_ms = 1;
        Ok((Some(ledger), ()))
    })
    .await
    .unwrap();
    rig.tick().await;
    rig.tick().await;
    let record = rig.cycle(cycle_id).await;
    let outcome = record.outcome.clone().unwrap();
    assert_eq!(
        outcome.code,
        OutcomeCodeV1::RolledBack,
        "{} {:?}",
        outcome.detail,
        record.history
    );
    assert_eq!(
        rig.target().adds()[1]["workers"],
        json!([{"worker": "package://harness", "version": PRIOR}])
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_token_objective_never_qualifies_a_source_change() {
    let Some(rig) = Rig::start("tokens").await else {
        return;
    };
    rig.configure(true).await;
    let cycle_id = rig.begin(json!({"objective": OBJECTIVE})).await;
    let record = rig.cycle(&cycle_id).await;
    let outcome = record.outcome.clone().unwrap();
    assert_eq!(
        outcome.code,
        OutcomeCodeV1::NoCoverage,
        "{}",
        outcome.detail
    );
    assert!(
        outcome.detail.contains("harness::metrics"),
        "{}",
        outcome.detail
    );
    assert!(record.plan.is_none(), "nothing frozen to spend on");
    assert!(rig.world().e2e.dispatched.is_empty(), "no baseline");
    assert!(
        rig.world().calls_to("harness::send").is_empty(),
        "no paid actor"
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_tampered_token_counter_without_the_criterion_fix_is_not_approved() {
    let Some(mut rig) = Rig::start("tampered").await else {
        return;
    };
    rig.configure(true).await;
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
    assert!(record.promotion.is_none());
    assert!(rig.services.lock().unwrap().seen.is_empty());
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn an_rc_whose_compiled_version_label_lags_is_judged_by_its_bytes() {
    let Some(mut rig) = Rig::start("label").await else {
        return;
    };
    rig.configure(true).await;
    let (cycle_id, _) = Box::pin(rig.reach_artifact()).await;
    // The official build does not rewrite the compiled version for the rc:
    // the run reports the previous label, with the rc archive's bytes.
    let rc = System {
        workers_revision: String::new(),
        harness_version: PRIOR.into(),
        binary: Some(format!("sha256:{}", rig.candidate_digest)),
    };
    rig.world()
        .e2e
        .finish("plan-3", "passed", side(TOKENS, FIXED), &rc);
    rig.changed("plan-3").await;
    let record = rig.cycle(&cycle_id).await;
    let decision = record.artifact_decision.clone().unwrap();
    assert_eq!(
        decision.outcome,
        DecisionOutcomeV1::Accepted,
        "{}",
        decision.reason
    );
    assert!(
        phases(&record).contains(&"artifact_approved".to_string()),
        "{:?}",
        record.history
    );
}

#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn an_rc_run_with_other_bytes_is_refused_before_finalization() {
    let Some(mut rig) = Rig::start("bytes").await else {
        return;
    };
    rig.configure(true).await;
    let (cycle_id, _) = Box::pin(rig.reach_artifact()).await;
    // Same rc label, other executable: never approved, nothing finalized.
    let rc = System {
        workers_revision: String::new(),
        harness_version: RC.into(),
        binary: Some(format!("sha256:{}", rig.prior_digest)),
    };
    rig.world()
        .e2e
        .finish("plan-3", "passed", side(TOKENS, FIXED), &rc);
    rig.changed("plan-3").await;
    let record = rig.cycle(&cycle_id).await;
    let journal = phases(&record);
    assert!(
        !journal.contains(&"artifact_approved".to_string()),
        "{journal:?}"
    );
    assert!(
        !journal.contains(&"finalize_planned".to_string()),
        "{journal:?}"
    );
    assert!(rig.target().adds().is_empty());
    let outcome = record.outcome.unwrap();
    assert_eq!(
        outcome.code,
        OutcomeCodeV1::Inconclusive,
        "{}",
        outcome.detail
    );
}

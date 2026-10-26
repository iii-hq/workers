//! The loop cycle over the real SDK wire protocol against an in-memory
//! engine: objective → coverage → frozen plan → reserved baseline →
//! imported result → first pass in the same cycle, and the execution input.
//! Every dependency (state, queue, E2E, Sentinel) is a local mock shaped
//! like the native contracts; no credentials or paid calls are used.

#[path = "../../judge-typesafe/tests/support/fake_engine.rs"]
mod fake_engine;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eval::contract::*;
use eval::runtime::{self, Deps};
use eval::state;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};

const OBJECTIVE: &str = "Reduzir o consumo de contexto mantendo a correção das tarefas";
const BEHAVIOR: &str = "sha256:a3d4";
const CASE: &str = "context_pressure:seed-0000000000000bbb";
const CRITERIA: [&str; 4] = [
    "charter_preserved",
    "needles_recovered",
    "ingestion_discipline",
    "concise_report",
];

struct World {
    state: BTreeMap<(String, String), Value>,
    steps: VecDeque<Value>,
    calls: Vec<(String, Value)>,
    executions: BTreeMap<String, Value>,
    tests_pages: Vec<Value>,
    catalog: Value,
    starts: usize,
    unanswered_start: bool,
    /// Starts refused because the runner holds another execution.
    busy_starts: usize,
    executions_next_cursor: Option<String>,
    /// `test-history-get` answers by test id; an absent test has no runs.
    histories: BTreeMap<String, Value>,
    /// What Jev answers of every failed call, or `None`: it does not answer.
    jev_class: Option<&'static str>,
    /// What the default LLM replies, or `None`: it does not answer.
    analyst_reply: Option<String>,
}

fn tests_row(
    test_id: &str,
    current: &str,
    summary: &str,
    prompt: &str,
    criteria: &[(&str, &str)],
) -> Value {
    json!({"test_id": test_id, "lifecycle": "active", "current_version": current,
        "selected_version": "sha256:older", "runs_current": 8, "runs_total": 28,
        "available_versions": [], "recent_scores": [],
        "spec": {"title": test_id, "summary": summary, "prompt": prompt,
            "denied_functions": [], "execution": {"stuck_timeout_seconds": 600},
            "criteria": criteria.iter().map(|(id, description)| json!({"id": id,
                "description": description, "gate": false, "kind": "signal",
                "policy": "advisory", "dimension": "deliverable", "weight": 10})).collect::<Vec<_>>()}})
}

impl World {
    fn new() -> Self {
        let context = tests_row(
            "context_pressure",
            BEHAVIOR,
            "Recovers early charter facts and ordered markers after a large distractor corpus fills the context.",
            "Complete a context-endurance run over 48 large documents.",
            &[
                ("charter_preserved", "Does the final report reproduce every planted charter fact verbatim after the full distractor load?"),
                ("needles_recovered", "Does every segment needle appear in the report exactly once?"),
                ("ingestion_discipline", "Was the charter read first and each segment exactly once?"),
                ("concise_report", "Does the report include the required facts without echoing document text?"),
            ],
        );
        let recovery = tests_row(
            "tool_contract_recovery",
            "sha256:5cfc",
            "Resolve a stale scheduling function, discover its current contract, and create exactly one event.",
            "Schedule the follow-up with the scheduling provider.",
            &[("contract_recovered", "The exact current contract was followed after at most one function-call error.")],
        );
        let fallback_noise = tests_row(
            "subagent_validation_failure",
            "sha256:sub",
            "Runs a child validation loop and wakes the parent through an expiry fallback.",
            "Validate the child.",
            &[("woken", "The parent was woken.")],
        );
        let scenario = |id: &str, behavior: &str, case: &str, seed: u64| {
            json!({
            "scenario_id": id, "behavior_sha256": behavior, "case_id": case, "seed": seed,
            "contract_sha256": format!("contract-{id}"), "inputs_sha256": format!("inputs-{id}"),
            "summary": "", "deliverable_contract": {"invariants": [
                {"id": "charter_preserved", "description": "The report reproduces every planted charter fact."}]}})
        };
        Self {
            state: BTreeMap::new(),
            steps: VecDeque::new(),
            calls: Vec::new(),
            executions: BTreeMap::new(),
            tests_pages: vec![
                json!({"rows": [recovery, fallback_noise], "total": 3, "revision": "rev-1", "next_cursor": "2"}),
                json!({"rows": [context], "total": 3, "revision": "rev-1", "next_cursor": null}),
            ],
            catalog: json!({"catalog_sha256": "sha256:cat", "schema": "e2e-scenario-catalog", "scenarios": [
                scenario("context_pressure", BEHAVIOR, CASE, 3003),
                scenario("tool_contract_recovery", "sha256:5cfc", "tool_contract_recovery:seed-1", 1),
                scenario("subagent_validation_failure", "sha256:sub", "subagent:seed-1", 2)]}),
            starts: 0,
            unanswered_start: false,
            busy_starts: 0,
            executions_next_cursor: None,
            histories: BTreeMap::new(),
            jev_class: None,
            analyst_reply: None,
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
                json!({"swapped": matches, "current": self.state.get(&key()).cloned().unwrap_or(Value::Null)})
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
            "e2e::dashboard::tests-list" => {
                let page = data["cursor"]
                    .as_str()
                    .map_or(0, |cursor| cursor.parse::<usize>().unwrap() - 1);
                self.tests_pages[page].clone()
            }
            "e2e::scenarios-list" => self.catalog.clone(),
            "e2e::dashboard::test-history-get" => {
                let test_id = data["test_id"].as_str().unwrap_or_default();
                match self.histories.get(test_id) {
                    Some(Value::Null) => return Some(Err(format!("unknown test '{test_id}'"))),
                    Some(history) => history.clone(),
                    None => json!({"observations": []}),
                }
            }
            "e2e::dashboard::execution-start" if self.busy_starts > 0 => {
                self.busy_starts -= 1;
                return Some(Err(
                    "\"nightly\" (plan-0) is still running; wait for it to finish or cancel it."
                        .into(),
                ));
            }
            "e2e::dashboard::execution-start" => {
                self.starts += 1;
                let id = format!("plan-{}", self.starts);
                let label = data["label"].as_str().unwrap_or_default().to_string();
                let mut execution = bundle(&id, &label, "running", Vec::new());
                execution["manifest"]["executions"][0]["idempotency_key"] =
                    data["idempotency_key"].clone();
                self.executions.insert(id.clone(), execution);
                // An answer that names no execution is as ambiguous as none.
                if self.unanswered_start {
                    return Some(Ok(json!({})));
                }
                json!({"execution_id": id})
            }
            "e2e::dashboard::execution-get" => {
                match self.executions.get(data["execution_id"].as_str().unwrap()) {
                    Some(bundle) => bundle.clone(),
                    None => return Some(Err("execution not found".into())),
                }
            }
            "e2e::dashboard::executions-list" => {
                let query = data["query"].as_str().unwrap_or_default();
                let mut executions: Vec<Value> = self
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
                executions.sort_by(|a, b| b["started_at"].as_str().cmp(&a["started_at"].as_str()));
                json!({"executions": executions, "total": executions.len(),
                    "next_cursor": self.executions_next_cursor})
            }
            "e2e::dashboard::execution-cancel" => json!({}),
            "judge::evaluate" => {
                let Some(class) = self.jev_class else {
                    return Some(Err("judge unavailable".into()));
                };
                let results: serde_json::Map<String, Value> = data["evaluations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|evaluation| {
                        let probabilities: serde_json::Map<String, Value> =
                            ["harness", "model", "task", "unclear"]
                                .iter()
                                .map(|choice| {
                                    (
                                        choice.to_string(),
                                        json!(if *choice == class { 0.9 } else { 0.1 / 3.0 }),
                                    )
                                })
                                .collect();
                        (
                            evaluation["id"].as_str().unwrap().to_string(),
                            json!({"answers": {"whose": {"type": "choice", "choice": class,
                                "confidence": 0.9, "probabilities": probabilities}}}),
                        )
                    })
                    .collect();
                json!({"status": "ok", "model": "jev-test", "results": results,
                    "stats": {"attempts": 1, "elapsed_ms": 1, "input_tokens": 10, "output_tokens": 1,
                        "questions": results.len(), "requests": 1, "usage_complete": true}})
            }
            "router::complete" => {
                let Some(reply) = &self.analyst_reply else {
                    return Some(Err("router unavailable".into()));
                };
                json!({"message": {"role": "assistant", "content": [{"type": "text", "text": reply}]},
                    "usage": {"cost_usd": 0.02, "input": 100, "output": 50}})
            }
            "sentinel::reports::ingest" => {
                let observations = data["observations"].as_array().cloned().unwrap_or_default();
                json!({"recorded": observations.len(), "deduped": 0, "skipped": 0,
                    "results": observations.iter().enumerate().map(|(at, _)| json!({
                        "dedupe_key": format!("report:{at}"), "classification": "insufficient_evidence",
                        "outcome": "created", "group_id": "g-1", "occurrence_id": format!("o-{at}"),
                        "triage": null})).collect::<Vec<_>>()})
            }
            other => return Some(Err(format!("unexpected call to {other}"))),
        }))
    }
}

/// A native run: `awards` per spec criterion (`None`: not evaluated).
fn run(run_id: &str, status: &str, completion: &str, tokens: f64, awards: &[Option<u8>]) -> Value {
    json!({"run_id": run_id, "attempt_id": format!("{run_id}-a"), "attempt_number": 1,
        "session_id": format!("s-{run_id}"), "status": status, "completion": completion,
        "technical": "valid", "wall_time_ms": 1000, "cost": {"total_usd": 0.2},
        "efficiency": {"total_tokens": tokens, "function_calls": 50},
        "criteria": CRITERIA.iter().zip(awards).map(|(id, awarded)| json!({
            "id": id, "possible": 10, "awarded": awarded, "gate": false, "reason": "r"})).collect::<Vec<_>>(),
        "transcript": {"messages": []}})
}

fn bundle(id: &str, label: &str, status: &str, runs: Vec<Value>) -> Value {
    let reports = if runs.is_empty() {
        vec![
            json!({"scenario_id": "context_pressure", "available": false, "report": null,
            "state": if status == "running" { "running" } else { "queued" }}),
        ]
    } else {
        vec![
            json!({"scenario_id": "context_pressure", "available": true, "report": {
            "result_contract_sha256": "rc-1",
            "system_under_test": {"stack": {"workers_repository": "iii-hq/workers",
                "workers_revision": "w-1"}, "engine_version": "0.22.0", "harness_version": "h-1",
                "e2e_repository": "iii-hq/harness-e2e", "e2e_revision": "e-1",
                "contract_hashes": {"harness::status": "x"}},
            "subject": {"model": "m", "provider": "p"},
            "scenarios": [{"scenario_id": "context_pressure", "case_id": CASE,
                "behavior_sha256": BEHAVIOR, "case": {"inputs_sha256": "inputs-context_pressure"},
                "runs": runs}]}}),
        ]
    };
    json!({"manifest": {"executions": [{"id": id, "label": label, "status": status,
            "run_id": id, "attempt": 1, "started_at": "2099-01-01T00:00:00Z",
            "completed_at": if status == "running" { "" } else { "2099-01-01T00:10:00Z" },
            "parameters": {"model": "m", "provider": "p", "runs": 1, "scenarios": ["context_pressure"],
                "technical_retries": 0}}]},
        "detail": {"reports": reports}})
}

fn full() -> Vec<Value> {
    vec![run("r1", "passed", "completed", 90_000.0, &[Some(10); 4])]
}

struct Harness {
    world: Arc<Mutex<World>>,
    deps: Deps,
    iii: Arc<IIIClient>,
    _engine: fake_engine::FakeEngine,
}

impl Harness {
    async fn start(world: World) -> Self {
        let world = Arc::new(Mutex::new(world));
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
            let Some(reply) = world.respond(&function, &frame["data"]) else {
                return vec![];
            };
            let mut response = json!({"type": "invocationresult",
                "invocation_id": frame["invocation_id"], "function_id": function});
            match reply {
                Ok(result) => response["result"] = result,
                Err(message) => response["error"] = json!({"code": "mock", "message": message}),
            }
            vec![response]
        })
        .await;
        let iii = Arc::new(register_worker(
            &engine.url,
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
        .expect("worker connects to the fake engine");
        let deps = Deps::new(iii.clone());
        Self {
            world,
            deps,
            iii,
            _engine: engine,
        }
    }

    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }

    async fn drain(&self) {
        loop {
            let next = self.world().steps.pop_front();
            let Some(payload) = next else { break };
            runtime::step(&self.deps, serde_json::from_value(payload).unwrap())
                .await
                .unwrap();
        }
    }

    async fn configure(&self, max_spend_usd: f64) {
        let config = json!({
            "subject": {"provider": "p", "model": "m"}, "where": "harness",
            "runs_per_side": 1, "technical_retries": 0, "max_scenarios": 1,
            "estimated_usd_per_run": 0.5,
            "min_effect": {"tokens": 1000.0, "pass_rate_pp": 10.0},
            "budget": {"max_spend_usd": max_spend_usd, "max_duration_ms": 3_600_000,
                       "max_calls": 10, "max_candidates": 1}});
        runtime::configure(
            &self.deps,
            serde_json::from_value(json!({"config": config})).unwrap(),
        )
        .await
        .unwrap();
    }

    /// [`Self::configure`] with Jev as the judge.
    async fn configure_judged(&self) {
        let config = json!({
            "subject": {"provider": "p", "model": "m"}, "where": "harness",
            "runs_per_side": 1, "technical_retries": 0, "max_scenarios": 1,
            "estimated_usd_per_run": 0.5,
            "min_effect": {"tokens": 1000.0, "pass_rate_pp": 10.0},
            "budget": {"max_spend_usd": 10.0, "max_duration_ms": 3_600_000,
                       "max_calls": 10, "max_candidates": 1},
            "judge": {"provider": "typesafe", "estimated_usd": 0.5}});
        runtime::configure(
            &self.deps,
            serde_json::from_value(json!({"config": config})).unwrap(),
        )
        .await
        .unwrap();
    }

    async fn start_cycle(&self, request: Value) -> String {
        runtime::cycle_start(&self.deps, serde_json::from_value(request).unwrap())
            .await
            .unwrap()
            .cycle_id
    }

    async fn cycle(&self, cycle_id: &str) -> CycleRecordV1 {
        state::get_cycle(&self.iii, cycle_id)
            .await
            .unwrap()
            .expect("cycle exists")
    }

    async fn changed(&self, execution_id: &str) -> ExecutionAdmissionResponseV1 {
        runtime::execution_changed(
            &self.deps,
            serde_json::from_value(json!({"kind": "completed", "execution_id": execution_id}))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    fn finish(&self, execution_id: &str, status: &str, runs: Vec<Value>) {
        let mut world = self.world();
        let label = world.executions[execution_id]["manifest"]["executions"][0]["label"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let idempotency_key =
            world.executions[execution_id]["manifest"]["executions"][0]["idempotency_key"].clone();
        let mut finished = bundle(execution_id, &label, status, runs);
        finished["manifest"]["executions"][0]["idempotency_key"] = idempotency_key;
        world.executions.insert(execution_id.into(), finished);
    }

    fn count(&self, scope: &str) -> usize {
        self.world()
            .state
            .keys()
            .filter(|(stored, _)| stored == scope)
            .count()
    }
}

#[tokio::test]
async fn objective_freezes_a_plan_reserves_dispatches_and_imports_the_baseline_into_the_same_cycle()
{
    let h = Harness::start(World::new()).await;
    h.configure(10.0).await;
    let cycle_id = h.start_cycle(json!({"objective": OBJECTIVE})).await;
    h.drain().await;

    let record = h.cycle(&cycle_id).await;
    assert_eq!(
        (record.status, record.stage),
        (CycleStatusV1::Waiting, CycleStageV1::AwaitingExecution)
    );
    // Every page of the catalog was read; only the current context test covers it.
    assert_eq!(h.world().calls_to("e2e::dashboard::tests-list").len(), 2);
    let coverage = record.coverage.clone().unwrap();
    assert_eq!(coverage.need, "context_consumption");
    assert_eq!(coverage.selected.len(), 1);
    let selected = &coverage.selected[0];
    assert_eq!(
        (
            selected.scenario_id.as_str(),
            selected.case_id.as_str(),
            selected.seed
        ),
        ("context_pressure", CASE, 3003)
    );
    assert_eq!(selected.behavior_sha256, BEHAVIOR);
    assert!(
        selected.history_note.is_some(),
        "a historical selected_version is not current evidence"
    );
    // The plan preserves every spec criterion and the pass rate, before any result.
    let plan = record.plan.clone().unwrap();
    assert_eq!(plan.criterion.metric, CriterionMetricV1::Tokens);
    assert_eq!(plan.criterion.direction, DirectionV1::Decrease);
    let preserved: Vec<_> = plan
        .preserved
        .iter()
        .filter_map(|criterion| criterion.criterion_id.clone())
        .collect();
    assert_eq!(preserved, CRITERIA);
    assert!(plan
        .preserved
        .iter()
        .any(|criterion| criterion.metric == CriterionMetricV1::PassRate));
    assert_eq!(plan.sampling.seeds, [CASE]);
    // Reserved before dispatch, then one execution-start with a deterministic label.
    let calls = h.world().calls.clone();
    let reserved_at = calls.iter().position(|(id, data)| {
        id == "state::compare-and-set"
            && data["scope"] == state::BUDGET_SCOPE
            && data["expected"].is_object()
    });
    let started_at = calls
        .iter()
        .position(|(id, _)| id == "e2e::dashboard::execution-start")
        .unwrap();
    assert!(
        reserved_at.unwrap() < started_at,
        "reserved before dispatch"
    );
    let start = &calls[started_at].1;
    assert_eq!(start["label"], format!("eval:{cycle_id}:baseline"));
    assert_eq!(start["idempotency_key"], format!("{cycle_id}:baseline"));
    assert_eq!(
        start["parameters"]["scenarios"],
        json!(["context_pressure"])
    );
    assert_eq!(start["parameters"]["where"], "harness");
    assert!(start.get("cycle_id").is_none() && start["parameters"].get("cycle_id").is_none());
    assert_eq!(record.executions[0].execution_id.as_deref(), Some("plan-1"));

    // A progress event while it runs admits nothing.
    assert!(!h.changed("plan-1").await.admitted);
    // Completed: imported into the same cycle and analyzed without a model.
    h.finish("plan-1", "passed", full());
    let admitted = h.changed("plan-1").await;
    assert_eq!(admitted.cycle_id.as_deref(), Some(cycle_id.as_str()));
    h.drain().await;
    let record = h.cycle(&cycle_id).await;
    // Without config.candidate the measured baseline stops, inconclusive with
    // the reason, before any paid proposal.
    assert_eq!(
        (record.status, record.stage),
        (CycleStatusV1::Completed, CycleStageV1::Done)
    );
    let outcome = record.outcome.clone().unwrap();
    assert_eq!(outcome.code, OutcomeCodeV1::Inconclusive);
    assert!(
        outcome.detail.contains("config.candidate"),
        "{}",
        outcome.detail
    );
    let baseline = record.baseline.clone().unwrap();
    assert!(baseline.usable, "{}", baseline.reason);
    assert_eq!(baseline.primary.mean, Some(90_000.0));
    assert_eq!(record.analyses.len(), 1);
    assert_eq!(record.analyses[0].role, ExecutionRoleV1::Baseline);
    assert_eq!(h.count(state::CYCLE_SCOPE), 1, "no second cycle");
    // The reservation settled at the reported cost.
    let usage = runtime::cycle_result(
        &h.deps,
        CycleIdRequestV1 {
            cycle_id: cycle_id.clone(),
        },
    )
    .await
    .unwrap()
    .unwrap()
    .usage
    .unwrap();
    assert_eq!((usage.calls, usage.unknown_cost), (1, 0));
    assert!((usage.spend_usd - 0.5).abs() < 1e-9);
    // Redelivery changes nothing.
    h.changed("plan-1").await;
    h.drain().await;
    assert_eq!(h.cycle(&cycle_id).await.analyses.len(), 1);
    assert_eq!(h.world().starts, 1);
}

#[tokio::test]
async fn a_terminal_execution_cannot_steal_an_owned_label() {
    let h = Harness::start(World::new()).await;
    h.configure(10.0).await;
    let cycle_id = h.start_cycle(json!({"objective": OBJECTIVE})).await;
    h.drain().await;
    let label = format!("eval:{cycle_id}:baseline");
    h.world()
        .executions
        .insert("evil".into(), bundle("evil", &label, "passed", full()));

    let response = h.changed("evil").await;
    assert!(!response.admitted);
    assert!(
        response.reason.contains("idempotency key"),
        "{}",
        response.reason
    );
    h.drain().await;

    let record = h.cycle(&cycle_id).await;
    assert_eq!(record.status, CycleStatusV1::Waiting);
    assert_eq!(record.executions[0].execution_id.as_deref(), Some("plan-1"));
    assert!(record.analyses.is_empty());
    let ledger = state::get_ledger(&h.iii, &cycle_id).await.unwrap().unwrap();
    assert_eq!(ledger.reservations.len(), 1);
    assert!(ledger
        .reservations
        .values()
        .all(|entry| entry.settled_at.is_none()));
}

#[tokio::test]
async fn a_native_child_owned_by_a_plan_is_never_admitted_as_another_cycle() {
    let h = Harness::start(World::new()).await;
    let mut child = bundle("native-child", "", "passed", full());
    child["manifest"]["executions"][0]["parent_plan_execution_id"] = json!("plan-parent");
    h.world().executions.insert("native-child".into(), child);

    let response = h.changed("native-child").await;

    assert!(!response.admitted);
    assert!(
        response.reason.contains("plan-parent"),
        "{}",
        response.reason
    );
    assert!(state::list_cycles(&h.iii).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_baseline_that_never_ran_says_why_the_execution_failed() {
    let h = Harness::start(World::new()).await;
    h.configure(10.0).await;
    let cycle_id = h.start_cycle(json!({"objective": OBJECTIVE})).await;
    h.drain().await;
    h.finish("plan-1", "incomplete", vec![]);
    {
        // As the E2E reports a preparation that failed: no slot ever ran.
        let mut world = h.world();
        let execution = world.executions.get_mut("plan-1").unwrap();
        execution["manifest"]["executions"][0]["first_failure"] = json!({"kind": "execution",
            "message": "Preparing the execution failed at `prepare resolve`"});
        execution["detail"]["reports"][0] =
            json!({"scenario_id": "context_pressure", "available": false, "report": null});
    }
    assert!(h.changed("plan-1").await.admitted);
    h.drain().await;
    let outcome = h.cycle(&cycle_id).await.outcome.unwrap();
    assert_eq!(outcome.code, OutcomeCodeV1::Inconclusive);
    assert!(
        outcome.detail.starts_with("baseline unusable: ")
            && outcome.detail.ends_with(
                "; the execution failed: Preparing the execution failed at `prepare resolve`"
            ),
        "{}",
        outcome.detail
    );
}

/// A history of `model` runs of one test with these function-call errors.
fn history(model: &str, errors: &[f64]) -> Value {
    json!({"observations": [{"subject_model": model, "runs": errors.iter()
        .map(|errors| json!({"status": "passed", "function_call_errors": errors})).collect::<Vec<_>>()}]})
}

#[tokio::test]
async fn a_function_call_error_objective_measures_the_test_whose_recorded_runs_show_them() {
    let mut world = World::new();
    // The spec that names errors is not where the subject makes them.
    world.histories.insert(
        "tool_contract_recovery".into(),
        history("m", &[0.0, 0.0, 0.0]),
    );
    world
        .histories
        .insert("context_pressure".into(), history("m", &[4.0, 6.0]));
    // A test whose history cannot be read is left out, the objective is not.
    world
        .histories
        .insert("subagent_validation_failure".into(), Value::Null);
    let h = Harness::start(world).await;
    h.configure(10.0).await;
    for objective in [
        "Diminuir a quantidade de errors nas chamadas de função iii",
        "I want to decrease the amount of function call errors",
    ] {
        let cycle_id = h.start_cycle(json!({"objective": objective})).await;
        h.drain().await;
        let record = h.cycle(&cycle_id).await;
        let coverage = record.coverage.unwrap();
        assert_eq!(coverage.need, "function_call_errors", "{objective}");
        assert_eq!(coverage.selected.len(), 1, "{:?}", coverage.gaps);
        assert_eq!(coverage.selected[0].scenario_id, "context_pressure");
        assert!(
            coverage.gaps.iter().any(|gap| gap
                .starts_with("subagent_validation_failure: its recorded runs could not be read")),
            "{:?}",
            coverage.gaps
        );
        assert!(
            coverage.selected[0]
                .rationale
                .contains("5.00 FunctionCallErrors per run (2 run(s)"),
            "{}",
            coverage.selected[0].rationale
        );
        let criterion = record.plan.unwrap().criterion;
        assert_eq!(
            (criterion.metric, criterion.direction, criterion.min_effect),
            (
                CriterionMetricV1::FunctionCallErrors,
                DirectionV1::Decrease,
                1.0
            )
        );
    }
    // The history asked for is the configured subject's.
    let asked = h.world().calls_to("e2e::dashboard::test-history-get");
    assert!(
        asked.iter().all(|call| call["subject_model"] == "m"),
        "{asked:?}"
    );
}

#[tokio::test]
async fn an_error_objective_without_recorded_errors_has_no_coverage_and_spends_nothing() {
    let mut world = World::new();
    world
        .histories
        .insert("tool_contract_recovery".into(), history("m", &[0.0, 0.0]));
    let h = Harness::start(world).await;
    h.configure(10.0).await;
    let cycle_id = h
        .start_cycle(json!({"objective": "fewer function call errors"}))
        .await;
    h.drain().await;
    let record = h.cycle(&cycle_id).await;
    let outcome = record.outcome.unwrap();
    assert_eq!(outcome.code, OutcomeCodeV1::NoCoverage);
    assert!(
        outcome
            .detail
            .contains("no recorded E2E run of the subject model"),
        "{}",
        outcome.detail
    );
    assert_eq!(h.world().starts, 0);
}

#[tokio::test]
async fn skills_and_provider_fallback_objectives_have_no_coverage_and_spend_nothing() {
    let h = Harness::start(World::new()).await;
    h.configure(10.0).await;
    for (objective, gap) in [
        (
            "Measure the effect of skills on selection and cost",
            "skill",
        ),
        (
            "Improve provider/model fallback when a model fails",
            "tool_contract_recovery covers a changed tool contract",
        ),
        ("Make it nicer", "no need"),
    ] {
        let cycle_id = h.start_cycle(json!({"objective": objective})).await;
        h.drain().await;
        let record = h.cycle(&cycle_id).await;
        assert_eq!(record.status, CycleStatusV1::Completed, "{objective}");
        let outcome = record.outcome.unwrap();
        assert_eq!(outcome.code, OutcomeCodeV1::NoCoverage, "{objective}");
        assert!(
            outcome.detail.contains(gap),
            "{objective}: {}",
            outcome.detail
        );
        assert!(record.plan.is_none());
    }
    assert_eq!(h.world().starts, 0);
    assert_eq!(h.count(state::BUDGET_SCOPE), 0);
}

#[tokio::test]
async fn an_unconfigured_loop_or_an_exhausted_budget_dispatches_nothing() {
    let h = Harness::start(World::new()).await;
    let unconfigured = h.start_cycle(json!({"objective": OBJECTIVE})).await;
    h.drain().await;
    let record = h.cycle(&unconfigured).await;
    assert_eq!(record.status, CycleStatusV1::Failed);
    assert!(record.failure.unwrap().contains("not configured"));
    assert!(record.coverage.unwrap().covered);

    h.configure(0.1).await;
    let poor = h.start_cycle(json!({"objective": OBJECTIVE})).await;
    h.drain().await;
    let record = h.cycle(&poor).await;
    assert_eq!(record.outcome.unwrap().code, OutcomeCodeV1::Budget);
    assert!(
        record.plan.is_some(),
        "the plan was frozen before the budget refused"
    );
    assert_eq!(h.world().starts, 0);
}

#[tokio::test]
async fn an_unanswered_dispatch_is_reconciled_by_key_after_a_label_rename() {
    let mut world = World::new();
    world.unanswered_start = true;
    let h = Harness::start(world).await;
    h.configure(10.0).await;
    let cycle_id = h.start_cycle(json!({"objective": OBJECTIVE})).await;
    h.drain().await;
    let record = h.cycle(&cycle_id).await;
    assert_eq!(
        (record.status, record.stage),
        (CycleStatusV1::Waiting, CycleStageV1::Baseline)
    );
    assert_eq!(record.executions[0].state, ExecutionStateV1::Dispatching);
    h.world().executions.get_mut("plan-1").unwrap()["manifest"]["executions"][0]["label"] =
        json!("renamed after dispatch");
    // The sweep reconciles it by the immutable key despite the rename.
    runtime::sweep(&h.deps).await.unwrap();
    h.drain().await;
    let record = h.cycle(&cycle_id).await;
    assert_eq!(record.stage, CycleStageV1::AwaitingExecution);
    assert_eq!(record.executions[0].execution_id.as_deref(), Some("plan-1"));
    assert_eq!(h.world().starts, 1);
}

#[tokio::test]
async fn an_unanswered_dispatch_refuses_the_same_label_with_a_foreign_key() {
    let mut world = World::new();
    world.unanswered_start = true;
    let h = Harness::start(world).await;
    h.configure(10.0).await;
    let cycle_id = h.start_cycle(json!({"objective": OBJECTIVE})).await;
    h.drain().await;
    let label = format!("eval:{cycle_id}:baseline");
    h.world().executions.remove("plan-1");
    let mut foreign = bundle("evil", &label, "passed", full());
    foreign["manifest"]["executions"][0]["idempotency_key"] = json!("foreign:dispatch");
    h.world().executions.insert("evil".into(), foreign);

    runtime::sweep(&h.deps).await.unwrap();
    h.drain().await;

    let record = h.cycle(&cycle_id).await;
    assert_eq!(record.status, CycleStatusV1::Waiting);
    assert_eq!(record.executions[0].state, ExecutionStateV1::Dispatching);
    assert!(record.executions[0].execution_id.is_none());
    assert!(record.analyses.is_empty());
    assert_eq!(h.world().starts, 1);
}

#[tokio::test]
async fn an_unanswered_dispatch_refuses_duplicate_or_partial_key_results() {
    for partial in [false, true] {
        let mut world = World::new();
        world.unanswered_start = true;
        let h = Harness::start(world).await;
        h.configure(10.0).await;
        let cycle_id = h.start_cycle(json!({"objective": OBJECTIVE})).await;
        h.drain().await;
        let key = format!("{cycle_id}:baseline");
        if partial {
            h.world().executions_next_cursor = Some("more".into());
        } else {
            let mut duplicate = bundle("duplicate", "other label", "running", Vec::new());
            duplicate["manifest"]["executions"][0]["idempotency_key"] = json!(key);
            h.world().executions.insert("duplicate".into(), duplicate);
        }

        runtime::sweep(&h.deps).await.unwrap();
        h.drain().await;

        let record = h.cycle(&cycle_id).await;
        assert_eq!(record.executions[0].state, ExecutionStateV1::Dispatching);
        assert!(record.executions[0].execution_id.is_none());
        assert_eq!(h.world().starts, 1);
    }
}

#[tokio::test]
async fn cancel_stops_the_dispatched_execution_and_beats_late_steps() {
    let h = Harness::start(World::new()).await;
    h.configure(10.0).await;
    let cycle_id = h.start_cycle(json!({"objective": OBJECTIVE})).await;
    h.drain().await;
    let stale = h.cycle(&cycle_id).await.step;
    let cancelled = runtime::cycle_cancel(
        &h.deps,
        CycleIdRequestV1 {
            cycle_id: cycle_id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(cancelled.cancelled);
    assert_eq!(
        h.world().calls_to("e2e::dashboard::execution-cancel"),
        [json!({"execution_id": "plan-1"})]
    );
    let late = runtime::step(
        &h.deps,
        StepRequestV1 {
            cycle_id: cycle_id.clone(),
            step: stale,
        },
    )
    .await
    .unwrap();
    assert!(late.skipped);
    h.finish("plan-1", "passed", full());
    h.changed("plan-1").await;
    h.drain().await;
    let record = h.cycle(&cycle_id).await;
    assert_eq!(record.status, CycleStatusV1::Cancelled);
    assert!(record.analyses.is_empty());
    assert!(
        !runtime::cycle_cancel(&h.deps, CycleIdRequestV1 { cycle_id })
            .await
            .unwrap()
            .cancelled
    );
}

#[tokio::test]
async fn an_existing_execution_is_analyzed_cheaply_with_known_and_unknown_findings() {
    let h = Harness::start(World::new()).await;
    // Fully assessed and held: expected behavior, nothing to do.
    h.world()
        .executions
        .insert("held".into(), bundle("held", "", "passed", full()));
    let held = h.start_cycle(json!({"execution_id": "held"})).await;
    h.drain().await;
    let record = h.cycle(&held).await;
    assert_eq!(
        record.status,
        CycleStatusV1::Completed,
        "{:?}",
        record.history
    );
    assert_eq!(record.outcome.unwrap().code, OutcomeCodeV1::NoAction);
    assert!(record.analyses[0]
        .findings
        .iter()
        .any(|finding| finding.class == FindingClassV1::ExpectedBehavior));

    // `passed` but task_incomplete with a lost criterion: candidate work.
    let incomplete = vec![run(
        "r1",
        "passed",
        "task_incomplete",
        90_000.0,
        &[Some(10), Some(0), Some(10), None],
    )];
    h.world()
        .executions
        .insert("weak".into(), bundle("weak", "", "passed", incomplete));
    let weak = h.start_cycle(json!({"execution_id": "weak"})).await;
    h.drain().await;
    let record = h.cycle(&weak).await;
    // Pertinent findings, but no candidate work is configured: inconclusive
    // with the reason, no paid work.
    assert_eq!(record.status, CycleStatusV1::Completed);
    assert_eq!(
        record.outcome.clone().unwrap().code,
        OutcomeCodeV1::Inconclusive
    );
    let analysis = &record.analyses[0];
    assert!(analysis
        .findings
        .iter()
        .any(|finding| finding.class == FindingClassV1::Defect));
    // The unevaluated criterion is named, as a finding or an unknown.
    assert!(
        analysis
            .unknowns
            .iter()
            .any(|unknown| unknown.contains("concise_report"))
            || analysis
                .findings
                .iter()
                .any(|finding| finding.observation.contains("concise_report"))
    );
    assert!(analysis
        .unknowns
        .iter()
        .any(|unknown| unknown.contains("child transcripts")));
    assert!(!analysis.child_transcripts_complete);
    assert!(!analysis
        .findings
        .iter()
        .any(|finding| finding.class == FindingClassV1::ExpectedBehavior));

    // No assessment at all is insufficient evidence, never expected.
    let mut blind = run("r1", "passed", "completed", 1.0, &[]);
    blind["criteria"] = json!([]);
    h.world()
        .executions
        .insert("blind".into(), bundle("blind", "", "passed", vec![blind]));
    let blind = h.start_cycle(json!({"execution_id": "blind"})).await;
    h.drain().await;
    let record = h.cycle(&blind).await;
    // Never expected behavior and never Harness candidate work.
    assert!(record.analyses[0].findings.iter().all(|finding| {
        finding.class != FindingClassV1::ExpectedBehavior && !finding.class.is_pertinent()
    }));
    assert_eq!(h.world().starts, 0, "the first pass starts no campaign");
}

#[tokio::test]
async fn a_running_execution_input_waits_and_converges_with_its_completion_event() {
    let h = Harness::start(World::new()).await;
    h.world()
        .executions
        .insert("live".into(), bundle("live", "", "running", Vec::new()));
    let cycle_id = h.start_cycle(json!({"execution_id": "live"})).await;
    h.drain().await;
    assert_eq!(
        h.cycle(&cycle_id).await.stage,
        CycleStageV1::AwaitingExecution
    );
    let mut failing = run("r1", "hard_gate_failed", "completed", 1.0, &[Some(0); 4]);
    failing["failures"] = json!([{"status": "hard_gate_failed", "domain": "subject",
        "phase": "evaluation", "code": "gate", "message": "charter missing"}]);
    h.finish("live", "failed", vec![failing]);
    let response = h.changed("live").await;
    assert_eq!(response.cycle_id.as_deref(), Some(cycle_id.as_str()));
    h.drain().await;
    assert_eq!(
        h.count(state::CYCLE_SCOPE),
        1,
        "both inputs converge on one cycle"
    );
    let record = h.cycle(&cycle_id).await;
    assert_eq!(
        record.outcome.clone().unwrap().code,
        OutcomeCodeV1::Inconclusive
    );
    // Only the recorded failure reached Sentinel, with its answer kept.
    let ingests = h.world().calls_to("sentinel::reports::ingest");
    assert_eq!(ingests.len(), 1);
    assert_eq!(ingests[0]["observations"].as_array().unwrap().len(), 1);
    assert_eq!(ingests[0]["observations"][0]["message"], "charter missing");
    assert!(ingests[0]["observations"][0]["expected_errors"].is_null());
    let sentinel = record.analyses[0].sentinel.clone().unwrap();
    assert_eq!((sentinel.observations, sentinel.recorded), (1, Some(1)));
    assert_eq!(sentinel.results[0]["group_id"], "g-1");
}

#[tokio::test]
async fn events_and_the_checkpoint_admit_once_and_analysis_survives_retention() {
    let h = Harness::start(World::new()).await;
    // The first sweep only sets the checkpoint: history is not backfilled.
    h.world().executions.insert("old".into(), {
        let mut old = bundle("old", "", "passed", full());
        old["manifest"]["executions"][0]["started_at"] = json!("2000-01-01T00:00:00Z");
        old
    });
    runtime::sweep(&h.deps).await.unwrap();
    assert_eq!(h.count(state::CYCLE_SCOPE), 0);
    // A terminal execution whose slot still runs holds the checkpoint.
    let mut partial = bundle("partial", "", "passed", full());
    partial["detail"]["reports"]
        .as_array_mut()
        .unwrap()
        .push(json!({"scenario_id": "context_pressure", "available": false, "report": null, "state": "running"}));
    h.world().executions.insert("partial".into(), partial);
    h.world()
        .executions
        .insert("done".into(), bundle("done", "", "passed", full()));
    let swept = runtime::sweep(&h.deps).await.unwrap();
    assert_eq!(swept.admitted, 1, "only the complete one");
    let checkpoint = state::get_admission_checkpoint(&h.iii)
        .await
        .unwrap()
        .unwrap();
    assert!(
        checkpoint.since < eval::ids::parse_ms("2099-01-01T00:00:00Z").unwrap(),
        "held before the partial one"
    );
    // Redelivery and a restart admit nothing twice.
    assert!(!h.changed("done").await.admitted);
    let restarted = Deps::new(h.iii.clone());
    assert!(
        !runtime::admit_execution(&restarted, "done")
            .await
            .unwrap()
            .0
            .admitted
    );
    assert_eq!(h.count(state::CYCLE_SCOPE), 1);
    assert_eq!(h.count(state::EVIDENCE_SCOPE), 1);
    // The source is retained away (and its containers gone): the stored copy suffices.
    h.world().executions.remove("done");
    h.drain().await;
    let cycles = runtime::cycle_list(&h.deps, CycleListRequestV1::default())
        .await
        .unwrap()
        .cycles;
    assert_eq!(cycles[0].analyses.len(), 1);
    assert_eq!(
        cycles[0].analyses[0].identity.harness_version.as_deref(),
        Some("h-1")
    );
}

#[tokio::test]
async fn an_execution_its_event_admitted_never_opens_a_second_cycle_on_start() {
    let h = Harness::start(World::new()).await;
    h.world()
        .executions
        .insert("done".into(), bundle("done", "", "passed", full()));
    assert!(h.changed("done").await.admitted);
    h.drain().await;
    let started = h.start_cycle(json!({"execution_id": "done"})).await;
    h.drain().await;
    let record = h.cycle(&started).await;
    let outcome = record.outcome.unwrap();
    assert_eq!(outcome.code, OutcomeCodeV1::Inconclusive);
    assert!(
        outcome.detail.contains("already admitted by cycle"),
        "{}",
        outcome.detail
    );
    assert!(record.analyses.is_empty(), "nothing is analyzed twice");
}

#[tokio::test]
async fn a_busy_runner_dispatches_the_same_work_again_on_the_next_poll() {
    let mut world = World::new();
    world.busy_starts = 1;
    let h = Harness::start(world).await;
    h.configure(10.0).await;
    let cycle_id = h.start_cycle(json!({"objective": OBJECTIVE})).await;
    h.drain().await;
    let record = h.cycle(&cycle_id).await;
    assert_eq!(
        (record.status, record.stage),
        (CycleStatusV1::Waiting, CycleStageV1::Baseline),
        "{:?}",
        record.history
    );
    assert!(record.executions.is_empty());
    runtime::sweep(&h.deps).await.unwrap();
    h.drain().await;
    let record = h.cycle(&cycle_id).await;
    assert_eq!(
        record.stage,
        CycleStageV1::AwaitingExecution,
        "{:?}",
        record.history
    );
    let keys: Vec<Value> = h
        .world()
        .calls_to("e2e::dashboard::execution-start")
        .iter()
        .map(|start| start["idempotency_key"].clone())
        .collect();
    let key = json!(format!("{cycle_id}:baseline"));
    assert_eq!(keys, [key.clone(), key]);
    let ledger = state::get_ledger(&h.iii, &cycle_id).await.unwrap().unwrap();
    assert_eq!(
        ledger.reservations.len(),
        1,
        "one reservation for both starts"
    );
}

#[tokio::test]
async fn a_budget_limit_ends_a_waiting_cycle_and_stops_its_execution() {
    let h = Harness::start(World::new()).await;
    h.configure(10.0).await;
    let cycle_id = h.start_cycle(json!({"objective": OBJECTIVE})).await;
    h.drain().await;
    assert_eq!(
        h.cycle(&cycle_id).await.stage,
        CycleStageV1::AwaitingExecution
    );
    // The cycle's time limit has passed while the baseline runs.
    let mut ledger = state::get_ledger(&h.iii, &cycle_id).await.unwrap().unwrap();
    ledger.started_at = 0;
    h.world().state.insert(
        (state::BUDGET_SCOPE.into(), cycle_id.clone()),
        serde_json::to_value(&ledger).unwrap(),
    );
    assert_eq!(runtime::sweep(&h.deps).await.unwrap().expired, 1);
    let record = h.cycle(&cycle_id).await;
    assert_eq!(record.outcome.unwrap().code, OutcomeCodeV1::Budget);
    assert_eq!(
        h.world().calls_to("e2e::dashboard::execution-cancel"),
        [json!({"execution_id": "plan-1"})]
    );
    assert_eq!(
        h.count(state::OPEN_SCOPE),
        0,
        "an ended cycle leaves the open index"
    );
}

#[tokio::test]
async fn the_execution_picker_says_what_a_cycle_would_restore_or_which_cycle_has_it() {
    let h = Harness::start(World::new()).await;
    h.configure(10.0).await;
    // A cycle's own dispatch is never offered, finished or not.
    let objective = h.start_cycle(json!({"objective": OBJECTIVE})).await;
    h.drain().await;
    h.finish("plan-1", "passed", full());
    let lost = vec![run(
        "r1",
        "passed",
        "completed",
        90_000.0,
        &[Some(10), Some(5), Some(10), Some(10)],
    )];
    for (id, status, runs) in [
        ("weak", "passed", lost),
        ("done", "passed", full()),
        ("live", "running", Vec::new()),
    ] {
        h.world()
            .executions
            .insert(id.into(), bundle(id, "", status, runs));
    }
    let admitted = h.changed("done").await;

    let options = runtime::executions(&h.deps, ExecutionsRequestV1::default())
        .await
        .unwrap()
        .executions;
    let option = |id: &str| options.iter().find(|option| option.execution_id == id);
    assert!(option("plan-1").is_none() && option("live").is_none());
    let done = option("done").unwrap();
    assert_eq!(done.cycle_id, admitted.cycle_id);
    assert!(done.scenarios.is_empty(), "a claimed execution is not read");
    let weak = option("weak").unwrap();
    assert_eq!(weak.cycle_id, None);
    assert_eq!(weak.scenarios[0].scenario_id, "context_pressure");
    let why = weak.scenarios[0].lost.as_deref().unwrap();
    assert!(why.contains("needles_recovered"), "{why}");

    let listed = runtime::cycle_list(&h.deps, CycleListRequestV1::default())
        .await
        .unwrap();
    assert!(
        listed.usage[&objective].spend_usd > 0.0,
        "the reserved baseline"
    );
}

/// A run whose agent called `coder::update-file` and got an error back.
fn failed_call(run_id: &str) -> Value {
    let mut run = run(run_id, "passed", "completed", 1000.0, &[Some(10); 4]);
    run["transcript"]["messages"] = json!([
        {"entry_id": "e1", "message": {"role": "assistant", "stop_reason": "function_call",
            "model": "m", "provider": "p", "timestamp": 0,
            "usage": {"output": 2000, "reasoning": 900},
            "content": [{"type": "text", "text": "Rewriting the file."},
                {"type": "function_call", "id": "c1", "function_id": "coder::update-file",
                 "arguments": {"path": "src/reconcile.py"}}]}},
        {"entry_id": "e2", "message": {"role": "function_result", "timestamp": 0,
            "function_call_id": "c1", "function_id": "coder::update-file", "is_error": true,
            "content": [{"type": "text", "text": "did not run: the arguments were truncated"}]}}
    ]);
    run
}

/// An error objective measured on one run with a failed call.
async fn read_failed_call(world: World, judged: bool) -> CycleRecordV1 {
    let mut world = world;
    world
        .histories
        .insert("context_pressure".into(), history("m", &[2.0]));
    let h = Harness::start(world).await;
    if judged {
        h.configure_judged().await;
    } else {
        h.configure(10.0).await;
    }
    let cycle_id = h
        .start_cycle(json!({"objective": "fewer function call errors"}))
        .await;
    h.drain().await;
    h.finish("plan-1", "passed", vec![failed_call("r1")]);
    h.changed("plan-1").await;
    h.drain().await;
    h.cycle(&cycle_id).await
}

#[tokio::test]
async fn jev_reads_each_failed_call_and_the_harness_ones_become_improvement_points() {
    let mut world = World::new();
    world.jev_class = Some("harness");
    let record = read_failed_call(world, true).await;
    let insight = record.insight.clone().expect("the transcripts were read");
    assert_eq!(
        insight.source,
        Some(InsightSourceV1::Jev),
        "{}",
        insight.reason
    );
    assert_eq!(insight.sites.len(), 1);
    let site = &insight.sites[0];
    assert_eq!(site.function_id, "coder::update-file");
    assert_eq!(site.class, Some(SiteClassV1::Harness));
    assert!(site.step.contains("output_tokens=2000"), "{}", site.step);
    assert_eq!(insight.points.len(), 1);
    assert_eq!(insight.points[0].sites, std::slice::from_ref(&site.site_id));
    // Without candidate work the points are the result: nothing is tried.
    let outcome = record.outcome.unwrap();
    assert_eq!(outcome.code, OutcomeCodeV1::Inconclusive);
    assert!(
        outcome.detail.contains("1 the Harness's") && outcome.detail.contains("config.candidate"),
        "{}",
        outcome.detail
    );
}

#[tokio::test]
async fn failed_calls_jev_lays_on_the_model_end_the_cycle_before_any_proposal() {
    let mut world = World::new();
    world.jev_class = Some("model");
    let record = read_failed_call(world, true).await;
    let insight = record.insight.unwrap();
    assert!(insight.points.is_empty());
    let outcome = record.outcome.unwrap();
    assert_eq!(outcome.code, OutcomeCodeV1::NoAction);
    assert!(
        outcome.detail.contains("1 the model's") && outcome.detail.contains("none is a failure"),
        "{}",
        outcome.detail
    );
}

#[tokio::test]
async fn without_jev_the_default_llm_reads_the_failed_calls() {
    let mut world = World::new();
    world.analyst_reply = Some("PLACEHOLDER".into());
    // The site id is derived from the execution, run and entry: read it from a
    // first pass, then answer with it.
    let first = read_failed_call(World::new(), false).await;
    let site_id = first.insight.unwrap().sites[0].site_id.clone();
    world.analyst_reply = Some(format!(
        "{{\"points\": [{{\"function_id\": \"coder::update-file\", \"summary\": \"Tell the model to split large edits.\", \"sites\": [\"{site_id}\"]}}]}}"
    ));
    let record = read_failed_call(world, false).await;
    let insight = record.insight.unwrap();
    assert_eq!(
        insight.source,
        Some(InsightSourceV1::Llm),
        "{}",
        insight.reason
    );
    assert!(
        insight.reason.contains("no judge is configured"),
        "{}",
        insight.reason
    );
    assert_eq!(insight.points.len(), 1);
    assert_eq!(
        insight.points[0].summary,
        "Tell the model to split large edits."
    );
    // Neither Jev nor the LLM: inconclusive, with both reasons.
    assert_eq!(first.outcome.unwrap().code, OutcomeCodeV1::Inconclusive);
}

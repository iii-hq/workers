//! The monitor flow over the real SDK wire protocol against an in-memory
//! engine: session end → admission → collection → diagnostics → Jev → LLM
//! (when routed) → result. Every dependency (state, queue, Harness,
//! session-manager, router, judge, E2E) is a local mock; no credentials or
//! remote models are used.

#[path = "../../judge-typesafe/tests/support/fake_engine.rs"]
mod fake_engine;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use eval::contract::*;
use eval::error::EvalError;
use eval::events::EvalEvents;
use eval::runtime::{self, Deps};
use eval::{ids, state};
use harness::functions::metrics::{SessionMetricsResponseV1, SessionUsageTotalsV1};
use harness::types::content::ContentBlock;
use harness::types::event::StopReason;
use harness::types::message::{
    empty_assistant, AgentMessage, FunctionResultMessage, FunctionResultRoleTag,
};
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};

const ROOT: &str = "s_root";
const CHILD: &str = "s_child";
const TURN: &str = "t_aaa";
const INFO: &str = "engine::functions::info";
const PAGE: usize = 3;

#[derive(Clone, Copy, PartialEq)]
enum JudgeMode {
    Answer(&'static str, f64),
    ProviderError(&'static str),
    Malformed,
    /// Hold the answer until `judge::cancel`, then answer `cancelled`.
    HoldUntilCancel,
    /// Answer an E2E pairing question with this option key.
    Pick(&'static str, f64),
    /// TypeSafe's HTTP 402 (no credits), with the provider's explanation.
    Billing,
}

struct World {
    state: BTreeMap<(String, String), Value>,
    steps: VecDeque<Value>,
    calls: Vec<(String, Value)>,
    status: HashMap<String, Value>,
    entries: HashMap<String, Vec<Value>>,
    tree: Value,
    descendants_running: bool,
    judge: JudgeMode,
    lose_next_send_reply: bool,
    sends: HashMap<String, String>,
    analyst_result: Value,
    analyst_fails: bool,
    /// The Harness's `stop_reason` for a `completed` analyst turn.
    analyst_stop_reason: Option<&'static str>,
    executions: HashMap<String, Value>,
    /// What `e2e::dashboard::executions-list` answers; `None` is a down E2E.
    executions_list: Option<Value>,
    held_judge: Option<Value>,
    extra_frames: Vec<Value>,
}

impl World {
    fn new() -> Self {
        let mut world = Self {
            state: BTreeMap::new(),
            steps: VecDeque::new(),
            calls: Vec::new(),
            status: HashMap::new(),
            entries: HashMap::new(),
            tree: json!({
                "root_session_id": ROOT,
                "sessions": [
                    {"session_id": ROOT, "depth": 0},
                    {"session_id": CHILD, "parent_session_id": ROOT, "parent_turn_id": TURN, "depth": 1}
                ],
                "complete": true
            }),
            descendants_running: false,
            judge: JudgeMode::Answer("needs_investigation", 0.92),
            lose_next_send_reply: false,
            sends: HashMap::new(),
            analyst_result: json!({"suggestions": []}),
            analyst_fails: false,
            analyst_stop_reason: None,
            executions: HashMap::new(),
            executions_list: None,
            held_judge: None,
            extra_frames: Vec::new(),
        };
        world
            .status
            .insert(ROOT.into(), status(ROOT, TURN, "completed", None));
        world
            .status
            .insert(CHILD.into(), status(CHILD, "t_child", "completed", None));
        world.entries.insert(ROOT.into(), rediscovery_transcript());
        world.entries.insert(
            CHILD.into(),
            vec![
                user("e_spawn_1", "Inspect the profile."),
                assistant_text("e_t_child_0_assistant", "Done."),
            ],
        );
        world
    }

    fn calls_to(&self, function: &str) -> Vec<Value> {
        self.calls
            .iter()
            .filter(|(id, _)| id == function)
            .map(|(_, payload)| payload.clone())
            .collect()
    }

    /// `None` drops the reply, like a lost response.
    fn respond(
        &mut self,
        function: &str,
        data: &Value,
        invocation_id: &Value,
    ) -> Option<Result<Value, String>> {
        let scope_key = || {
            (
                data["scope"].as_str().unwrap_or_default().to_string(),
                data["key"].as_str().unwrap_or_default().to_string(),
            )
        };
        Some(Ok(match function {
            "state::get" => self.state.get(&scope_key()).cloned().unwrap_or(Value::Null),
            "state::set" => {
                self.state.insert(scope_key(), data["value"].clone());
                json!({})
            }
            "state::delete" => {
                self.state.remove(&scope_key());
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
            "router::models::list" => json!({"models": [
                {"id": "analyst-model", "provider": "analyst-provider", "context_window": 200000,
                 "max_output_tokens": 8192, "supports_thinking": true}
            ]}),
            "harness::status" => self
                .status
                .get(data["session_id"].as_str().unwrap())
                .cloned()
                .unwrap_or(Value::Null),
            "harness::session-tree" if data["root_session_id"] == ROOT => self.tree.clone(),
            "harness::metrics" => {
                let root = data["root_session_id"].as_str().unwrap();
                serde_json::to_value(SessionMetricsResponseV1 {
                    root_session_id: root.into(),
                    complete: !(root == ROOT && self.descendants_running),
                    totals: SessionUsageTotalsV1 {
                        sessions: 2,
                        turns: 2,
                        function_calls: 2,
                        input_tokens: Some(1_200),
                        output_tokens: Some(80),
                        ..SessionUsageTotalsV1::default()
                    },
                    by_session: Vec::new(),
                    traces: None,
                })
                .unwrap()
            }
            "session::get" => json!({"meta": {"session_id": data["session_id"],
                "title": "Schedule the follow-up", "metadata": {}}}),
            "judge::models::list" => json!({"status": "ok",
                "models": [{"name": "jev-test-1", "description": "", "release_date": ""}],
                "stats": {"attempts": 1, "requests": 1, "questions": 0, "input_tokens": 0,
                          "output_tokens": 0, "elapsed_ms": 3, "usage_complete": true}}),
            "session::messages" => {
                let entries = self
                    .entries
                    .get(data["session_id"].as_str().unwrap())
                    .cloned()
                    .unwrap_or_default();
                let start: usize = data["cursor"].as_str().map_or(0, |c| c.parse().unwrap());
                let end = (start + PAGE).min(entries.len());
                json!({
                    "messages": entries[start..end],
                    "next_cursor": (end < entries.len()).then(|| end.to_string()),
                })
            }
            "judge::evaluate" => match self.judge {
                JudgeMode::Answer(choice, confidence) => judge_ok(choice, confidence),
                JudgeMode::ProviderError(code) => json!({
                    "status": "error", "code": code,
                    "stats": {"attempts": 0, "requests": 0, "questions": 0, "input_tokens": 0,
                              "output_tokens": 0, "elapsed_ms": 1, "usage_complete": false}
                }),
                JudgeMode::Malformed => json!({"status": "ok", "model": "jev-test", "results": {}}),
                JudgeMode::Pick(choice, confidence) => judge_pick(data, choice, confidence),
                JudgeMode::Billing => json!({
                    "status": "error", "code": "http", "http_status": 402,
                    "provider_error": {"detail": {"error_type": "billing_error",
                        "message": "Your organization has no available TypeSafe API credits."},
                        "truncated": false},
                    "stats": {"attempts": 1, "requests": 1, "questions": 0, "input_tokens": 0,
                              "output_tokens": 0, "elapsed_ms": 40, "usage_complete": false}
                }),
                JudgeMode::HoldUntilCancel => {
                    self.held_judge = Some(invocation_id.clone());
                    return None;
                }
            },
            "judge::cancel" => {
                if let Some(held) = self.held_judge.take() {
                    self.extra_frames.push(json!({
                        "type": "invocationresult", "invocation_id": held,
                        "function_id": "judge::evaluate",
                        "result": {"status": "error", "code": "cancelled",
                            "stats": {"attempts": 1, "requests": 0, "questions": 0,
                                "input_tokens": 0, "output_tokens": 0, "elapsed_ms": 900,
                                "usage_complete": false}}
                    }));
                }
                json!({"status": "ok", "cancelled": true})
            }
            "harness::send" => {
                let session_id = data["session_id"].as_str().unwrap().to_string();
                let key = data["idempotency_key"].as_str().unwrap().to_string();
                let deduplicated = self.sends.contains_key(&key);
                let turn = self
                    .sends
                    .entry(key)
                    .or_insert_with(|| format!("t_analyst{}", self.status.len()))
                    .clone();
                if !deduplicated {
                    let (status, error) = if self.analyst_fails {
                        ("failed", Some("provider unavailable"))
                    } else {
                        ("completed", None)
                    };
                    let mut report = status_report(&session_id, &turn, status, error);
                    if error.is_none() {
                        report["result"] = self.analyst_result.clone();
                    }
                    if let Some(reason) = self.analyst_stop_reason {
                        report["stop_reason"] = json!(reason);
                        report["max_turns"] = json!(32);
                    }
                    self.status.insert(session_id.clone(), report);
                    let mut reply = empty_assistant("analyst-provider", "analyst-model");
                    reply.content = vec![ContentBlock::text(self.analyst_result.to_string())];
                    self.entries.insert(
                        session_id.clone(),
                        vec![entry(
                            &format!("e_{turn}_0_assistant"),
                            AgentMessage::Assistant(reply),
                        )],
                    );
                }
                if std::mem::take(&mut self.lose_next_send_reply) {
                    return Some(Err("connection reset before the reply".into()));
                }
                json!({"session_id": session_id, "turn_id": turn, "accepted": true,
                       "deduplicated": deduplicated})
            }
            "harness::stop" => json!({"stopped": true}),
            "e2e::dashboard::executions-list" => match &self.executions_list {
                Some(list) => list.clone(),
                None => {
                    return Some(Err(
                        "Function e2e::dashboard::executions-list not found in namespace p.".into(),
                    ))
                }
            },
            "e2e::dashboard::execution-get" => {
                match self.executions.get(data["execution_id"].as_str().unwrap()) {
                    Some(bundle) => bundle.clone(),
                    None => return Some(Err("execution not found".into())),
                }
            }
            other => return Some(Err(format!("unexpected call to {other}"))),
        }))
    }
}

fn status_report(session_id: &str, turn_id: &str, status: &str, error: Option<&str>) -> Value {
    json!({
        "session_id": session_id, "turn_id": turn_id, "status": status, "step": 3,
        "turn_count": 1, "children": [], "expects_wake": false, "pending_function_calls": [],
        "depth": 0, "result": "ok", "result_error": error,
    })
}

fn status(session_id: &str, turn_id: &str, status: &str, error: Option<&str>) -> Value {
    status_report(session_id, turn_id, status, error)
}

fn judge_ok(choice: &str, confidence: f64) -> Value {
    let options = [
        "expected_behavior",
        "insufficient_evidence",
        "needs_investigation",
    ];
    let rest = (1.0 - confidence) / 2.0;
    let probabilities: BTreeMap<_, _> = options
        .iter()
        .map(|option| (*option, if *option == choice { confidence } else { rest }))
        .collect();
    json!({
        "status": "ok", "model": "jev-test-1",
        "results": {"session": {
            "answers": {"investigation": {"type": "choice", "choice": choice,
                "probabilities": probabilities, "confidence": confidence}},
            "usage": {"input_tokens": 410, "output_tokens": 3}
        }},
        "stats": {"attempts": 1, "requests": 1, "questions": 1, "input_tokens": 410,
                  "output_tokens": 3, "elapsed_ms": 25, "usage_complete": true}
    })
}

/// A pairing answer over exactly the options the request offered.
fn judge_pick(request: &Value, choice: &str, confidence: f64) -> Value {
    let criteria = request["evaluations"][0]["questions"]["pair"]["criteria"]
        .as_object()
        .expect("a Choice question with criteria");
    let rest = (1.0 - confidence) / (criteria.len() as f64 - 1.0);
    let probabilities: BTreeMap<_, _> = criteria
        .keys()
        .map(|key| (key.as_str(), if key == choice { confidence } else { rest }))
        .collect();
    json!({
        "status": "ok", "model": "jev-test-1",
        "results": {"pairing": {
            "answers": {"pair": {"type": "choice", "choice": choice,
                "probabilities": probabilities, "confidence": confidence}},
            "usage": {"input_tokens": 520, "output_tokens": 4}
        }},
        "stats": {"attempts": 1, "requests": 1, "questions": 1, "input_tokens": 520,
                  "output_tokens": 4, "elapsed_ms": 30, "usage_complete": true}
    })
}

// Transcript entries serialized from the Harness's own message types.

fn entry(entry_id: &str, message: AgentMessage) -> Value {
    json!({ "entry_id": entry_id, "message": message })
}

fn user(entry_id: &str, text: &str) -> Value {
    entry(entry_id, AgentMessage::user_text(text))
}

fn assistant_text(entry_id: &str, text: &str) -> Value {
    let mut message = empty_assistant("p", "task-model");
    message.content = vec![ContentBlock::text(text)];
    entry(entry_id, AgentMessage::Assistant(message))
}

fn info_call(step: u32, call_id: &str) -> Value {
    let mut message = empty_assistant("p", "task-model");
    message.stop_reason = StopReason::FunctionCall;
    message.content = vec![ContentBlock::FunctionCall {
        id: call_id.into(),
        function_id: "agent_trigger".into(),
        arguments: json!({"function": INFO, "description": "read the contract",
            "payload": {"function_id": "crm::profile"}}),
    }];
    entry(
        &format!("e_{TURN}_{step}_assistant"),
        AgentMessage::Assistant(message),
    )
}

fn info_result(call_id: &str, shown: Value) -> Value {
    entry(
        &format!("e_{TURN}_{call_id}"),
        AgentMessage::FunctionResult(FunctionResultMessage {
            role: FunctionResultRoleTag::FunctionResult,
            function_call_id: call_id.into(),
            function_id: INFO.into(),
            content: vec![ContentBlock::text(shown.to_string())],
            details: json!({"function_id": "crm::profile", "request_schema": {"type": "object"},
                "response_schema": {"type": "object"}, "api_key": "sk-live-secret"}),
            is_error: false,
            timestamp: 1,
        }),
    )
}

fn rediscovery_transcript() -> Vec<Value> {
    let notice = json!({"role": "user", "timestamp": 1, "content": [{"type": "text",
        "text": "NOTE: the function registry changed during this conversation."}]});
    vec![
        user(
            "e_idem_task",
            "Schedule the follow-up from the old runbook.",
        ),
        info_call(0, "c1"),
        info_result(
            "c1",
            json!({"function_id": "crm::profile", "request_schema": {"type": "object"}}),
        ),
        json!({"entry_id": format!("e_{TURN}_1_notice_0"), "custom": {
            "custom_type": "model_notice",
            "data": harness::window::notice_data("registry-changed", &notice)}}),
        info_call(1, "c2"),
        info_result(
            "c2",
            json!({"function_id": "crm::profile", "contract_status": "unchanged_in_context",
                "source_function_call_id": "c1"}),
        ),
        assistant_text(
            &format!("e_{TURN}_2_assistant"),
            "Scheduled once; receipt R-1.",
        ),
    ]
}

fn suggestion(evidence_entry: &str) -> Value {
    json!({
        "title": "Registry notice triggers redundant contract discovery",
        "observation": "After a registry-changed notice the model re-fetched crm::profile and received unchanged_in_context.",
        "hypothesis": "The broad notice may prompt re-discovery even when the used contracts are unchanged.",
        "harness_component": "turn loop registry-change notice",
        "proposed_change": "Name the functions whose contracts changed in the notice.",
        "expected_effect": "Fewer redundant info calls when unrelated functions change.",
        "evidence": [{"session_id": ROOT, "entry_id": evidence_entry}],
        "limitations": "One occurrence; the notice may be unrelated.",
        "validation": {
            "scenario_id": "tool_contract_recovery",
            "reproduction": "Change an unrelated function between discovery and the next decision.",
            "invariants": ["schedule exactly once", "return the correct receipt"],
            "primary_metric": "redundant info calls after the notice",
            "expectation": "candidate makes fewer redundant calls with the same invariants",
            "non_regression_controls": ["a contract that really changes is still re-fetched"]
        }
    })
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
            let Some(reply) = world.respond(&function, &frame["data"], &frame["invocation_id"])
            else {
                return vec![];
            };
            let mut response = json!({"type": "invocationresult",
                "invocation_id": frame["invocation_id"], "function_id": function});
            match reply {
                Ok(result) => response["result"] = result,
                Err(message) => response["error"] = json!({"code": "mock", "message": message}),
            }
            let mut frames = vec![response];
            frames.append(&mut world.extra_frames);
            frames
        })
        .await;
        let iii = Arc::new(register_worker(&engine.url, InitOptions::default()));
        tokio::time::timeout(Duration::from_secs(10), async {
            while iii.get_connection_state() != iii_sdk::runtime::IIIConnectionState::Connected {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("worker connects to the fake engine");
        let deps = Deps::new(iii.clone(), EvalEvents::register(&iii));
        Self {
            world,
            deps,
            iii,
            _engine: engine,
        }
    }

    /// A fresh process on the same storage: empty in-flight set and locks.
    fn restart(&mut self) {
        self.deps = Deps::new(self.iii.clone(), EvalEvents::register(&self.iii));
    }

    fn world(&self) -> std::sync::MutexGuard<'_, World> {
        self.world.lock().unwrap()
    }

    /// Plays the queue: runs every enqueued step until none is left.
    async fn drain(&self) {
        loop {
            let next = self.world().steps.pop_front();
            let Some(payload) = next else { break };
            let request: StepRequestV1 = serde_json::from_value(payload).unwrap();
            runtime::step(&self.deps, request).await.unwrap();
        }
    }

    async fn configure(&self, enabled: bool) -> MonitorConfigV1 {
        runtime::configure(
            &self.deps,
            serde_json::from_value(json!({"enabled": enabled,
                "model": {"model": "analyst-model", "provider": "analyst-provider",
                          "thinking_level": "low"}}))
            .unwrap(),
        )
        .await
        .unwrap()
    }

    /// `configure(true)` with the codebase directory the investigation runs
    /// in (`None`: no code access).
    async fn configure_code(
        &self,
        code_repository: Option<&str>,
    ) -> Result<MonitorConfigV1, EvalError> {
        runtime::configure(
            &self.deps,
            serde_json::from_value(json!({"enabled": true,
                "model": {"model": "analyst-model", "provider": "analyst-provider",
                          "thinking_level": "low"},
                "code_repository": code_repository}))
            .unwrap(),
        )
        .await
    }

    async fn end_turn(&self, session_id: &str, turn_id: &str) -> WakeResponseV1 {
        runtime::wake(
            &self.deps,
            serde_json::from_value(json!({"session_id": session_id, "turn_id": turn_id,
                "terminal": true, "status": "completed", "timestamp": ids::now_ms()}))
            .unwrap(),
        )
        .await
        .unwrap()
    }

    async fn result(&self, evaluation_id: &str) -> EvalResultResponseV1 {
        runtime::result(
            &self.deps,
            EvaluationIdRequestV1 {
                evaluation_id: evaluation_id.into(),
            },
        )
        .await
        .unwrap()
        .expect("analysis exists")
    }

    fn records(&self) -> usize {
        self.world()
            .state
            .keys()
            .filter(|(scope, _)| scope == state::ANALYSIS_SCOPE)
            .count()
    }
}

#[tokio::test]
async fn full_flow_with_pending_children_lost_reply_and_validated_references() {
    let mut world = World::new();
    world.descendants_running = true;
    world.lose_next_send_reply = true;
    let fingerprint = ids::sha256_text(&format!("repeated_contract_discovery\n{ROOT}\n{TURN}\nc2"));
    world.analyst_result = json!({"suggestions": [
        suggestion(&format!("e_{TURN}_1_notice_0")),
        suggestion("e_invented_entry"),
    ], "signal_assessments": [
        {"fingerprint": fingerprint, "verdict": "worth_changing",
         "explanation": "The contract was unchanged; the notice was unrelated."},
        {"fingerprint": "unknown", "verdict": "likely_expected", "explanation": "invented"},
    ]});
    let h = Harness::start(world).await;

    // Inactive by default: no configuration, nothing admitted.
    assert_eq!(
        h.end_turn(ROOT, TURN).await.outcome,
        WakeOutcomeV1::Disabled
    );
    h.configure(false).await;
    assert_eq!(
        h.end_turn(ROOT, TURN).await.outcome,
        WakeOutcomeV1::Disabled
    );
    let config = h.configure(true).await;

    // Progress and descendant events never admit an analysis.
    let progress = runtime::wake(
        &h.deps,
        serde_json::from_value(json!({"session_id": ROOT, "turn_id": "t_x", "terminal": false}))
            .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(progress.outcome, WakeOutcomeV1::Progress);
    let child = runtime::wake(
        &h.deps,
        serde_json::from_value(
            json!({"session_id": CHILD, "turn_id": "t_child", "terminal": true,
            "parent": {"session_id": ROOT, "turn_id": TURN, "function_call_id": "c9"}}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(child.outcome, WakeOutcomeV1::Descendant);

    let admitted = h.end_turn(ROOT, TURN).await;
    assert_eq!(admitted.outcome, WakeOutcomeV1::Admitted);
    let evaluation_id = admitted.evaluation_id.unwrap();
    // A redelivered event reuses the analysis.
    let again = h.end_turn(ROOT, TURN).await;
    assert_eq!(again.outcome, WakeOutcomeV1::Reused);
    assert_eq!(again.evaluation_id.as_deref(), Some(evaluation_id.as_str()));
    assert_eq!(h.records(), 1);

    // Children still running: collection stays pending.
    h.drain().await;
    let record = h.result(&evaluation_id).await.record;
    assert_eq!(record.status, EvalStatusV1::Collecting);
    assert!(record.pending_reason.unwrap().contains("descendant"));
    assert!(h.world().calls_to("judge::evaluate").is_empty());

    // Children finish; the sweep resumes collection, then Jev, then the LLM.
    h.world().descendants_running = false;
    runtime::sweep(&h.deps).await.unwrap();
    h.drain().await;
    let record = h.result(&evaluation_id).await.record;
    assert_eq!(record.status, EvalStatusV1::Investigating);
    // The first send's reply was lost; the step waits instead of failing.
    assert!(record.analyst.as_ref().unwrap().turn_id.is_none());
    runtime::sweep(&h.deps).await.unwrap();
    h.drain().await;
    let record = h.result(&evaluation_id).await.record;
    let analyst = record.analyst.clone().unwrap();
    assert_eq!(analyst.session_id, ids::analyst_session(&evaluation_id));
    assert!(analyst.turn_id.is_some());

    // The investigation's own turn-completed wakes it, never observes it.
    let own = h
        .end_turn(&analyst.session_id, analyst.turn_id.as_deref().unwrap())
        .await;
    assert_eq!(own.outcome, WakeOutcomeV1::MonitorSession);
    h.drain().await;
    assert_eq!(
        h.records(),
        1,
        "the monitor's own session is never admitted"
    );

    let EvalResultResponseV1 { record, assets } = h.result(&evaluation_id).await;
    assert_eq!(
        record.status,
        EvalStatusV1::Completed,
        "{:?}",
        record.failure
    );
    assert_eq!(record.model, config.model);
    assert_eq!(record.config_revision, config.revision);
    assert_eq!(record.counters.diagnostics, 1);
    assert_eq!(record.counters.suggestions, 1);
    assert_eq!(record.counters.rejected_suggestions, 1);
    assert_eq!(
        record.source_title.as_deref(),
        Some("Schedule the follow-up")
    );
    let stages: Vec<_> = record.stages.iter().map(|stage| stage.status).collect();
    assert_eq!(
        stages,
        [
            EvalStatusV1::Queued,
            EvalStatusV1::Collecting,
            EvalStatusV1::Judging,
            EvalStatusV1::Investigating,
            EvalStatusV1::Completed
        ]
    );
    assert_eq!(record.usage.judge_calls, 1);
    assert_eq!(record.usage.judge_input_tokens, 410);
    assert!(record.usage.judge_usage_complete);
    assert_eq!(record.usage.llm_input_tokens, Some(1_200));
    assert_eq!(
        record.usage.llm_cost_usd, None,
        "unknown cost stays unknown"
    );

    let snapshot = assets.snapshot.unwrap();
    assert_eq!(snapshot.metrics_scope, MetricsScopeV1::SessionTree);
    assert_eq!(snapshot.window_turn_ids, [TURN]);
    assert_eq!(snapshot.observed_model.as_deref(), Some("task-model"));
    assert_eq!(
        snapshot.diagnostics[0].correlation,
        CorrelationV1::HarnessNoticeCorrelated
    );
    assert!(snapshot.sessions.iter().all(|session| session.in_scope));
    assert!(snapshot.coverage.context_bytes as usize <= 32 * 1024);
    let shown = serde_json::to_string(&snapshot.sessions).unwrap();
    assert!(
        !shown.contains("sk-live-secret"),
        "secrets never reach previews"
    );

    let triage = assets.triage.unwrap();
    assert_eq!(triage.model, "jev-test-1");
    assert_eq!(triage.stats.input_tokens, 410);
    assert_eq!(
        record.routing.unwrap().reasons,
        [
            RoutingReasonV1::Diagnostics,
            RoutingReasonV1::NeedsInvestigation
        ]
    );

    let investigation = assets.investigation.unwrap();
    assert_eq!(
        investigation.signal_assessments.len(),
        1,
        "unknown fingerprints are dropped"
    );
    assert_eq!(investigation.signal_assessments[0].fingerprint, fingerprint);
    assert_eq!(
        investigation.signal_assessments[0].verdict,
        SignalVerdictV1::WorthChanging
    );
    assert_eq!(
        investigation.effective_model.as_deref(),
        Some("analyst-model")
    );
    assert_eq!(
        investigation.effective_provider.as_deref(),
        Some("analyst-provider")
    );
    assert!(
        investigation.metrics.is_some(),
        "the monitor's consumption is kept apart"
    );
    assert_eq!(investigation.suggestions.len(), 1);
    assert!(investigation.rejected[0].reasons[0].contains("e_invented_entry"));

    let world = h.world();
    assert!(world
        .calls_to("session::messages")
        .iter()
        .all(|page| page["include_image_data"] == false));
    let judge_calls = world.calls_to("judge::evaluate");
    assert_eq!(judge_calls.len(), 1);
    assert_eq!(judge_calls[0]["provider"], "typesafe");
    assert!(judge_calls[0]["expires_at_unix_ms"].is_u64());
    let judge_state = judge_calls[0]["evaluations"][0]["state"].to_string();
    assert!(
        !judge_state.contains("runbook"),
        "Jev receives facts, not transcripts"
    );

    let sends = world.calls_to("harness::send");
    assert_eq!(sends.len(), 2, "one resend after the lost reply");
    assert_eq!(sends[0]["idempotency_key"], sends[1]["idempotency_key"]);
    assert_eq!(world.sends.len(), 1, "one investigation turn");
    let send = &sends[0];
    assert_eq!(send["model"], "analyst-model");
    assert_eq!(send["provider"], "analyst-provider");
    assert_eq!(send["options"]["thinking_level"], "low");
    assert_eq!(send["options"]["functions"]["allow"], json!([]));
    assert_eq!(send["options"]["max_turns"], 1);
    assert_eq!(send["options"]["max_validation_retries"], 0);
    assert_eq!(send["options"]["system_prompt_strategy"], "override");
    assert_eq!(send["options"]["output"]["type"], "json");
    let schema = send["options"]["output"]["schema"].to_string();
    assert!(
        !schema.contains("$ref") && !schema.contains("$schema"),
        "{schema}"
    );
    assert!(schema.contains("non_regression_controls"));
    assert_eq!(send["session"]["kind"], "automation");
    assert_eq!(send["session"]["metadata"]["origin"], "eval_monitor");
    assert!(!send["message"].as_str().unwrap().contains("sk-live-secret"));
    assert!(world.calls_to("harness::stop").is_empty());
}

#[tokio::test]
async fn quiet_session_completes_without_the_llm() {
    // A turn outside the 5% audit sample, with no deterministic finding.
    let turn = (0..)
        .map(|index| format!("t_quiet{index}"))
        .find(|turn| !ids::audit_sample(&ids::observation_key(ROOT, turn)))
        .unwrap();
    let mut world = World::new();
    world.judge = JudgeMode::Answer("expected_behavior", 0.95);
    world
        .status
        .insert(ROOT.into(), status(ROOT, &turn, "completed", None));
    world.entries.insert(
        ROOT.into(),
        vec![
            user("e_idem_q", "Say hi."),
            assistant_text(&format!("e_{turn}_0_assistant"), "Hi."),
        ],
    );
    world.tree = json!({"root_session_id": ROOT, "sessions": [{"session_id": ROOT, "depth": 0}],
        "complete": true});
    let h = Harness::start(world).await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, &turn).await.evaluation_id.unwrap();
    h.drain().await;
    let EvalResultResponseV1 { record, assets } = h.result(&evaluation_id).await;
    assert_eq!(record.status, EvalStatusV1::Completed);
    assert_eq!(record.counters.diagnostics, 0);
    assert!(!record.routing.unwrap().investigate);
    assert!(assets.investigation.is_none());
    assert!(h.world().calls_to("harness::send").is_empty());
}

#[tokio::test]
async fn unavailable_providers_fail_their_stage_and_keep_the_evidence() {
    let h = Harness::start({
        let mut world = World::new();
        world.judge = JudgeMode::ProviderError("provider_unavailable");
        world
    })
    .await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    h.drain().await;
    let EvalResultResponseV1 { record, assets } = h.result(&evaluation_id).await;
    assert_eq!(record.status, EvalStatusV1::Failed);
    let failure = record.failure.unwrap();
    assert_eq!(failure.stage, EvalStatusV1::Judging);
    assert_eq!(failure.code, "judge_provider_unavailable");
    assert!(
        assets.snapshot.is_some(),
        "observations survive the failure"
    );
    assert_eq!(assets.triage_failure.unwrap().code, "provider_unavailable");
    assert!(
        h.world().calls_to("harness::send").is_empty(),
        "no silent fallback"
    );

    // A malformed Jev answer is a failure too, never "expected behavior".
    h.world().judge = JudgeMode::Malformed;
    let again = runtime::analyze_session(
        &h.deps,
        serde_json::from_value(json!({"session_id": ROOT, "reanalyze": true})).unwrap(),
    )
    .await
    .unwrap();
    assert!(!again.reused);
    h.drain().await;
    let record = h.result(&again.evaluation_id).await.record;
    assert_eq!(record.failure.unwrap().code, "judge_invalid_response");

    // The chosen LLM failing is reported at the investigating stage.
    {
        let mut world = h.world();
        world.judge = JudgeMode::Answer("needs_investigation", 0.9);
        world.analyst_fails = true;
    }
    let third = runtime::analyze_session(
        &h.deps,
        serde_json::from_value(json!({"session_id": ROOT, "reanalyze": true})).unwrap(),
    )
    .await
    .unwrap();
    h.drain().await;
    let record = h.result(&third.evaluation_id).await.record;
    let analyst = record.analyst.unwrap();
    h.end_turn(&analyst.session_id, analyst.turn_id.as_deref().unwrap())
        .await;
    h.drain().await;
    let EvalResultResponseV1 { record, assets } = h.result(&third.evaluation_id).await;
    let failure = record.failure.unwrap();
    assert_eq!(
        (failure.stage, failure.code.as_str()),
        (EvalStatusV1::Investigating, "analyst_failed")
    );
    assert!(assets.triage.is_some());
    assert_eq!(
        record.usage.llm_input_tokens,
        Some(1_200),
        "a failed investigation keeps what it consumed"
    );
    assert_eq!(h.records(), 3);
}

#[tokio::test]
async fn restart_after_jev_started_is_unknown_and_not_repeated() {
    let mut h = Harness::start(World::new()).await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    // Collect only; keep the judging step queued.
    let collect = h.world().steps.pop_front().unwrap();
    runtime::step(&h.deps, serde_json::from_value(collect).unwrap())
        .await
        .unwrap();
    // The process died after persisting the Jev call but before its answer.
    {
        let mut world = h.world();
        let key = (state::ANALYSIS_SCOPE.to_string(), evaluation_id.clone());
        let record = world.state.get_mut(&key).unwrap();
        record["judge_call"] = json!({"request_id": "lost", "started_at": 1,
            "deadline": record["deadline"]});
    }
    h.restart();
    h.drain().await;
    let record = h.result(&evaluation_id).await.record;
    assert_eq!(record.failure.unwrap().code, "external_outcome_unknown");
    assert!(h.world().calls_to("judge::evaluate").is_empty());
}

#[tokio::test]
async fn cancel_stops_only_the_monitor_and_beats_late_steps() {
    let h = Harness::start(World::new()).await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    h.drain().await;
    let record = h.result(&evaluation_id).await.record;
    assert_eq!(record.status, EvalStatusV1::Investigating);
    let cancelled = runtime::cancel(
        &h.deps,
        EvaluationIdRequestV1 {
            evaluation_id: evaluation_id.clone(),
        },
    )
    .await
    .unwrap();
    assert!(cancelled.cancelled);
    let stops = h.world().calls_to("harness::stop");
    assert_eq!(stops.len(), 1);
    assert_eq!(stops[0]["session_id"], ids::analyst_session(&evaluation_id));
    // A step queued before the cancel is ignored.
    let late = runtime::step(
        &h.deps,
        StepRequestV1 {
            evaluation_id: evaluation_id.clone(),
            step: record.step,
        },
    )
    .await
    .unwrap();
    assert!(late.skipped);
    assert_eq!(
        h.result(&evaluation_id).await.record.status,
        EvalStatusV1::Cancelled
    );
    // Cancelling again is a no-op.
    let again = runtime::cancel(&h.deps, EvaluationIdRequestV1 { evaluation_id })
        .await
        .unwrap();
    assert!(!again.cancelled);
}

#[tokio::test]
async fn recovery_republishes_lost_indexes_and_deadlines_fail_pending_work() {
    let mut h = Harness::start({
        let mut world = World::new();
        world.descendants_running = true;
        world
    })
    .await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    // Crash between saving the record and publishing its index.
    let key = ids::observation_key(ROOT, TURN);
    h.world()
        .state
        .remove(&(state::OBSERVATION_SCOPE.to_string(), key.clone()));
    h.world().steps.clear();
    h.restart();
    let recovered = runtime::recover(&h.deps).await.unwrap();
    assert_eq!(recovered.reconciled, 1);
    assert_eq!(recovered.requeued, 1);
    let again = h.end_turn(ROOT, TURN).await;
    assert_eq!(again.outcome, WakeOutcomeV1::Reused);
    assert_eq!(h.records(), 1);

    // Pending past the deadline: the sweep fails it with the stage and reason.
    h.drain().await;
    {
        let mut world = h.world();
        let record = world
            .state
            .get_mut(&(state::ANALYSIS_SCOPE.to_string(), evaluation_id.clone()))
            .unwrap();
        record["deadline"] = json!(1);
    }
    let swept = runtime::sweep(&h.deps).await.unwrap();
    assert_eq!(swept.expired, 1);
    let record = h.result(&evaluation_id).await.record;
    let failure = record.failure.unwrap();
    assert_eq!(
        (failure.stage, failure.code.as_str()),
        (EvalStatusV1::Collecting, "deadline")
    );
    assert!(failure.message.contains("descendant"));
}

#[tokio::test]
async fn validation_links_reference_e2e_executions_without_a_verdict() {
    let h = Harness::start({
        let mut world = World::new();
        world.analyst_result = json!({"suggestions": [suggestion(&format!("e_{TURN}_c2"))]});
        let bundle = |id: &str, available: bool, harness: &str, behavior: &str| {
            json!({"manifest": {"executions": [{"id": id, "status": "passed",
                    "label": format!("run {id}"), "conclusion": "success"}]},
                "detail": {
                    "availability": if available { "full" } else { "unavailable" },
                    "assessment_summary": {"assessment_count": 8,
                        "assessment_outcomes": {"passed": 8, "failed": 0}},
                    "scenario_metrics": [{"scenario_id": "tool_contract_recovery",
                        "behavior_sha256": behavior, "contract_fingerprint": "fnv1a32:beaea932",
                        "run_count": 2,
                        "averages": {"function_calls": 5.0, "tokens": 6513.5,
                            "duration_seconds": 18.9, "cost_usd": 0.0036},
                        "samples": {"function_calls": 2, "tokens": 2,
                            "duration_seconds": 2, "cost_usd": 1}}],
                    "reports": [{"subject_id": "deepseek-flash", "scenario_id": "tool_contract_recovery",
                        "available": available, "report": {
                            "subject": {"model": "deepseek-flash", "provider": "deepseek"},
                            "system_under_test": {"harness_version": harness,
                                "engine_version": "0.24.4-rc.1", "e2e_revision": "f42b900"}}}]}})
        };
        world.executions.insert(
            "exec-base".into(),
            bundle("exec-base", true, "1.8.42", "sha256:aa"),
        );
        world.executions.insert(
            "exec-cand".into(),
            bundle("exec-cand", false, "1.8.43", "sha256:aa"),
        );
        world.executions.insert(
            "exec-other".into(),
            bundle("exec-other", true, "1.8.43", "sha256:bb"),
        );
        world
    })
    .await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    h.drain().await;
    let analyst = h.result(&evaluation_id).await.record.analyst.unwrap();
    h.end_turn(&analyst.session_id, analyst.turn_id.as_deref().unwrap())
        .await;
    h.drain().await;
    assert_eq!(
        h.result(&evaluation_id).await.record.counters.suggestions,
        1
    );

    let attach = |base: &str, cand: &str, index: usize| {
        serde_json::from_value::<AttachValidationRequestV1>(json!({
            "evaluation_id": evaluation_id, "suggestion_index": index,
            "baseline_execution_id": base, "candidate_execution_id": cand}))
        .unwrap()
    };
    assert!(
        runtime::attach_validation(&h.deps, attach("exec-base", "exec-base", 0))
            .await
            .is_err()
    );
    assert!(
        runtime::attach_validation(&h.deps, attach("exec-base", "exec-cand", 3))
            .await
            .is_err()
    );
    assert!(
        runtime::attach_validation(&h.deps, attach("exec-base", "missing", 0))
            .await
            .is_err()
    );
    // A dry run looks both runs up without saving anything.
    let mut preview = attach("exec-base", "exec-other", 0);
    preview.dry_run = true;
    let preview = runtime::attach_validation(&h.deps, preview).await.unwrap();
    assert!(!preview.saved);
    assert!(!preview.link.comparability.comparable);
    let mismatch = preview
        .link
        .comparability
        .checks
        .iter()
        .find(|check| !check.matches)
        .unwrap();
    assert_eq!(mismatch.field, "behavior_sha256");
    assert_eq!(
        h.result(&evaluation_id).await.record.counters.validations,
        0
    );

    let attached = runtime::attach_validation(&h.deps, attach("exec-base", "exec-cand", 0))
        .await
        .unwrap();
    assert!(attached.saved);
    let link = attached.link;
    assert!(link.comparability.comparable, "{:?}", link.comparability);
    assert_eq!(link.baseline.harness_version.as_deref(), Some("1.8.42"));
    assert_eq!(link.candidate.harness_version.as_deref(), Some("1.8.43"));
    assert_eq!(link.baseline.assessments.as_ref().unwrap().passed, 8);
    let cost = &link.baseline.scenarios[0].measures["cost_usd"];
    assert_eq!((cost.samples, link.baseline.scenarios[0].run_count), (1, 2));
    assert!(link.baseline.reports_available);
    assert!(
        !link.candidate.reports_available,
        "unavailable assets stay visible"
    );
    let EvalResultResponseV1 { record, assets } = h.result(&evaluation_id).await;
    assert_eq!(record.counters.validations, 1);
    assert_eq!(assets.validations, [link]);
}

#[tokio::test]
async fn manual_analysis_rules() {
    let h = Harness::start(World::new()).await;
    let analyze = |session_id: &str, reanalyze: bool| {
        serde_json::from_value::<AnalyzeSessionRequestV1>(
            json!({"session_id": session_id, "reanalyze": reanalyze}),
        )
        .unwrap()
    };
    // Needs a configured model, works while paused.
    assert!(runtime::analyze_session(&h.deps, analyze(ROOT, false))
        .await
        .is_err());
    h.configure(false).await;
    let first = runtime::analyze_session(&h.deps, analyze(ROOT, false))
        .await
        .unwrap();
    assert!(!first.reused);
    let same = runtime::analyze_session(&h.deps, analyze(ROOT, false))
        .await
        .unwrap();
    assert!(same.reused);
    assert_eq!(same.evaluation_id, first.evaluation_id);
    // Reanalysis waits for the previous analysis to end.
    assert!(runtime::analyze_session(&h.deps, analyze(ROOT, true))
        .await
        .is_err());
    // The monitor's own sessions are never analyzed.
    assert!(
        runtime::analyze_session(&h.deps, analyze(&ids::analyst_session("eval_x"), false))
            .await
            .is_err()
    );
    // A still-running source is refused.
    h.world()
        .status
        .insert("s_busy".into(), status("s_busy", "t_busy", "running", None));
    assert!(runtime::analyze_session(&h.deps, analyze("s_busy", false))
        .await
        .is_err());
    // A deleted analysis keeps blocking automatic re-admission.
    h.drain().await;
    let record = h.result(&first.evaluation_id).await.record;
    assert_eq!(
        record.status,
        EvalStatusV1::Investigating,
        "manual requests investigate"
    );
    runtime::cancel(
        &h.deps,
        EvaluationIdRequestV1 {
            evaluation_id: first.evaluation_id.clone(),
        },
    )
    .await
    .unwrap();
    runtime::delete(
        &h.deps,
        EvaluationIdRequestV1 {
            evaluation_id: first.evaluation_id.clone(),
        },
    )
    .await
    .unwrap();
    h.configure(true).await;
    assert_eq!(h.end_turn(ROOT, TURN).await.outcome, WakeOutcomeV1::Reused);
    assert_eq!(h.records(), 0);
}

#[tokio::test]
async fn cancel_interrupts_an_in_flight_jev_call_and_keeps_its_late_usage() {
    let h = Harness::start({
        let mut world = World::new();
        world.judge = JudgeMode::HoldUntilCancel;
        world
    })
    .await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    let collect = h.world().steps.pop_front().unwrap();
    runtime::step(&h.deps, serde_json::from_value(collect).unwrap())
        .await
        .unwrap();
    let judging = h.world().steps.pop_front().unwrap();
    let deps = h.deps.clone();
    let step = tokio::spawn(async move {
        runtime::step(&deps, serde_json::from_value(judging).unwrap()).await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while h.world().held_judge.is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("Jev call in flight");

    // The step holds no lock while Jev runs, so cancel answers at once.
    let cancelled = tokio::time::timeout(
        Duration::from_secs(2),
        runtime::cancel(
            &h.deps,
            EvaluationIdRequestV1 {
                evaluation_id: evaluation_id.clone(),
            },
        ),
    )
    .await
    .expect("cancel is not blocked by the in-flight call")
    .unwrap();
    assert!(cancelled.cancelled);
    let cancels = h.world().calls_to("judge::cancel");
    assert_eq!(cancels.len(), 1);
    assert_eq!(cancels[0]["provider"], "typesafe");

    // The interrupted call returns; its usage is kept, the cancel stands.
    let late = tokio::time::timeout(Duration::from_secs(5), step)
        .await
        .expect("the interrupted step returns")
        .unwrap()
        .unwrap();
    assert!(late.skipped);
    let EvalResultResponseV1 { record, assets } = h.result(&evaluation_id).await;
    assert_eq!(record.status, EvalStatusV1::Cancelled);
    assert!(record.failure.is_none());
    let failure = assets.triage_failure.unwrap();
    assert_eq!(failure.code, "cancelled");
    assert_eq!(failure.stats.unwrap().elapsed_ms, 900);
    assert!(h.world().calls_to("harness::send").is_empty());
}

#[tokio::test]
async fn window_covers_wake_continuations_but_not_history_from_before_the_monitor() {
    let h = Harness::start(World::new()).await;
    h.configure(true).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    {
        let mut world = h.world();
        // An old turn with a rediscovery, written before the monitor existed.
        let mut entries: Vec<Value> = rediscovery_transcript()
            .into_iter()
            .map(|mut entry| {
                if let Some(timestamp) = entry["message"].get_mut("timestamp") {
                    *timestamp = json!(1);
                }
                entry
            })
            .collect();
        // A wake continuation: a non-terminal turn, then the definitive one.
        entries.push(user("e_idem_next", "Wait for the workers, then report."));
        entries.push(assistant_text("e_t_wake_0_assistant", "Waiting."));
        entries.push(assistant_text("e_t_final_0_assistant", "All done."));
        world.entries.insert(ROOT.into(), entries);
        world
            .status
            .insert(ROOT.into(), status(ROOT, "t_final", "completed", None));
        world.tree = json!({"root_session_id": ROOT,
            "sessions": [{"session_id": ROOT, "depth": 0}], "complete": true});
    }
    let evaluation_id = h.end_turn(ROOT, "t_final").await.evaluation_id.unwrap();
    h.drain().await;
    let EvalResultResponseV1 { record, assets } = h.result(&evaluation_id).await;
    let snapshot = assets.snapshot.unwrap();
    assert_eq!(snapshot.window_turn_ids, ["t_final", "t_wake"]);
    assert!(
        snapshot.diagnostics.is_empty(),
        "history from before the monitor is not reported as new"
    );
    assert_eq!(record.counters.diagnostics, 0);
}

#[tokio::test]
async fn configuration_keeps_the_observation_start_while_observing() {
    let h = Harness::start(World::new()).await;
    let first = h.configure(true).await;
    let since = first.enabled_since.unwrap();
    tokio::time::sleep(Duration::from_millis(3)).await;
    let edited = h.configure(true).await;
    assert_eq!(edited.enabled_since, Some(since), "an edit keeps the start");
    assert!(h.configure(false).await.enabled_since.is_none());
    tokio::time::sleep(Duration::from_millis(3)).await;
    assert!(h.configure(true).await.enabled_since.unwrap() > since);
}

#[tokio::test]
async fn a_turn_superseded_before_capture_joins_the_next_window() {
    let h = Harness::start(World::new()).await;
    h.configure(true).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    {
        // The rediscovery happens in TURN; the user already sent the next
        // message, so the session is on t_next when TURN is collected.
        let mut world = h.world();
        let mut entries = rediscovery_transcript();
        entries.push(user("e_idem_next", "And now the receipt."));
        entries.push(assistant_text("e_t_next_0_assistant", "R-1."));
        world.entries.insert(ROOT.into(), entries);
        world
            .status
            .insert(ROOT.into(), status(ROOT, "t_next", "completed", None));
    }
    let first = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    h.drain().await;
    let failure = h.result(&first).await.record.failure.unwrap();
    assert_eq!(failure.code, "source_advanced");

    let next = h.end_turn(ROOT, "t_next").await.evaluation_id.unwrap();
    h.drain().await;
    let snapshot = h.result(&next).await.assets.snapshot.unwrap();
    assert_eq!(snapshot.window_turn_ids, ["t_next", TURN]);
    assert_eq!(
        snapshot.diagnostics.len(),
        1,
        "the superseded turn is analyzed"
    );
}

#[tokio::test]
async fn capacity_rejections_and_provider_checks_are_reported() {
    let h = Harness::start(World::new()).await;
    h.configure(true).await;
    let admitted = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    {
        // Fill the monitor up to its cap of unfinished analyses.
        let mut world = h.world();
        let record = world
            .state
            .get(&(state::ANALYSIS_SCOPE.to_string(), admitted))
            .unwrap()
            .clone();
        for index in 0..runtime::limits().max_active_analyses {
            let mut copy = record.clone();
            copy["evaluation_id"] = json!(format!("eval_filler_{index}"));
            world.state.insert(
                (
                    state::ANALYSIS_SCOPE.to_string(),
                    format!("eval_filler_{index}"),
                ),
                copy,
            );
        }
    }
    let rejected = h.end_turn(ROOT, "t_more").await;
    assert_eq!(rejected.outcome, WakeOutcomeV1::AtCapacity);
    let monitor = runtime::monitor_state(
        &h.deps,
        MonitorStateRequestV1 {
            check_providers: true,
        },
    )
    .await
    .unwrap();
    let rejection = monitor.last_rejection.unwrap();
    assert_eq!(
        (rejection.session_id.as_str(), rejection.turn_id.as_str()),
        (ROOT, "t_more")
    );
    let triage = monitor.triage.unwrap();
    assert!(triage.available);
    assert_eq!(triage.models, ["jev-test-1"]);
    assert!(!h.world().calls_to("judge::models::list").is_empty());
}

/// One `executions-list` entry as the E2E returns it: an array stack with the
/// Harness build, or the object-shaped stack of a source run.
fn e2e_run(
    id: &str,
    label: &str,
    status: &str,
    started_at: &str,
    harness: Option<(&str, &str)>,
    scenarios: &[&str],
) -> Value {
    let stack = match harness {
        Some((version, commit)) => json!([
            {"name": "ade", "observed": "1.9.48", "commit": "d0b6c00", "dirty": false},
            {"name": "harness", "observed": version, "commit": commit, "dirty": false,
             "source": "path"}]),
        None => json!({"lock_digest": null, "mode": "source", "versions": null}),
    };
    json!({"id": id, "label": label, "status": status, "started_at": started_at,
        "completed_at": started_at, "stack": stack,
        "parameters": {"model": "deepseek-flash", "provider": "deepseek",
                       "scenarios": scenarios}})
}

const SCENARIO: &str = "tool_contract_recovery";

/// Six executions: three results of the plan's scenario (`exec-twin` failed
/// its task, a valid result, and records the same stack as `exec-cand`), plus
/// one incomplete, one technically failed and one on another scenario.
fn e2e_executions() -> Value {
    json!({"executions": [
        e2e_run("exec-tech", "crashed run", "technical_failed", "2026-10-02T12:00:00Z",
            Some(("1.8.43", "00c21f5")), &[SCENARIO]),
        e2e_run("exec-cand", "after the change", "passed", "2026-10-02T10:00:00.123456789+00:00",
            Some(("1.8.43", "00c21f5")), &[SCENARIO]),
        e2e_run("exec-twin", "repeat of the change", "failed", "2026-10-02T11:00:00Z",
            Some(("1.8.43", "00c21f5")), &[SCENARIO]),
        e2e_run("exec-base", "before the change", "passed", "2026-10-01T10:00:00Z",
            Some(("1.8.42", "f3a49e1")), &[SCENARIO]),
        e2e_run("exec-wip", "still going", "incomplete", "2026-10-02T13:00:00Z", None, &[SCENARIO]),
        e2e_run("exec-timer", "timer only", "passed", "2026-10-02T09:00:00Z",
            Some(("1.8.42", "f3a49e1")), &["timer_wake"]),
    ], "total": 6})
}

/// An analysis that finished with one suggestion whose plan names `SCENARIO`.
async fn analyzed_with_suggestion(world: World) -> (Harness, String) {
    analyzed_with_scenario(world, Some(SCENARIO)).await
}

/// The same, with the plan's scenario given (`None`: a new case is needed).
async fn analyzed_with_scenario(mut world: World, scenario: Option<&str>) -> (Harness, String) {
    let mut planned = suggestion(&format!("e_{TURN}_c2"));
    planned["validation"]["scenario_id"] = json!(scenario);
    world.analyst_result = json!({"suggestions": [planned]});
    let h = Harness::start(world).await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    h.drain().await;
    let analyst = h.result(&evaluation_id).await.record.analyst.unwrap();
    h.end_turn(&analyst.session_id, analyst.turn_id.as_deref().unwrap())
        .await;
    h.drain().await;
    let record = h.result(&evaluation_id).await.record;
    assert_eq!(record.counters.suggestions, 1, "{:?}", record.failure);
    (h, evaluation_id)
}

async fn propose(
    h: &Harness,
    evaluation_id: &str,
    suggestion_index: usize,
) -> Result<ProposeValidationResponseV1, EvalError> {
    runtime::propose_validation(
        &h.deps,
        serde_json::from_value(
            json!({"evaluation_id": evaluation_id, "suggestion_index": suggestion_index}),
        )
        .unwrap(),
    )
    .await
}

#[tokio::test]
async fn jev_proposes_a_comparable_pair_without_attaching_it() {
    let mut world = World::new();
    world.executions_list = Some(e2e_executions());
    let (h, evaluation_id) = analyzed_with_suggestion(world).await;
    let before = h.result(&evaluation_id).await.record.usage;
    assert_eq!((before.judge_calls, before.judge_input_tokens), (1, 410));
    h.world().judge = JudgeMode::Pick("R1_R2", 0.93);

    let response = propose(&h, &evaluation_id, 0).await.unwrap();
    assert_eq!(response.outcome, ProposalOutcomeV1::Proposed);
    assert_eq!(
        response.proposal,
        Some(ValidationProposalV1 {
            baseline_execution_id: "exec-base".into(),
            candidate_execution_id: "exec-cand".into(),
            confidence: 0.93,
            low_confidence: false,
            stack_note: "recorded stack differs: harness 1.8.42·f3a49e1 → 1.8.43·00c21f5".into(),
        })
    );
    assert_eq!(
        (
            response.runs_listed,
            response.runs_considered,
            response.pairs_considered
        ),
        (6, 3, 6),
        "base, cand and twin make six ordered pairs; the twin's identical stack drops none"
    );
    assert_eq!(response.pairs_dropped, 0);
    assert_eq!(
        response.excluded,
        BTreeMap::from([
            ("technical_failed".to_string(), 1),
            ("incomplete".to_string(), 1),
            ("other_scenario".to_string(), 1),
        ])
    );
    let jev = response.jev.as_ref().unwrap();
    assert_eq!(jev.model, "jev-test-1");
    assert_eq!(jev.stats.input_tokens, 520);
    let shown = serde_json::to_value(&response).unwrap();
    assert_eq!(shown["proposal"]["baseline_execution_id"], "exec-base");
    assert_eq!(shown["outcome"], "proposed");

    {
        let world = h.world();
        assert_eq!(
            world.calls_to("e2e::dashboard::executions-list"),
            [json!({"limit": 100})]
        );
        let judge_calls = world.calls_to("judge::evaluate");
        assert_eq!(judge_calls.len(), 2, "the triage call and this one");
        let payload = &judge_calls[1];
        assert_eq!(payload["provider"], "typesafe");
        assert_eq!(payload["timeout_ms"], 60_000);
        assert!(payload["expires_at_unix_ms"].is_u64());
        assert!(payload["model"].is_null());
        assert_eq!(payload["request_id"], jev.request_id);
        assert!(jev
            .request_id
            .starts_with(&format!("{evaluation_id}-propose-")));
        let evaluation = &payload["evaluations"][0];
        assert_eq!(evaluation["id"], "pairing");
        let question = &evaluation["questions"]["pair"];
        assert_eq!(question["type"], "choice");
        let keys: Vec<_> = question["criteria"].as_object().unwrap().keys().collect();
        assert_eq!(
            keys,
            ["R1_R2", "R1_R3", "R2_R1", "R2_R3", "R3_R1", "R3_R2", "none"]
        );
        // Each criterion carries both labels and the recorded stack difference.
        assert_eq!(
            question["criteria"]["R1_R2"],
            "Baseline R1 (before the change) runs the Harness without this change and candidate \
             R2 (after the change) runs it with the change; recorded stack differs: harness \
             1.8.42·f3a49e1 → 1.8.43·00c21f5"
        );
        assert_eq!(
            question["criteria"]["R2_R3"],
            "Baseline R2 (after the change) runs the Harness without this change and candidate \
             R3 (repeat of the change) runs it with the change; recorded stacks identical"
        );
        // Runs are numbered oldest first; Jev sees builds and times, not ids.
        let state = &evaluation["state"];
        assert_eq!(state["plan"]["scenario_id"], SCENARIO);
        let runs = state["runs"].as_object().unwrap();
        assert_eq!(runs.len(), 3);
        assert_eq!(runs["R1"]["label"], "before the change");
        assert_eq!(runs["R1"]["harness"], "1.8.42·f3a49e1");
        assert_eq!(runs["R1"]["status"], "passed");
        assert_eq!(runs["R1"]["started_at"], "2026-10-01T10:00:00Z");
        assert_eq!(
            runs["R2"]["started_at"],
            "2026-10-02T10:00:00.123456789+00:00"
        );
        assert_eq!(runs["R3"]["status"], "failed");
        assert!(!state.to_string().contains("exec-"), "no execution ids");
        assert!(world.calls_to("e2e::dashboard::execution-get").is_empty());
    }

    // Read-only apart from the spend: the Jev call is on the record, and
    // nothing is attached.
    let EvalResultResponseV1 { record, assets } = h.result(&evaluation_id).await;
    assert_eq!(record.usage.judge_calls, 2);
    assert_eq!(record.usage.judge_input_tokens, 410 + 520);
    assert_eq!(record.usage.judge_output_tokens, 3 + 4);
    assert!(record.usage.judge_usage_complete);
    assert!(record.updated_at > before_updated(&before, &record));
    assert!(assets.validations.is_empty());
    assert_eq!(record.counters.validations, 0);

    // Low confidence is flagged, and each click is its own Jev request.
    h.world().judge = JudgeMode::Pick("R2_R1", 0.55);
    let second = propose(&h, &evaluation_id, 0).await.unwrap();
    let proposal = second.proposal.unwrap();
    assert_eq!(
        (
            proposal.baseline_execution_id.as_str(),
            proposal.candidate_execution_id.as_str()
        ),
        ("exec-cand", "exec-base")
    );
    assert!(proposal.low_confidence);
    assert_ne!(second.jev.unwrap().request_id, jev.request_id);
    assert_eq!(h.result(&evaluation_id).await.record.usage.judge_calls, 3);
}

/// `updated_at` only has to move; the usage struct is unrelated, so compare
/// against the time the analysis completed.
fn before_updated(_usage: &MonitorUsageV1, record: &AnalysisRecordV1) -> i64 {
    record.completed_at.unwrap()
}

#[tokio::test]
async fn jev_may_find_no_pair_that_fits() {
    let mut world = World::new();
    world.executions_list = Some(e2e_executions());
    let (h, evaluation_id) = analyzed_with_suggestion(world).await;
    h.world().judge = JudgeMode::Pick("none", 0.88);
    let response = propose(&h, &evaluation_id, 0).await.unwrap();
    assert_eq!(response.outcome, ProposalOutcomeV1::NoneFits);
    assert!(response.proposal.is_none());
    assert_eq!(response.pairs_considered, 6);
    assert_eq!(response.jev.as_ref().unwrap().stats.input_tokens, 520);
    assert!(serde_json::to_value(&response)
        .unwrap()
        .get("proposal")
        .is_none());
    assert_eq!(
        h.result(&evaluation_id).await.record.usage.judge_calls,
        2,
        "the answer was paid for"
    );
}

#[tokio::test]
async fn without_a_comparable_pair_jev_is_not_asked() {
    let mut world = World::new();
    // Three results that never share model and provider, plus runs that
    // cannot be a result.
    let run = |id: &str, model: &str, provider: &str, started: &str| {
        let mut run = e2e_run(
            id,
            id,
            "passed",
            started,
            Some(("1.8.43", "00c21f5")),
            &[SCENARIO],
        );
        run["parameters"]["model"] = json!(model);
        run["parameters"]["provider"] = json!(provider);
        run
    };
    world.executions_list = Some(json!({"executions": [
        run("exec-a", "deepseek-flash", "deepseek", "2026-10-02T10:00:00Z"),
        run("exec-opus", "claude-opus-5-5", "anthropic", "2026-10-02T11:00:00Z"),
        run("exec-proxied", "deepseek-flash", "openai", "2026-10-02T12:00:00Z"),
        e2e_run("exec-wip", "wip", "incomplete", "2026-10-02T13:00:00Z", None, &[SCENARIO]),
        e2e_run("exec-infra", "infra", "infra_failed", "2026-10-02T13:30:00Z", None, &[SCENARIO]),
        e2e_run("exec-cancelled", "cancelled", "cancelled", "2026-10-02T13:40:00Z", None, &[SCENARIO]),
    ]}));
    let (h, evaluation_id) = analyzed_with_suggestion(world).await;
    let usage = h.result(&evaluation_id).await.record.usage;

    let response = propose(&h, &evaluation_id, 0).await.unwrap();
    assert_eq!(response.outcome, ProposalOutcomeV1::NoComparablePair);
    assert!(response.proposal.is_none() && response.jev.is_none());
    assert_eq!(
        (
            response.runs_listed,
            response.runs_considered,
            response.pairs_considered,
            response.pairs_dropped
        ),
        (6, 3, 0, 0)
    );
    assert_eq!(
        response.excluded,
        BTreeMap::from([
            ("incomplete".to_string(), 1),
            ("infra_failed".to_string(), 1),
            ("cancelled".to_string(), 1),
        ]),
        "unusable runs are counted by status, not hidden"
    );
    assert_eq!(
        h.world().calls_to("judge::evaluate").len(),
        1,
        "only the triage call: no pair, no Jev call"
    );
    assert_eq!(h.result(&evaluation_id).await.record.usage, usage);
}

/// Genuine A/B pairs often record identical stacks (the change lived in a
/// build the stack record does not capture), and plans that name no
/// scenario ask for the same scenario set, in any order.
#[tokio::test]
async fn without_a_plan_scenario_pairs_need_the_same_suite_and_identical_stacks_stay() {
    let mut world = World::new();
    let suite = [SCENARIO, "timer_wake"];
    world.executions_list = Some(json!({"executions": [
        e2e_run("exec-a", "A: SEM #1292", "passed", "2026-10-02T10:00:00Z",
            Some(("1.8.42", "f3a49e1")), &suite),
        e2e_run("exec-b", "B: COM #1292", "passed", "2026-10-02T11:00:00Z",
            Some(("1.8.42", "f3a49e1")), &[suite[1], suite[0]]),
        e2e_run("exec-source", "older source run", "passed", "2026-10-01T09:00:00Z",
            None, &suite),
        e2e_run("exec-single", "single case", "passed", "2026-10-02T12:00:00Z",
            Some(("1.8.42", "f3a49e1")), &[SCENARIO]),
    ]}));
    let (h, evaluation_id) = analyzed_with_scenario(world, None).await;
    h.world().judge = JudgeMode::Pick("R2_R3", 0.9);

    let response = propose(&h, &evaluation_id, 0).await.unwrap();
    assert_eq!(response.outcome, ProposalOutcomeV1::Proposed);
    let proposal = response.proposal.unwrap();
    assert_eq!(
        (
            proposal.baseline_execution_id.as_str(),
            proposal.candidate_execution_id.as_str()
        ),
        ("exec-a", "exec-b")
    );
    assert_eq!(
        (response.runs_considered, response.pairs_considered),
        (4, 6),
        "the single-case run joins no pair"
    );
    assert!(response.excluded.is_empty());
    let calls = h.world().calls_to("judge::evaluate");
    let evaluation = &calls[1]["evaluations"][0];
    assert_eq!(evaluation["state"]["plan"]["scenario_id"], Value::Null);
    let runs = evaluation["state"]["runs"].as_object().unwrap();
    assert_eq!(runs.len(), 3, "only the runs a kept pair refers to");
    assert!(!evaluation["state"].to_string().contains("single case"));
    assert_eq!(
        runs["R1"]["harness"], "unknown",
        "an object-shaped stack is unknown"
    );
    let criteria = &evaluation["questions"]["pair"]["criteria"];
    assert_eq!(
        criteria["R2_R3"],
        "Baseline R2 (A: SEM #1292) runs the Harness without this change and candidate R3 \
         (B: COM #1292) runs it with the change; recorded stacks identical"
    );
    assert_eq!(
        criteria["R1_R3"],
        "Baseline R1 (older source run) runs the Harness without this change and candidate R3 \
         (B: COM #1292) runs it with the change; recorded stack unknown"
    );
}

/// 12 results of the case make 132 ordered pairs; the 60 most recent (by the
/// older run of each pair) are offered and the rest is reported.
#[tokio::test]
async fn pairs_are_capped_and_the_dropped_ones_are_reported() {
    let mut world = World::new();
    world.executions_list = Some(json!({"executions": (0..12)
        .map(|index| e2e_run(&format!("exec-{index:02}"), &format!("run {index}"), "passed",
            &format!("2026-10-02T10:{index:02}:00Z"), Some(("1.8.43", "00c21f5")), &[SCENARIO]))
        .collect::<Vec<_>>()}));
    let (h, evaluation_id) = analyzed_with_suggestion(world).await;
    h.world().judge = JudgeMode::Pick("none", 0.9);

    let response = propose(&h, &evaluation_id, 0).await.unwrap();
    assert_eq!(response.outcome, ProposalOutcomeV1::NoneFits);
    assert_eq!(
        (
            response.runs_considered,
            response.pairs_considered,
            response.pairs_dropped
        ),
        (12, 60, 72)
    );
    let calls = h.world().calls_to("judge::evaluate");
    let evaluation = &calls[1]["evaluations"][0];
    let criteria = evaluation["questions"]["pair"]["criteria"]
        .as_object()
        .unwrap();
    assert_eq!(criteria.len(), 61, "60 pairs and none");
    let runs = evaluation["state"]["runs"].as_object().unwrap();
    assert_eq!(
        runs.len(),
        9,
        "the three oldest runs belong to no kept pair"
    );
    assert!(!evaluation["state"].to_string().contains("run 2"));
}

#[tokio::test]
async fn jev_failures_surface_with_the_providers_explanation_and_keep_their_usage() {
    let mut world = World::new();
    world.executions_list = Some(e2e_executions());
    let (h, evaluation_id) = analyzed_with_suggestion(world).await;
    let message = |result: Result<ProposeValidationResponseV1, EvalError>| match result {
        Err(EvalError::Dependency(message)) => message,
        other => panic!("expected a dependency error, got {other:?}"),
    };

    h.world().judge = JudgeMode::Billing;
    let billing = message(propose(&h, &evaluation_id, 0).await);
    assert!(billing.starts_with("jev_unavailable: "), "{billing}");
    assert!(billing.contains("HTTP 402"), "{billing}");
    assert!(
        billing.contains("Your organization has no available TypeSafe API credits."),
        "{billing}"
    );
    let usage = h.result(&evaluation_id).await.record.usage;
    assert_eq!(usage.judge_calls, 2, "an error that answered still counts");
    assert_eq!(usage.judge_input_tokens, 410);
    assert!(
        !usage.judge_usage_complete,
        "the failure's usage is incomplete"
    );

    // A reply the contract cannot parse has no stats: counted, usage unknown.
    h.world().judge = JudgeMode::Malformed;
    let malformed = message(propose(&h, &evaluation_id, 0).await);
    assert!(
        malformed.starts_with("jev_invalid_response: "),
        "{malformed}"
    );
    assert_eq!(h.result(&evaluation_id).await.record.usage.judge_calls, 3);

    // An answer outside the offered options is rejected, never trusted.
    h.world().judge = JudgeMode::Pick("R9_R9", 0.9);
    let outside = message(propose(&h, &evaluation_id, 0).await);
    assert!(outside.starts_with("jev_invalid_response: "), "{outside}");
    let usage = h.result(&evaluation_id).await.record.usage;
    assert_eq!(usage.judge_calls, 4);
    assert_eq!(usage.judge_input_tokens, 410 + 520, "its stats are kept");
    assert!(
        !usage.judge_usage_complete,
        "one incomplete call keeps it incomplete"
    );
}

#[tokio::test]
async fn the_analysis_lock_is_free_while_jev_answers_and_a_cancelled_call_is_counted() {
    let mut world = World::new();
    world.executions_list = Some(e2e_executions());
    let (h, evaluation_id) = analyzed_with_suggestion(world).await;
    h.world().judge = JudgeMode::HoldUntilCancel;
    let deps = h.deps.clone();
    let id = evaluation_id.clone();
    let proposing = tokio::spawn(async move {
        runtime::propose_validation(
            &deps,
            serde_json::from_value(json!({"evaluation_id": id, "suggestion_index": 0})).unwrap(),
        )
        .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while h.world().held_judge.is_none() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("Jev call in flight");

    // An action that needs the analysis lock is not blocked by the held call.
    let attach = runtime::attach_validation(
        &h.deps,
        serde_json::from_value(
            json!({"evaluation_id": evaluation_id, "suggestion_index": 0,
            "baseline_execution_id": "exec-x", "candidate_execution_id": "exec-y"}),
        )
        .unwrap(),
    );
    let attached = tokio::time::timeout(Duration::from_secs(2), attach)
        .await
        .expect("the lock is free while Jev answers");
    assert!(attached.is_err(), "these executions do not exist");

    // The provider answers `cancelled` with its stats: still real spend.
    h.iii
        .trigger(TriggerRequest {
            function_id: "judge::cancel".into(),
            payload: json!({"request_id": "any"}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(5), proposing)
        .await
        .expect("the proposal returns")
        .unwrap();
    match outcome {
        Err(EvalError::Dependency(message)) => {
            assert!(message.starts_with("jev_unavailable: "), "{message}");
            assert!(message.contains("cancelled"), "{message}");
        }
        other => panic!("expected jev_unavailable, got {other:?}"),
    }
    let usage = h.result(&evaluation_id).await.record.usage;
    assert_eq!(usage.judge_calls, 2);
    assert!(!usage.judge_usage_complete);
}

#[tokio::test]
async fn an_unavailable_e2e_is_reported_before_jev_is_asked() {
    let (h, evaluation_id) = analyzed_with_suggestion(World::new()).await;
    let usage = h.result(&evaluation_id).await.record.usage;
    match propose(&h, &evaluation_id, 0).await {
        Err(EvalError::Dependency(message)) => {
            assert!(message.starts_with("e2e_unavailable: "), "{message}");
            assert!(message.contains("executions-list"), "{message}");
        }
        other => panic!("expected e2e_unavailable, got {other:?}"),
    }
    assert_eq!(h.world().calls_to("judge::evaluate").len(), 1);
    assert_eq!(h.result(&evaluation_id).await.record.usage, usage);

    // A reply that is not an execution list is the same failure.
    h.world().executions_list = Some(json!({"error": "starting"}));
    assert!(matches!(
        propose(&h, &evaluation_id, 0).await,
        Err(EvalError::Dependency(message)) if message.starts_with("e2e_unavailable: ")
    ));
}

#[tokio::test]
async fn proposals_follow_the_attach_preconditions() {
    let mut world = World::new();
    world.executions_list = Some(e2e_executions());
    let (h, evaluation_id) = analyzed_with_suggestion(world).await;
    assert!(matches!(
        propose(&h, "eval_missing", 0).await,
        Err(EvalError::NotFound(_))
    ));
    assert!(matches!(
        propose(&h, &evaluation_id, 1).await,
        Err(EvalError::InvalidRequest(message)) if message.contains("does not exist")
    ));
    // An analysis that has not finished has no suggestions to validate.
    let pending = h.end_turn(ROOT, "t_next").await.evaluation_id.unwrap();
    assert!(matches!(
        propose(&h, &pending, 0).await,
        Err(EvalError::Conflict(_))
    ));
    assert!(h
        .world()
        .calls_to("e2e::dashboard::executions-list")
        .is_empty());
}

/// A directory that exists on this host and holds real files: this crate.
const CODE_DIR: &str = env!("CARGO_MANIFEST_DIR");

/// A suggestion on the notice evidence that cites these code lines.
fn citing(refs: Vec<Value>) -> Value {
    let mut cited = suggestion(&format!("e_{TURN}_1_notice_0"));
    cited["code_refs"] = Value::Array(refs);
    cited
}

fn lines(path: &str, line_from: u32, line_to: u32) -> Value {
    json!({"path": path, "line_from": line_from, "line_to": line_to})
}

/// Plays an admitted analysis through its investigation and returns it.
async fn investigated(h: &Harness, evaluation_id: &str) -> EvalResultResponseV1 {
    h.drain().await;
    let analyst = h.result(evaluation_id).await.record.analyst.unwrap();
    h.end_turn(&analyst.session_id, analyst.turn_id.as_deref().unwrap())
        .await;
    h.drain().await;
    h.result(evaluation_id).await
}

async fn reanalyze(h: &Harness) -> String {
    runtime::analyze_session(
        &h.deps,
        serde_json::from_value(json!({"session_id": ROOT, "reanalyze": true})).unwrap(),
    )
    .await
    .unwrap()
    .evaluation_id
}

#[tokio::test]
async fn the_code_directory_is_validated_when_it_is_configured() {
    let h = Harness::start(World::new()).await;
    let plain = h.configure(true).await;
    assert_eq!(plain.code_repository, None);
    assert!(serde_json::to_value(&plain)
        .unwrap()
        .get("code_repository")
        .is_none());

    for (path, expected) in [
        ("workers", "absolute path"),
        ("/nonexistent/workers", "not readable"),
        (&format!("{CODE_DIR}/Cargo.toml"), "not a directory"),
    ] {
        let error = h.configure_code(Some(path)).await.unwrap_err();
        assert!(matches!(error, EvalError::InvalidRequest(_)), "{error:?}");
        assert!(error.to_string().contains(expected), "{error}");
    }
    let stored = state::get_config(&h.deps.iii).await.unwrap().unwrap();
    assert_eq!(
        stored.revision, plain.revision,
        "refused requests save nothing"
    );

    let with_code = h.configure_code(Some(CODE_DIR)).await.unwrap();
    assert_eq!(with_code.code_repository.as_deref(), Some(CODE_DIR));
    assert_ne!(with_code.revision, plain.revision);
    let stored = state::get_config(&h.deps.iii).await.unwrap().unwrap();
    assert_eq!(stored, with_code);

    // A blank path is no path; configurations without code keep their revision.
    let blank = h.configure_code(Some("  ")).await.unwrap();
    assert_eq!(blank.code_repository, None);
    assert_eq!(blank.revision, plain.revision);

    // Pausing or resuming with the same directory never needs it to exist
    // still; choosing another one does.
    let vanishing =
        std::env::temp_dir().join(format!("eval-flow-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&vanishing).unwrap();
    let vanishing_path = vanishing.to_string_lossy().into_owned();
    h.configure_code(Some(&vanishing_path)).await.unwrap();
    std::fs::remove_dir_all(&vanishing).unwrap();
    assert!(h.configure_code(Some(&vanishing_path)).await.is_ok());
    assert!(h
        .configure_code(Some(&format!("{vanishing_path}/other")))
        .await
        .is_err());
}

#[tokio::test]
async fn with_a_code_directory_the_investigation_runs_in_it_and_the_directory_is_frozen() {
    let mut world = World::new();
    world.analyst_result = json!({"suggestions": [
        citing(vec![lines("src/ids.rs", 1, 5), lines("./src/runtime.rs", 10, 20)]),
        citing(vec![lines("src/not_there.rs", 1, 1)]),
        citing(vec![lines("../Cargo.toml", 1, 1)]),
    ]});
    let h = Harness::start(world).await;
    h.configure_code(Some(CODE_DIR)).await.unwrap();
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    // A later change never reaches an analysis that was already admitted.
    let other = format!("{CODE_DIR}/src");
    h.configure_code(Some(&other)).await.unwrap();

    let EvalResultResponseV1 { record, assets } = investigated(&h, &evaluation_id).await;
    assert_eq!(
        record.status,
        EvalStatusV1::Completed,
        "{:?}",
        record.failure
    );
    assert_eq!(record.code_root.as_deref(), Some(CODE_DIR));
    let investigation = assets.investigation.unwrap();
    assert_eq!(investigation.code_root.as_deref(), Some(CODE_DIR));
    assert_eq!(investigation.suggestions.len(), 1);
    assert_eq!(
        investigation.suggestions[0].code_refs,
        [
            serde_json::from_value::<CodeRefV1>(lines("src/ids.rs", 1, 5)).unwrap(),
            serde_json::from_value::<CodeRefV1>(lines("./src/runtime.rs", 10, 20)).unwrap(),
        ]
    );
    assert_eq!(record.counters.rejected_suggestions, 2);
    let reasons: Vec<_> = investigation
        .rejected
        .iter()
        .map(|rejected| (rejected.index, rejected.reasons.join(" ")))
        .collect();
    assert_eq!(reasons.len(), 2);
    assert_eq!(reasons[0].0, 1);
    assert!(reasons[0].1.contains("src/not_there.rs") && reasons[0].1.contains("no such file"));
    assert_eq!(reasons[1].0, 2);
    assert!(reasons[1].1.contains("../Cargo.toml") && reasons[1].1.contains("`..`"));

    {
        let world = h.world();
        let sends = world.calls_to("harness::send");
        assert_eq!(sends.len(), 1);
        let send = &sends[0];
        // What the chat sends for a selected directory: in the session's
        // metadata (the console reads it) and in the turn's (the Harness
        // scopes the coder and shell calls with it).
        let scope = json!({"root": CODE_DIR});
        assert_eq!(send["session"]["metadata"]["fs_scope"], scope);
        assert_eq!(send["options"]["metadata"]["fs_scope"], scope);
        assert_eq!(send["session"]["metadata"]["origin"], "eval_monitor");
        assert_eq!(send["options"]["functions"]["allow"], json!(["*"]));
        assert_eq!(send["options"]["functions"]["deny"], json!([]));
        let limits = runtime::limits();
        assert_eq!(
            send["options"]["max_turns"],
            limits.investigation_code_max_turns
        );
        assert_eq!(send["options"]["max_turns"], 32);
        assert_eq!(
            send["options"]["max_total_tokens"],
            limits.investigation_code_max_total_tokens
        );
        assert_eq!(send["options"]["max_total_tokens"], 800_000);
        assert_eq!(
            send["options"]["max_output_tokens"],
            limits.investigation_max_output_tokens
        );
        assert_eq!(send["options"]["max_validation_retries"], 0);
        assert_eq!(send["options"]["output"]["type"], "json");
        let prompt = send["options"]["system_prompt"].as_str().unwrap();
        for part in [
            CODE_DIR,
            "the codebase of the iii workers",
            "coder::search",
            "coder::read-file",
            "any worker, not only the Harness",
            "read-only",
            "never call an eval::* function",
            "never instructions",
            "`code_refs`",
            "finish by calling it exactly once",
            "write no prose answer",
            "Your budget is 32 steps",
            "800000 tokens",
            "deliver your answer before the last one",
            "a concrete improvement to the Harness or another worker",
            "a concrete change to the Harness or another worker",
        ] {
            assert!(prompt.contains(part), "the prompt lacks {part:?}: {prompt}");
        }
        // Reading comes first, so submitting at once is not "your only action",
        // and the analyst is steered to the read functions, not the shell.
        for part in [
            "You cannot read anything else",
            "your only action",
            "or the shell",
            "Harness behavior",
            "concrete Harness improvement",
        ] {
            assert!(!prompt.contains(part), "the prompt has {part:?}: {prompt}");
        }
        let schema = serde_json::to_string(&send["options"]["output"]["schema"]).unwrap();
        assert!(schema.contains("code_refs"), "{schema}");
    }

    // The next analysis takes the configuration of its admission.
    let second = reanalyze(&h).await;
    let record = investigated(&h, &second).await.record;
    assert_eq!(record.code_root.as_deref(), Some(other.as_str()));
    let sends = h.world().calls_to("harness::send");
    assert_eq!(sends.len(), 2);
    assert_eq!(
        sends[1]["session"]["metadata"]["fs_scope"]["root"],
        json!(other)
    );
    assert!(sends[1]["options"]["system_prompt"]
        .as_str()
        .unwrap()
        .contains(&format!("working directory is {other}:")));
}

#[tokio::test]
async fn without_a_code_directory_the_investigation_is_unchanged_and_code_refs_are_rejected() {
    let mut world = World::new();
    world.analyst_result = json!({"suggestions": [
        citing(vec![lines("src/ids.rs", 1, 5)]),
        citing(vec![]),
    ]});
    let h = Harness::start(world).await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    // A directory chosen after admission does not turn code access on.
    h.configure_code(Some(CODE_DIR)).await.unwrap();

    let EvalResultResponseV1 { record, assets } = investigated(&h, &evaluation_id).await;
    assert_eq!(
        record.status,
        EvalStatusV1::Completed,
        "{:?}",
        record.failure
    );
    assert_eq!(record.code_root, None);
    assert!(serde_json::to_value(&record)
        .unwrap()
        .get("code_root")
        .is_none());
    let investigation = assets.investigation.unwrap();
    assert_eq!(investigation.code_root, None);
    assert_eq!(investigation.suggestions.len(), 1);
    assert!(investigation.suggestions[0].code_refs.is_empty());
    assert_eq!(investigation.rejected.len(), 1);
    assert_eq!(investigation.rejected[0].index, 0);
    assert!(investigation.rejected[0].reasons[0].contains("code access was off"));

    let world = h.world();
    let sends = world.calls_to("harness::send");
    assert_eq!(sends.len(), 1);
    let send = &sends[0];
    assert!(send["session"]["metadata"].get("fs_scope").is_none());
    assert!(send["options"]["metadata"].get("fs_scope").is_none());
    assert_eq!(send["options"]["functions"]["allow"], json!([]));
    assert_eq!(send["options"]["functions"]["deny"], json!([]));
    assert_eq!(send["options"]["max_turns"], 1);
    assert_eq!(send["options"]["max_total_tokens"], 200_000);
    let prompt = send["options"]["system_prompt"].as_str().unwrap();
    assert!(prompt.contains("You cannot read anything else, run other functions"));
    assert!(!prompt.contains("working directory") && !prompt.contains("coder::"));
    assert!(prompt.contains("your only action is to call it exactly once"));
    assert!(prompt.contains("a concrete Harness improvement"));
    assert!(prompt.contains("a concrete change to Harness behavior"));
    // Nothing to cite: the model is not offered the field either.
    let schema = serde_json::to_string(&send["options"]["output"]["schema"]).unwrap();
    assert!(!schema.contains("code_refs"), "{schema}");
    assert!(
        schema.contains("proposed_change"),
        "the rest is still offered"
    );
}

#[tokio::test]
async fn a_turn_that_used_all_its_steps_fails_with_its_own_code_and_keeps_its_usage() {
    let mut world = World::new();
    // What the Harness leaves as the result of a turn ended by the step cap.
    world.analyst_result = json!("max_turns (32) reached; ending the turn.");
    world.analyst_stop_reason = Some("max_turns");
    let h = Harness::start(world).await;
    h.configure_code(Some(CODE_DIR)).await.unwrap();
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();

    let EvalResultResponseV1 { record, assets } = investigated(&h, &evaluation_id).await;
    assert_eq!(record.status, EvalStatusV1::Failed);
    let failure = record.failure.unwrap();
    assert_eq!(
        (failure.stage, failure.code.as_str()),
        (EvalStatusV1::Investigating, "analyst_step_cap")
    );
    assert!(
        failure.message.contains("all its steps (32)"),
        "{failure:?}"
    );
    assert!(assets.investigation.is_none());
    assert!(assets.snapshot.is_some(), "the evidence is kept");
    assert_eq!(record.usage.llm_input_tokens, Some(1_200));
}

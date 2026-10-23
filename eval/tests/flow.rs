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
use harness::types::turn::TurnRecord;
use iii_sdk::{register_worker, IIIClient, InitOptions};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

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
    /// TypeSafe's HTTP 402 (no credits), with the provider's explanation.
    Billing,
}

struct World {
    state: BTreeMap<(String, String), Value>,
    /// session-manager `(kind, metadata)` by session id; absent means a
    /// console chat.
    sessions: HashMap<String, (&'static str, Value)>,
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
    /// The cost every `harness::metrics` call reports; `None` is unknown.
    cost_usd: Option<f64>,
    /// What `engine::workers::list` answers; `None` is an engine without it.
    workers: Option<Value>,
    /// What `e2e::dashboard::tests-list` answers; `None` is a down E2E.
    tests_list: Option<Value>,
    /// What `e2e::dashboard::stacks-list` answers; `None` is a down E2E.
    stacks: Option<Value>,
    /// Makes the n-th (0-based) `e2e::dashboard::execution-start` fail with
    /// this message; the others answer `plan-<n+1>`.
    start_failure: Option<(usize, &'static str)>,
    /// The n-th (0-based) `execution-start` is never answered.
    unanswered_start: Option<usize>,
    starts: usize,
    /// Every `e2e::dashboard::execution-get` fails like an E2E that is down.
    executions_down: bool,
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
            cost_usd: None,
            workers: None,
            tests_list: None,
            stacks: None,
            start_failure: None,
            unanswered_start: None,
            starts: 0,
            executions_down: false,
            sessions: HashMap::new(),
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
        namespace: &Value,
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
            // Set-if-absent when `expected` is omitted, as the engine answers.
            "state::compare-and-set" => {
                let key = scope_key();
                let current = self.state.get(&key).cloned();
                let matches = match data.get("expected") {
                    None | Some(Value::Null) => current.is_none(),
                    Some(expected) => current.as_ref() == Some(expected),
                };
                if matches {
                    self.state.insert(key, data["value"].clone());
                    json!({"swapped": true, "current": data["value"]})
                } else {
                    json!({"swapped": false, "current": current.unwrap_or(Value::Null)})
                }
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
                        cost_usd: self.cost_usd,
                        ..SessionUsageTotalsV1::default()
                    },
                    by_session: Vec::new(),
                    traces: None,
                })
                .unwrap()
            }
            "session::get" => {
                let (kind, metadata) = self
                    .sessions
                    .get(data["session_id"].as_str().unwrap_or_default())
                    .cloned()
                    .unwrap_or(("user", json!({"surface": "console"})));
                json!({"meta": {"session_id": data["session_id"],
                    "title": "Schedule the follow-up", "metadata": metadata, "kind": kind}})
            }
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
                // With `ids` only those executions, as the E2E answers.
                Some(list) if data["ids"].is_array() => json!({"executions": list["executions"]
                    .as_array()
                    .map(|entries| entries
                        .iter()
                        .filter(|entry| data["ids"].as_array().unwrap().contains(&entry["id"]))
                        .cloned()
                        .collect::<Vec<_>>())
                    .unwrap_or_default()}),
                Some(list) => list.clone(),
                None => {
                    return Some(Err(
                        "Function e2e::dashboard::executions-list not found in namespace p.".into(),
                    ))
                }
            },
            // The engine's own functions live in its `default` namespace, as
            // the real engine answers a call into another one.
            "engine::workers::list" if namespace != "default" => {
                return Some(Err(format!(
                    "Function engine::workers::list not found in namespace {}. It is registered \
                     in namespace(s): default.",
                    namespace.as_str().unwrap_or("(this worker's)")
                )))
            }
            "engine::workers::list" => match &self.workers {
                Some(list) => list.clone(),
                None => {
                    return Some(Err(
                        "Function engine::workers::list not found in namespace default.".into(),
                    ))
                }
            },
            "e2e::dashboard::tests-list" => match &self.tests_list {
                Some(list) => list.clone(),
                None => {
                    return Some(Err(
                        "Function e2e::dashboard::tests-list not found in namespace p.".into(),
                    ))
                }
            },
            "e2e::dashboard::stacks-list" => match &self.stacks {
                Some(list) => list.clone(),
                None => {
                    return Some(Err(
                        "Function e2e::dashboard::stacks-list not found in namespace p.".into(),
                    ))
                }
            },
            "e2e::dashboard::execution-start" => {
                let nth = self.starts;
                self.starts += 1;
                if self.unanswered_start == Some(nth) {
                    return None;
                }
                match self.start_failure {
                    Some((failing, message)) if failing == nth => return Some(Err(message.into())),
                    _ => json!({"execution_id": format!("plan-{}", nth + 1)}),
                }
            }
            "e2e::dashboard::execution-get" if self.executions_down => {
                return Some(Err(
                    "Function e2e::dashboard::execution-get not found in namespace p.".into(),
                ))
            }
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

/// An analysis `Harness::end_turn` seeded or found.
struct Seeded {
    evaluation_id: Option<String>,
    reused: bool,
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
            let Some(reply) = world.respond(
                &function,
                &frame["data"],
                &frame["invocation_id"],
                &frame["namespace"],
            ) else {
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
        // A namespaced worker, like the deployed one: the engine's own
        // functions are not in it.
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

    /// Stands in for the admission the worker no longer has (it never
    /// consults or admits user chats): an analysis of the turn is seeded in
    /// state and its first step enqueued, so the rest of the pipeline is
    /// still exercised. The same turn again reuses its analysis. An analyst
    /// session's turn re-enqueues its analysis' current step, as the
    /// harness' wake-up of the investigation does.
    async fn end_turn(&self, session_id: &str, turn_id: &str) -> Seeded {
        if let Some(evaluation_id) = session_id.strip_prefix(ids::ANALYST_PREFIX) {
            if let Some(record) = state::get_record(&self.iii, evaluation_id)
                .await
                .unwrap()
                .filter(|record| !record.status.is_terminal())
            {
                eval::queue::enqueue_step(&self.iii, evaluation_id, record.step)
                    .await
                    .unwrap();
            }
            return Seeded {
                evaluation_id: Some(evaluation_id.into()),
                reused: true,
            };
        }
        self.seed(session_id, turn_id, AnalysisOriginV1::Automatic, false)
            .await
    }

    async fn seed(
        &self,
        session_id: &str,
        turn_id: &str,
        origin: AnalysisOriginV1,
        reanalyze: bool,
    ) -> Seeded {
        let observation_key = ids::observation_key(session_id, turn_id);
        let mut supersedes = None;
        // The turn's newest analysis: the one no other analysis replaces.
        let earlier: Vec<_> = state::list_records(&self.iii)
            .await
            .unwrap()
            .into_iter()
            .filter(|record| record.observation_key == observation_key)
            .collect();
        if let Some(latest) = earlier.iter().find(|record| {
            !earlier
                .iter()
                .any(|other| other.supersedes.as_deref() == Some(record.evaluation_id.as_str()))
        }) {
            if !reanalyze {
                return Seeded {
                    evaluation_id: Some(latest.evaluation_id.clone()),
                    reused: true,
                };
            }
            supersedes = Some(latest.evaluation_id.clone());
        }
        let config = state::get_config(&self.iii)
            .await
            .unwrap()
            .expect("configure the monitor before seeding an analysis");
        let now = ids::now_ms();
        let record = AnalysisRecordV1 {
            schema_version: RECORD_SCHEMA_VERSION,
            evaluation_id: ids::evaluation_id(),
            observation_key: observation_key.clone(),
            origin,
            session_id: session_id.into(),
            turn_id: turn_id.into(),
            source_title: None,
            model: config.model.clone(),
            code_root: config.code_repository.clone(),
            config_revision: config.revision.clone(),
            rules_version: RULES_VERSION.into(),
            criteria_version: runtime::criteria_version(),
            status: EvalStatusV1::Queued,
            step: 0,
            created_at: now,
            updated_at: now,
            deadline: now + 30 * 60 * 1_000,
            // Turns written before the monitor was configured are history.
            observe_since: config.updated_at,
            completed_at: None,
            counters: AnalysisCountersV1::default(),
            stages: vec![StageTimeV1 {
                status: EvalStatusV1::Queued,
                at: now,
            }],
            usage: MonitorUsageV1::default(),
            coverage: None,
            routing: None,
            pending_reason: None,
            judge_call: None,
            analyst: None,
            failure: None,
            supersedes,
            harness_version: match origin {
                AnalysisOriginV1::Automatic => self.harness_version(),
                AnalysisOriginV1::Manual => None,
            },
            signals: BTreeMap::new(),
        };
        state::put_record(&self.iii, &record).await.unwrap();
        eval::queue::enqueue_step(&self.iii, &record.evaluation_id, 0)
            .await
            .unwrap();
        Seeded {
            evaluation_id: Some(record.evaluation_id),
            reused: false,
        }
    }

    /// The version of this namespace's Harness the engine lists, which an
    /// automatic analysis ran on.
    fn harness_version(&self) -> Option<String> {
        let namespace = self.iii.namespace().unwrap_or_else(|| "default".into());
        let workers = self.world().workers.clone()?;
        workers["workers"]
            .as_array()?
            .iter()
            .find(|worker| worker["name"] == "harness" && worker["namespace"] == namespace.as_str())
            .and_then(|worker| worker["version"].as_str())
            .map(str::to_string)
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

    let config = h.configure(true).await;
    let admitted = h.end_turn(ROOT, TURN).await;
    assert!(!admitted.reused);
    let evaluation_id = admitted.evaluation_id.unwrap();
    let again = h.end_turn(ROOT, TURN).await;
    assert!(again.reused);
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

    // The investigation's own turn completion only wakes it.
    h.end_turn(&analyst.session_id, analyst.turn_id.as_deref().unwrap())
        .await;
    h.drain().await;
    assert_eq!(h.records(), 1);

    let EvalResultResponseV1 { record, assets, .. } = h.result(&evaluation_id).await;
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
        [RoutingReasonV1::NeedsInvestigation]
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
async fn only_a_needs_investigation_answer_reaches_the_llm() {
    // The default world has a deterministic finding; Jev's answer alone decides.
    let h = Harness::start(World::new()).await;
    h.configure(true).await;
    h.world().judge = JudgeMode::Answer("expected_behavior", 0.95);
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    h.drain().await;
    let EvalResultResponseV1 { record, assets, .. } = h.result(&evaluation_id).await;
    assert_eq!(record.status, EvalStatusV1::Completed);
    assert_eq!(record.counters.diagnostics, 1, "a finding does not route");
    let routing = record.routing.unwrap();
    assert!(!routing.investigate && routing.reasons.is_empty());
    assert!(assets.investigation.is_none());
    assert!(h.world().calls_to("harness::send").is_empty());

    // A manual request neither: expected behavior, an unsure answer and
    // insufficient evidence all stop at Jev.
    for (choice, confidence) in [
        ("expected_behavior", 0.95),
        ("expected_behavior", 0.3),
        ("insufficient_evidence", 0.9),
    ] {
        h.world().judge = JudgeMode::Answer(choice, confidence);
        let evaluation_id = reanalyze(&h).await;
        h.drain().await;
        let record = h.result(&evaluation_id).await.record;
        assert_eq!(
            record.status,
            EvalStatusV1::Completed,
            "{choice} {confidence}"
        );
        assert_eq!(record.origin, AnalysisOriginV1::Manual);
        let routing = record.routing.unwrap();
        assert!(
            !routing.investigate && routing.reasons.is_empty(),
            "{choice} {confidence}"
        );
    }
    assert!(h.world().calls_to("harness::send").is_empty());

    // needs_investigation, even unsure, investigates and says why.
    h.world().judge = JudgeMode::Answer("needs_investigation", 0.55);
    let evaluation_id = reanalyze(&h).await;
    h.drain().await;
    let record = h.result(&evaluation_id).await.record;
    assert_eq!(record.status, EvalStatusV1::Investigating);
    let routing = record.routing.unwrap();
    assert!(routing.investigate);
    assert_eq!(routing.reasons, [RoutingReasonV1::NeedsInvestigation]);
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
    let EvalResultResponseV1 { record, assets, .. } = h.result(&evaluation_id).await;
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
    let again = reanalyze(&h).await;
    h.drain().await;
    let record = h.result(&again).await.record;
    assert_eq!(record.failure.unwrap().code, "judge_invalid_response");

    // The chosen LLM failing is reported at the investigating stage.
    {
        let mut world = h.world();
        world.judge = JudgeMode::Answer("needs_investigation", 0.9);
        world.analyst_fails = true;
    }
    let third = reanalyze(&h).await;
    h.drain().await;
    let record = h.result(&third).await.record;
    let analyst = record.analyst.unwrap();
    h.end_turn(&analyst.session_id, analyst.turn_id.as_deref().unwrap())
        .await;
    h.drain().await;
    let EvalResultResponseV1 { record, assets, .. } = h.result(&third).await;
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
async fn recovery_requeues_lost_steps_and_deadlines_fail_pending_work() {
    let mut h = Harness::start({
        let mut world = World::new();
        world.descendants_running = true;
        world
    })
    .await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    // Crash after saving the record, before its step ran.
    h.world().steps.clear();
    h.restart();
    let recovered = runtime::recover(&h.deps).await.unwrap();
    assert_eq!(recovered.requeued, 1);
    assert!(h
        .world()
        .state
        .keys()
        .all(|(scope, _)| scope != "eval_observation" && scope != "eval_suggestion"));
    let again = h.end_turn(ROOT, TURN).await;
    assert!(again.reused);
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
    let EvalResultResponseV1 { record, assets, .. } = h.result(&evaluation_id).await;
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
    let EvalResultResponseV1 { record, assets, .. } = h.result(&evaluation_id).await;
    let snapshot = assets.snapshot.unwrap();
    assert_eq!(snapshot.window_turn_ids, ["t_final", "t_wake"]);
    assert!(
        snapshot.diagnostics.is_empty(),
        "history from before the monitor is not reported as new"
    );
    assert_eq!(record.counters.diagnostics, 0);
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
async fn an_earlier_turn_already_analyzed_is_left_out_of_the_next_window() {
    let h = Harness::start(World::new()).await;
    h.configure(true).await;
    tokio::time::sleep(Duration::from_millis(5)).await;
    {
        let mut world = h.world();
        world.entries.insert(ROOT.into(), rediscovery_transcript());
        world
            .status
            .insert(ROOT.into(), status(ROOT, TURN, "completed", None));
    }
    // TURN is analyzed to the end, its rediscovery counted once.
    let first = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    let first = investigated(&h, &first).await;
    assert_eq!(
        first.record.status,
        EvalStatusV1::Completed,
        "{:?}",
        first.record.failure
    );
    assert_eq!(first.assets.snapshot.unwrap().diagnostics.len(), 1);

    // The user continues: the next turn's window stops at the analyzed one.
    {
        let mut world = h.world();
        let entries = world.entries.get_mut(ROOT).unwrap();
        entries.push(user("e_idem_next", "And now the receipt."));
        entries.push(assistant_text("e_t_next_0_assistant", "R-1."));
        world
            .status
            .insert(ROOT.into(), status(ROOT, "t_next", "completed", None));
    }
    let next = h.end_turn(ROOT, "t_next").await.evaluation_id.unwrap();
    h.drain().await;
    let snapshot = h.result(&next).await.assets.snapshot.unwrap();
    assert_eq!(snapshot.window_turn_ids, ["t_next"]);
    assert!(
        snapshot.diagnostics.is_empty(),
        "the analyzed turn's signal is not counted again: {:?}",
        snapshot.diagnostics
    );
}

#[tokio::test]
async fn provider_checks_are_reported() {
    let h = Harness::start(World::new()).await;
    h.configure(true).await;
    let monitor = runtime::monitor_state(
        &h.deps,
        MonitorStateRequestV1 {
            check_providers: true,
        },
    )
    .await
    .unwrap();
    let triage = monitor.triage.unwrap();
    assert!(triage.available);
    assert_eq!(triage.models, ["jev-test-1"]);
    assert!(!h.world().calls_to("judge::models::list").is_empty());
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

/// A manual reanalysis of the root session's latest turn.
async fn reanalyze(h: &Harness) -> String {
    let turn_id = h
        .world()
        .status
        .get(ROOT)
        .and_then(|status| status["turn_id"].as_str())
        .unwrap_or(TURN)
        .to_string();
    h.seed(ROOT, &turn_id, AnalysisOriginV1::Manual, true)
        .await
        .evaluation_id
        .unwrap()
}

/// The same turn admitted again by the automatic path: another automatic
/// analysis of it.
async fn admitted_again(h: &Harness) -> String {
    h.seed(ROOT, TURN, AnalysisOriginV1::Automatic, true)
        .await
        .evaluation_id
        .unwrap()
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

    let EvalResultResponseV1 { record, assets, .. } = investigated(&h, &evaluation_id).await;
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
        // It reads untrusted transcripts: only the read-only functions its
        // prompt names (and the contract lookup), nothing that writes, runs a
        // shell or starts a session. Only a person starts an E2E execution or
        // records a review, so those stay denied as well.
        assert_eq!(
            send["options"]["functions"],
            json!({
                "allow": ["coder::search", "coder::tree", "coder::read-file",
                    "session::messages", "github::pr::list", "engine::functions::info"],
                "deny": ["eval::*", "e2e::dashboard::execution-*"],
                "expose": "agent_trigger"
            })
        );
        let prompt = send["options"]["system_prompt"].as_str().unwrap();
        for id in [
            "coder::search",
            "coder::tree",
            "coder::read-file",
            "session::messages",
            "github::pr::list",
        ] {
            assert!(prompt.contains(id), "the prompt names {id}");
        }
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
            "github::pr::list",
            "say so in that suggestion's `limitations`",
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

    let EvalResultResponseV1 { record, assets, .. } = investigated(&h, &evaluation_id).await;
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
    // Deny all: the analyst can call nothing.
    assert_eq!(
        send["options"]["functions"],
        json!({"allow": [], "deny": [], "expose": "agent_trigger"})
    );
    assert_eq!(send["options"]["max_turns"], 1);
    assert_eq!(send["options"]["max_total_tokens"], 200_000);
    let prompt = send["options"]["system_prompt"].as_str().unwrap();
    assert!(prompt.contains("You cannot read anything else, run other functions"));
    assert!(!prompt.contains("working directory") && !prompt.contains("coder::"));
    assert!(!prompt.contains("github::pr::list"));
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

    let EvalResultResponseV1 { record, assets, .. } = investigated(&h, &evaluation_id).await;
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

#[tokio::test]
async fn no_credits_fail_with_their_own_code_and_keep_the_providers_text() {
    let mut world = World::new();
    world.judge = JudgeMode::Billing;
    let h = Harness::start(world).await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    h.drain().await;
    let EvalResultResponseV1 { record, assets, .. } = h.result(&evaluation_id).await;
    assert_eq!(record.status, EvalStatusV1::Failed);
    let failure = record.failure.unwrap();
    assert_eq!(
        (failure.stage, failure.code.as_str()),
        (EvalStatusV1::Judging, "judge_out_of_credits")
    );
    assert!(failure.message.contains("HTTP 402"), "{failure:?}");
    assert!(
        failure
            .message
            .contains("Your organization has no available TypeSafe API credits."),
        "{failure:?}"
    );
    // The provider still reports a generic `http` error; only the monitor's
    // code is specific.
    let triage_failure = assets.triage_failure.unwrap();
    assert_eq!(
        (triage_failure.code.as_str(), triage_failure.http_status),
        ("http", Some(402))
    );
    assert!(assets.snapshot.is_some(), "the evidence survives");
    assert!(h.world().calls_to("harness::send").is_empty());
}

async fn configure_capped(h: &Harness, cap: Value) -> Result<MonitorConfigV1, EvalError> {
    runtime::configure(
        &h.deps,
        serde_json::from_value(json!({"enabled": true,
            "model": {"model": "analyst-model", "provider": "analyst-provider",
                      "thinking_level": "low"},
            "daily_cost_cap_usd": cap}))
        .unwrap(),
    )
    .await
}

async fn cost_block(h: &Harness) -> MonitorCostV1 {
    runtime::monitor_state(&h.deps, MonitorStateRequestV1::default())
        .await
        .unwrap()
        .cost
}

#[tokio::test]
async fn the_daily_cost_cap_is_validated_and_reported() {
    let h = Harness::start(World::new()).await;
    for invalid in [json!(0), json!(-1.5)] {
        let error = configure_capped(&h, invalid).await.unwrap_err();
        assert!(matches!(error, EvalError::InvalidRequest(_)), "{error:?}");
        assert!(error.to_string().contains("daily_cost_cap_usd"), "{error}");
    }
    // Without a cap the revision is what it always was.
    let plain = h.configure(true).await;
    assert_eq!(plain.daily_cost_cap_usd, None);
    assert!(serde_json::to_value(&plain)
        .unwrap()
        .get("daily_cost_cap_usd")
        .is_none());
    let capped = configure_capped(&h, json!(0.10)).await.unwrap();
    assert_eq!(capped.daily_cost_cap_usd, Some(0.10));
    assert_ne!(capped.revision, plain.revision);
    assert_eq!(h.configure(true).await.revision, plain.revision);
    configure_capped(&h, json!(0.10)).await.unwrap();

    let empty = cost_block(&h).await;
    assert_eq!((empty.today_usd, empty.today_unknown), (0.0, 0));
    assert_eq!((empty.cap_usd, empty.capped), (Some(0.10), false));
    assert_eq!(empty.per_analysis.count, 0);
    assert_eq!(empty.per_analysis.median, None);

    // An investigation without a reported cost is unknown, never zero.
    let first = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    let first = investigated(&h, &first).await.record;
    assert_eq!(first.usage.llm_cost_usd, None);
    let cost = cost_block(&h).await;
    assert_eq!((cost.today_usd, cost.today_unknown), (0.0, 1));
    assert_eq!((cost.per_analysis.count, cost.per_analysis.unknown), (0, 1));
    assert!(!cost.capped);

    h.world().cost_usd = Some(0.06);
    let second = reanalyze(&h).await;
    assert_eq!(
        investigated(&h, &second).await.record.usage.llm_cost_usd,
        Some(0.06)
    );
    let cost = cost_block(&h).await;
    assert_eq!((cost.today_usd, cost.today_unknown), (0.06, 1));
    assert!(!cost.capped, "0.06 is under the 0.10 cap");

    let third = reanalyze(&h).await;
    investigated(&h, &third).await;
    let cost = cost_block(&h).await;
    assert_eq!((cost.today_usd, cost.capped), (0.12, true));
    assert_eq!(cost.since, eval::cost::day_start(ids::now_ms()));
    let stats = cost.per_analysis;
    assert_eq!(
        (
            stats.count,
            stats.min,
            stats.median,
            stats.max,
            stats.unknown
        ),
        (2, Some(0.06), Some(0.06), Some(0.06), 1)
    );
}

#[tokio::test]
async fn the_cap_is_checked_where_the_money_is_spent_and_deleting_gives_nothing_back() {
    let h = Harness::start(World::new()).await;
    configure_capped(&h, json!(0.10)).await.unwrap();
    h.world().cost_usd = Some(0.06);
    // One analysis on record: investigations of this setup cost 0.06.
    let first = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    investigated(&h, &first).await;
    assert_eq!(cost_block(&h).await.per_analysis.median, Some(0.06));

    // A burst is admitted whole: under the cap, and no cost reported yet.
    let (second, third) = (admitted_again(&h).await, admitted_again(&h).await);
    assert!(!cost_block(&h).await.capped);
    // The second investigates; once it is running the third would reach the
    // cap (0.06 spent + 0.06 in flight), so it never calls the model.
    let second = investigated(&h, &second).await.record;
    assert_eq!(second.status, EvalStatusV1::Completed);
    let third = h.result(&third).await.record;
    assert_eq!(third.status, EvalStatusV1::Failed);
    let failure = third.failure.unwrap();
    assert_eq!(
        (failure.stage, failure.code.as_str()),
        (EvalStatusV1::Investigating, "cost_cap")
    );
    assert!(failure.message.contains("$0.10"), "{failure:?}");
    assert!(third.analyst.is_none());
    assert_eq!(h.world().calls_to("harness::send").len(), 2);
    assert_eq!(
        cost_block(&h).await.today_usd,
        0.12,
        "the day's spend is persisted as it happens"
    );

    // Deleting the analyses that spent it does not give the budget back.
    for id in [first, second.evaluation_id] {
        runtime::delete(&h.deps, EvaluationIdRequestV1 { evaluation_id: id })
            .await
            .unwrap();
    }
    let cost = cost_block(&h).await;
    assert_eq!((cost.today_usd, cost.capped), (0.12, true));
}

#[tokio::test]
async fn a_reanalysis_records_what_it_replaces_and_the_list_filters_by_turn() {
    let h = Harness::start(World::new()).await;
    h.configure(true).await;
    let first = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    h.drain().await;
    runtime::cancel(
        &h.deps,
        EvaluationIdRequestV1 {
            evaluation_id: first.clone(),
        },
    )
    .await
    .unwrap();
    let second = reanalyze(&h).await;
    runtime::cancel(
        &h.deps,
        EvaluationIdRequestV1 {
            evaluation_id: second.clone(),
        },
    )
    .await
    .unwrap();
    let third = reanalyze(&h).await;
    // An analysis of another turn of the same session.
    let other = h.end_turn(ROOT, "t_other").await.evaluation_id.unwrap();

    let first_record = h.result(&first).await.record;
    assert_eq!(first_record.supersedes, None);
    assert!(serde_json::to_value(&first_record)
        .unwrap()
        .get("supersedes")
        .is_none());
    assert_eq!(
        h.result(&second).await.record.supersedes.as_deref(),
        Some(first.as_str())
    );
    assert_eq!(
        h.result(&third).await.record.supersedes.as_deref(),
        Some(second.as_str())
    );

    let list = |observation_key: Option<String>| {
        let deps = h.deps.clone();
        async move {
            runtime::list(
                &deps,
                EvalListRequestV1 {
                    limit: None,
                    observation_key,
                },
            )
            .await
            .unwrap()
            .evaluations
            .into_iter()
            .map(|record| record.evaluation_id)
            .collect::<Vec<_>>()
        }
    };
    let key = first_record.observation_key;
    let mut turn: Vec<_> = list(Some(key)).await;
    turn.sort();
    let mut expected = vec![first, second, third];
    expected.sort();
    assert_eq!(turn, expected, "every analysis of the turn, and only those");
    assert_eq!(list(None).await.len(), 4);
    assert!(list(Some(ids::observation_key(ROOT, "t_nobody")))
        .await
        .is_empty());
    assert!(!expected.contains(&other));
}

#[tokio::test]
async fn collection_records_the_signals() {
    let h = Harness::start(World::new()).await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    let admitted = h.result(&evaluation_id).await.record;
    assert!(admitted.signals.is_empty(), "nothing is collected yet");

    h.drain().await;
    let EvalResultResponseV1 { record, assets, .. } = h.result(&evaluation_id).await;
    assert_eq!(record.counters.diagnostics, 1);
    let diagnostic = &assets.snapshot.unwrap().diagnostics[0];
    assert_eq!(
        record.signals,
        BTreeMap::from([(
            format!("{}:{}", diagnostic.rule_id, diagnostic.target),
            1u32
        )])
    );
    assert!(
        record
            .signals
            .keys()
            .any(|key| key.starts_with("repeated_contract_discovery:")),
        "{:?}",
        record.signals
    );
}

#[tokio::test]
async fn the_analyst_is_offered_the_e2e_scenarios_once_per_ten_minutes() {
    let long = "x".repeat(300);
    let mut world = World::new();
    world.tests_list = Some(json!({"total": 59, "next_cursor": "c:100", "rows": [
        {"test_id": "tool_contract_recovery",
         "spec": {"title": "Tool Contract\nRecovery",
                  "summary": "Recover when a contract\n  changes mid-run."}},
        {"test_id": "timer_wake", "spec": {"title": "Timer Wake", "summary": long}},
        {"spec": {"title": "a row without an id"}},
    ]}));
    let h = Harness::start(world).await;
    h.configure(true).await;
    let first = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    investigated(&h, &first).await;
    {
        let world = h.world();
        assert_eq!(
            world.calls_to("e2e::dashboard::tests-list"),
            [json!({"limit": 100})]
        );
        let send = &world.calls_to("harness::send")[0];
        let message: Value = serde_json::from_str(send["message"].as_str().unwrap()).unwrap();
        assert_eq!(
            message["monitor"]["e2e_scenarios"],
            json!({"total": 59, "scenarios": [
                {"id": "tool_contract_recovery", "title": "Tool Contract Recovery",
                 "summary": "Recover when a contract changes mid-run."},
                {"id": "timer_wake", "title": "Timer Wake", "summary": "x".repeat(160)},
            ]})
        );
        let prompt = send["options"]["system_prompt"].as_str().unwrap();
        for part in [
            "`monitor.e2e_scenarios`",
            "set `validation.scenario_id` to its exact `id`",
            "Never invent an id",
            "set it to null only when none does",
        ] {
            assert!(prompt.contains(part), "the prompt lacks {part:?}: {prompt}");
        }
    }

    // A second analysis within the ten minutes reuses the list.
    let second = reanalyze(&h).await;
    investigated(&h, &second).await;
    assert_eq!(h.world().calls_to("e2e::dashboard::tests-list").len(), 1);
    assert!(h.world().calls_to("harness::send")[1]["message"]
        .as_str()
        .unwrap()
        .contains("timer_wake"));

    // An E2E that does not answer only leaves the analyst without the list.
    let down = Harness::start(World::new()).await;
    down.configure(true).await;
    let evaluation_id = down.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    let record = investigated(&down, &evaluation_id).await.record;
    assert_eq!(
        record.status,
        EvalStatusV1::Completed,
        "{:?}",
        record.failure
    );
    let world = down.world();
    let send = &world.calls_to("harness::send")[0];
    let message: Value = serde_json::from_str(send["message"].as_str().unwrap()).unwrap();
    assert!(message["monitor"].get("e2e_scenarios").is_none());
    let prompt = send["options"]["system_prompt"].as_str().unwrap();
    assert!(!prompt.contains("e2e_scenarios"), "{prompt}");
    assert!(
        prompt.contains("an existing harness-e2e scenario"),
        "{prompt}"
    );
}

// ---------------------------------------------------------------------------
// Capture of the analyzed turn and the suggestion's check
// ---------------------------------------------------------------------------

/// The Harness's turn record of `turn_id`, with fields beyond the options.
fn turn_record(turn_id: &str) -> Value {
    let minimal = json!({
        "turn_id": turn_id,
        "session_id": ROOT,
        "status": "completed",
        "step": 7,
        "turn_count": 3,
        "depth": 0,
        "options": {
            "model": "task-model",
            "provider": "p",
            "system_prompt": "You are an agent.",
            "max_turns": 16,
            "functions": {"expose": "agent_trigger"}
        },
        "created_at": 1,
        "updated_at": 1
    });
    serde_json::to_value(serde_json::from_value::<TurnRecord>(minimal).unwrap()).unwrap()
}

/// An analysis whose suggestion carries a check of its decision point, made
/// while the Harness holds `record` as the session's latest turn.
async fn captured(mut world: World, record: Value) -> (Harness, String) {
    let decision = format!("e_{TURN}_1_assistant");
    let mut planned = suggestion(&decision);
    planned["check"] = json!({
        "decision_point": decision,
        "signal": {"rule": "contract_rediscovery"},
        "change": [{"target": format!("e_{TURN}_1_notice_0"), "remove": true}]
    });
    world.analyst_result = json!({"suggestions": [planned]});
    world
        .state
        .insert((state::HARNESS_TURN_SCOPE.into(), ROOT.into()), record);
    let h = Harness::start(world).await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    h.drain().await;
    let analyst = h.result(&evaluation_id).await.record.analyst.unwrap();
    h.end_turn(&analyst.session_id, analyst.turn_id.as_deref().unwrap())
        .await;
    h.drain().await;
    (h, evaluation_id)
}

#[tokio::test]
async fn a_suggestion_check_is_kept_with_the_captured_turn_and_nothing_replays_it() {
    let (h, evaluation_id) = captured(World::new(), turn_record(TURN)).await;
    let result = h.result(&evaluation_id).await;
    assert_eq!(result.assets.capture.as_ref().unwrap().turn_id, TURN);
    let suggestion = &result.assets.investigation.as_ref().unwrap().suggestions[0];
    assert!(suggestion.check.is_some(), "{}", suggestion.limitations);
    // The one-step replay is gone: it is not offered and no model is sampled.
    assert!(!eval::surface::catalog()
        .iter()
        .any(|spec| spec.function_id == "eval::reproduce"));
    assert!(h.world().calls_to("router::complete").is_empty());
    assert!(h.world().calls_to(INFO).is_empty());
}

#[tokio::test]
async fn a_legacy_stored_config_stays_inert_and_never_breaks_eval() {
    // A config stored by the removed chat monitor (with its enabled_since,
    // observer and rejection fields) no longer parses: it is ignored as if
    // never configured, never migrated, and configuring replaces it.
    let legacy = json!({"enabled": true, "enabled_since": 1_700_000_000_000_i64,
        "observer_bound": true, "observer_error": "gone",
        "last_rejection": {"reason": "capacity", "at": 1_700_000_000_000_i64},
        "model": {"model": "analyst-model", "provider": "analyst-provider",
                  "thinking_level": "low"},
        "daily_cost_cap_usd": 1.0,
        "revision": "legacy", "updated_at": 1_700_000_000_000_i64});
    let h = Harness::start(World::new()).await;
    h.world().state.insert(
        (state::CONFIG_SCOPE.into(), state::CONFIG_KEY.into()),
        legacy.clone(),
    );
    assert_eq!(state::get_config(&h.deps.iii).await.unwrap(), None);
    let monitor = runtime::monitor_state(&h.deps, MonitorStateRequestV1::default())
        .await
        .unwrap();
    assert_eq!(
        monitor.config, None,
        "the legacy record reads as unconfigured"
    );
    assert_eq!(monitor.cost.cap_usd, None, "its cap is not applied");
    runtime::sweep(&h.deps).await.unwrap();
    assert_eq!(
        h.world()
            .state
            .get(&(state::CONFIG_SCOPE.into(), state::CONFIG_KEY.into())),
        Some(&legacy),
        "reading never migrates or rewrites the legacy record"
    );

    let config = h.configure(true).await;
    assert_ne!(config.revision, "legacy");
    assert_eq!(
        state::get_config(&h.deps.iii).await.unwrap(),
        Some(config.clone())
    );
    let monitor = runtime::monitor_state(&h.deps, MonitorStateRequestV1::default())
        .await
        .unwrap();
    assert_eq!(monitor.config, Some(config));
}

#[tokio::test]
async fn user_chats_are_never_admitted() {
    // The worker no longer observes chats: no admission entry is offered, and
    // a finished chat from any client, configured or not, never becomes an
    // analysis nor makes the worker read its session record.
    let offered: Vec<&str> = eval::surface::catalog()
        .iter()
        .map(|spec| spec.function_id)
        .collect();
    for removed in ["eval::on-turn-completed", "eval::analyze-session"] {
        assert!(!offered.contains(&removed), "{removed} in {offered:?}");
    }
    for metadata in [
        json!({"surface": "console", "model": "anthropic::m", "fs_scope": {"root": "/w"},
            "agent_profile": {"id": "default"}}),
        json!({"surface": "slack"}),
        json!({"surface": "telegram"}),
    ] {
        let mut world = World::new();
        world
            .sessions
            .insert(ROOT.into(), ("user", metadata.clone()));
        let h = Harness::start(world).await;
        runtime::sweep(&h.deps).await.unwrap();
        h.configure(true).await;
        runtime::recover(&h.deps).await.unwrap();
        runtime::sweep(&h.deps).await.unwrap();
        h.drain().await;
        assert_eq!(h.records(), 0, "{metadata}: nothing admitted");
        let world = h.world();
        assert!(
            world.calls_to("session::get").is_empty(),
            "{metadata}: the session record is never consulted"
        );
        assert!(world.calls_to("eval::step").is_empty(), "{metadata}");
    }
}

#[tokio::test]
async fn the_capture_is_the_analyzed_turns_whole_record_even_after_another_turn_ran() {
    let (h, evaluation_id) = captured(World::new(), turn_record(TURN)).await;
    // The Harness keeps only a session's latest turn: the next one replaces it.
    h.world().state.insert(
        (state::HARNESS_TURN_SCOPE.into(), ROOT.into()),
        turn_record("t_next"),
    );
    let capture = h.result(&evaluation_id).await.assets.capture.unwrap();
    assert_eq!(capture.record, Some(turn_record(TURN)));
    assert_eq!(capture.record_omitted, None);
    // The options are still there, from the copy.
    assert_eq!(capture.options, turn_record(TURN)["options"]);
}

#[tokio::test]
async fn the_capture_hydrates_the_harness_prompt_references() {
    let (prompt, skills) = (
        "You are the session's exact agent.",
        "<available_skills>review</available_skills>",
    );
    let digest = |text: &str| format!("sha256:{:x}", Sha256::digest(text.as_bytes()));
    let (prompt_ref, skills_ref) = (digest(prompt), digest(skills));
    let mut record = turn_record(TURN);
    record["options"]["system_prompt"] = json!({"$ref": prompt_ref});
    record["options"]["skill_context"] = json!({
        "filter": ["review"],
        "baseline": {"$ref": skills_ref}
    });
    record["future_evidence"] = json!({"kept": true});
    let mut world = World::new();
    world.state.insert(
        ("harness_prompt".into(), prompt_ref),
        json!({"body": prompt, "created_at": 1}),
    );
    world.state.insert(
        ("harness_prompt".into(), skills_ref),
        json!({"body": skills, "created_at": 1}),
    );

    let (h, evaluation_id) = captured(world, record).await;
    let capture = h.result(&evaluation_id).await.assets.capture.unwrap();
    assert_eq!(capture.options["system_prompt"], prompt);
    assert_eq!(capture.options["skill_context"]["baseline"], skills);
    let captured_record = capture.record.unwrap();
    assert_eq!(captured_record["options"], capture.options);
    assert_eq!(captured_record["future_evidence"], json!({"kept": true}));
}

#[tokio::test]
async fn a_turn_record_too_big_for_the_assets_is_left_out_and_says_why() {
    let mut record = turn_record(TURN);
    // Outside the options: the captured options stay small.
    record["result"] = json!("x".repeat(2 * 1024 * 1024));
    let (h, evaluation_id) = captured(World::new(), record).await;
    let result = h.result(&evaluation_id).await;
    assert_eq!(result.record.status, EvalStatusV1::Completed);
    let capture = result.assets.capture.unwrap();
    assert_eq!(capture.record, None);
    let why = capture.record_omitted.unwrap();
    assert!(why.contains("2097152-byte limit"), "{why}");
    assert_eq!(capture.options, turn_record(TURN)["options"]);
    assert_eq!(capture.turn_id, TURN);
}

// ---------------------------------------------------------------------------
// Loop cycles: one record per cycle in the loop's own scope, whichever input
// started it.
// ---------------------------------------------------------------------------

/// The scopes the monitor wrote before the loop existed.
const LEGACY_SCOPES: [&str; 5] = [
    state::CONFIG_SCOPE,
    "eval_observation",
    state::ANALYSIS_SCOPE,
    state::ASSETS_SCOPE,
    "eval_suggestion",
];

async fn start_cycle(h: &Harness, request: Value) -> Result<CycleStartResponseV1, EvalError> {
    runtime::cycle_start(&h.deps, serde_json::from_value(request).unwrap()).await
}

fn cycle_request(cycle_id: &str) -> CycleIdRequestV1 {
    serde_json::from_value(json!({"cycle_id": cycle_id})).unwrap()
}

async fn cycle(h: &Harness, cycle_id: &str) -> CycleRecordV1 {
    runtime::cycle_status(&h.deps, cycle_request(cycle_id))
        .await
        .unwrap()
        .expect("cycle exists")
}

fn statuses(record: &CycleRecordV1) -> Vec<CycleStatusV1> {
    record.history.iter().map(|event| event.status).collect()
}

fn stored_cycles(h: &Harness) -> Vec<String> {
    h.world()
        .state
        .keys()
        .filter(|(scope, _)| scope == state::CYCLE_SCOPE)
        .map(|(_, key)| key.clone())
        .collect()
}

#[tokio::test]
async fn cycle_from_objective_and_from_execution_share_one_history() {
    let h = Harness::start(World::new()).await;
    let from_objective = start_cycle(&h, json!({"objective": "  Stop rediscovering contracts  "}))
        .await
        .unwrap();
    let from_execution = start_cycle(&h, json!({"execution_id": "exec-1"}))
        .await
        .unwrap();
    assert_eq!(from_objective.status, CycleStatusV1::Queued);
    assert_eq!(from_execution.status, CycleStatusV1::Queued);
    assert_ne!(from_objective.cycle_id, from_execution.cycle_id);
    assert!(ids::is_cycle(&from_objective.cycle_id));
    assert!(ids::is_cycle(&from_execution.cycle_id));

    // Both inputs write one record each, of one shape, in the one loop scope.
    let mut expected = vec![
        from_objective.cycle_id.clone(),
        from_execution.cycle_id.clone(),
    ];
    expected.sort();
    assert_eq!(stored_cycles(&h), expected);
    assert_eq!(h.records(), 0, "a cycle is not a monitor analysis");
    let objective = cycle(&h, &from_objective.cycle_id).await;
    let execution = cycle(&h, &from_execution.cycle_id).await;
    assert_eq!(
        objective.input,
        CycleInputV1::Objective("Stop rediscovering contracts".into())
    );
    assert_eq!(execution.input, CycleInputV1::ExecutionId("exec-1".into()));
    let wire = serde_json::to_value(&execution).unwrap();
    assert_eq!(wire["input"], json!({"execution_id": "exec-1"}));
    for record in [&objective, &execution] {
        assert_eq!(record.schema_version, CYCLE_SCHEMA_VERSION);
        assert_eq!(record.status, CycleStatusV1::Queued);
        assert_eq!(statuses(record), [CycleStatusV1::Queued]);
        assert!(record.result.is_none());
    }

    // Both steps go through the one run queue and advance the same history.
    assert_eq!(h.world().steps.len(), 2);
    h.drain().await;
    for cycle_id in [&from_objective.cycle_id, &from_execution.cycle_id] {
        let record = cycle(&h, cycle_id).await;
        assert_eq!(record.status, CycleStatusV1::Running);
        assert_eq!(
            statuses(&record),
            [CycleStatusV1::Queued, CycleStatusV1::Running]
        );
        assert_eq!(record.step, 1);
    }

    // One list holds both, newest first.
    let listed = runtime::cycle_list(&h.deps, serde_json::from_value(json!({})).unwrap())
        .await
        .unwrap()
        .cycles;
    let mut listed_ids: Vec<_> = listed.iter().map(|cycle| cycle.cycle_id.clone()).collect();
    assert!(listed
        .windows(2)
        .all(|pair| pair[0].created_at >= pair[1].created_at));
    listed_ids.sort();
    assert_eq!(listed_ids, expected);
    let one = runtime::cycle_list(
        &h.deps,
        serde_json::from_value(json!({"limit": 1})).unwrap(),
    )
    .await
    .unwrap();
    assert_eq!(one.cycles.len(), 1);
    assert!(matches!(
        runtime::cycle_list(
            &h.deps,
            serde_json::from_value(json!({"limit": 0})).unwrap()
        )
        .await,
        Err(EvalError::InvalidRequest(_))
    ));

    // Exactly one input: none, both or a blank one start nothing.
    for request in [
        json!({}),
        json!({"objective": "Fix it", "execution_id": "exec-1"}),
        json!({"objective": "   "}),
        json!({"execution_id": ""}),
    ] {
        let refused = start_cycle(&h, request.clone()).await;
        assert!(
            matches!(refused, Err(EvalError::InvalidRequest(_))),
            "{request}: {refused:?}"
        );
    }
    assert_eq!(stored_cycles(&h), expected);
    assert!(h.world().steps.is_empty());
}

#[tokio::test]
async fn cycle_can_be_started_followed_and_canceled() {
    let mut h = Harness::start(World::new()).await;
    let started = start_cycle(&h, json!({"objective": "Shorter plans"}))
        .await
        .unwrap();
    let cycle_id = started.cycle_id;

    // Followed: status and result read the same record; no result yet.
    let queued = runtime::cycle_result(&h.deps, cycle_request(&cycle_id))
        .await
        .unwrap()
        .expect("cycle exists");
    assert_eq!(queued.cycle.status, CycleStatusV1::Queued);
    assert!(queued.result.is_none());
    h.drain().await;
    let running = runtime::cycle_result(&h.deps, cycle_request(&cycle_id))
        .await
        .unwrap()
        .expect("cycle exists");
    assert_eq!(running.cycle, cycle(&h, &cycle_id).await);
    assert_eq!(running.cycle.status, CycleStatusV1::Running);
    assert!(running.result.is_none());
    let step_before_cancel = running.cycle.step;

    // Cancelled: the history records it and a step queued before is skipped.
    let cancelled = runtime::cycle_cancel(&h.deps, cycle_request(&cycle_id))
        .await
        .unwrap();
    assert!(cancelled.cancelled);
    assert_eq!(cancelled.status, CycleStatusV1::Cancelled);
    let record = cycle(&h, &cycle_id).await;
    assert_eq!(record.status, CycleStatusV1::Cancelled);
    assert!(record.completed_at.is_some());
    assert_eq!(
        statuses(&record),
        [
            CycleStatusV1::Queued,
            CycleStatusV1::Running,
            CycleStatusV1::Cancelled
        ]
    );
    let late = runtime::step(
        &h.deps,
        StepRequestV1 {
            evaluation_id: cycle_id.clone(),
            step: step_before_cancel,
        },
    )
    .await
    .unwrap();
    assert!(late.skipped);
    assert_eq!(late.status, EvalStatusV1::Cancelled);
    assert_eq!(cycle(&h, &cycle_id).await, record);
    // Cancelling a finished cycle leaves it as is.
    let again = runtime::cycle_cancel(&h.deps, cycle_request(&cycle_id))
        .await
        .unwrap();
    assert!(!again.cancelled);
    assert_eq!(again.status, CycleStatusV1::Cancelled);
    assert_eq!(cycle(&h, &cycle_id).await, record);

    // A cycle cancelled before its first step never runs.
    let early = start_cycle(&h, json!({"execution_id": "exec-2"}))
        .await
        .unwrap()
        .cycle_id;
    assert!(
        runtime::cycle_cancel(&h.deps, cycle_request(&early))
            .await
            .unwrap()
            .cancelled
    );
    h.drain().await;
    let record = cycle(&h, &early).await;
    assert_eq!(
        statuses(&record),
        [CycleStatusV1::Queued, CycleStatusV1::Cancelled]
    );

    // A lost enqueue is resumed after a restart.
    let lost = start_cycle(&h, json!({"objective": "Resume me"}))
        .await
        .unwrap()
        .cycle_id;
    h.world().steps.clear();
    h.restart();
    runtime::recover(&h.deps).await.unwrap();
    let resumed: Vec<_> = h
        .world()
        .steps
        .iter()
        .map(|step| step["evaluation_id"].clone())
        .collect();
    assert_eq!(resumed, [json!(lost)]);
    h.drain().await;
    assert_eq!(cycle(&h, &lost).await.status, CycleStatusV1::Running);

    // An unknown cycle reads as null and cannot be cancelled.
    let unknown = "cycle_unknown";
    assert!(runtime::cycle_status(&h.deps, cycle_request(unknown))
        .await
        .unwrap()
        .is_none());
    assert!(runtime::cycle_result(&h.deps, cycle_request(unknown))
        .await
        .unwrap()
        .is_none());
    assert!(matches!(
        runtime::cycle_cancel(&h.deps, cycle_request(unknown)).await,
        Err(EvalError::NotFound(_))
    ));
}

#[tokio::test]
async fn legacy_scopes_are_never_read() {
    let mut world = World::new();
    // Old records no current type can read; touching one would fail the call.
    for scope in LEGACY_SCOPES {
        world.state.insert(
            (scope.to_string(), "cycle_legacy".to_string()),
            json!({"legacy": true}),
        );
    }
    let h = Harness::start(world).await;
    h.world().calls.clear();

    let objective = start_cycle(&h, json!({"objective": "Fewer retries"}))
        .await
        .unwrap()
        .cycle_id;
    let execution = start_cycle(&h, json!({"execution_id": "exec-3"}))
        .await
        .unwrap()
        .cycle_id;
    h.drain().await;
    cycle(&h, &objective).await;
    runtime::cycle_result(&h.deps, cycle_request(&execution))
        .await
        .unwrap()
        .expect("cycle exists");
    let listed = runtime::cycle_list(&h.deps, CycleListRequestV1::default())
        .await
        .unwrap();
    assert_eq!(listed.cycles.len(), 2);
    assert!(
        runtime::cycle_status(&h.deps, cycle_request("cycle_legacy"))
            .await
            .unwrap()
            .is_none(),
        "a legacy record is not a cycle"
    );
    runtime::cycle_cancel(&h.deps, cycle_request(&execution))
        .await
        .unwrap();

    // Every call was the loop's own state or its run queue: no legacy scope,
    // no user chat, no session.
    let calls = h.world().calls.clone();
    assert!(!calls.is_empty());
    for (function, payload) in &calls {
        if function == "eval::step" {
            continue;
        }
        assert!(
            function.starts_with("state::"),
            "unexpected call to {function}"
        );
        assert_eq!(
            payload["scope"],
            state::CYCLE_SCOPE,
            "{function} touched {payload}"
        );
    }
    // The legacy records stay inert: neither migrated nor deleted.
    for scope in LEGACY_SCOPES {
        assert_eq!(
            h.world()
                .state
                .get(&(scope.to_string(), "cycle_legacy".to_string())),
            Some(&json!({"legacy": true}))
        );
    }
}

/// The functions `register_all` announces to the engine, in order.
async fn registered_ids() -> Vec<String> {
    let registered = Arc::new(Mutex::new(Vec::new()));
    let seen = registered.clone();
    let engine = fake_engine::start(move |frame: Value| {
        if frame["type"] == "registerfunction" {
            if let Some(id) = frame["id"].as_str() {
                seen.lock().unwrap().push(id.to_string());
            }
        }
        vec![]
    })
    .await;
    let iii = Arc::new(register_worker(&engine.url, InitOptions::default()));
    let deps = Deps::new(iii.clone(), EvalEvents::register(&iii));
    eval::functions::register_all(&iii, &deps);
    let expected = eval::surface::catalog().len();
    tokio::time::timeout(Duration::from_secs(10), async {
        while registered.lock().unwrap().len() < expected {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("every function reaches the fake engine");
    // Anything announced late would still be caught.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let ids = registered.lock().unwrap().clone();
    drop(engine);
    ids
}

#[tokio::test]
async fn discarded_ids_are_not_registered() {
    // Removed with the chat monitor (KAN-461), the one-step replay (KAN-462)
    // and the manual review and validation path (KAN-458).
    const DISCARDED: [&str; 8] = [
        "eval::on-turn-completed",
        "eval::analyze-session",
        "eval::reproduce",
        "eval::attach-validation",
        "eval::start-validation",
        "eval::review",
        "eval::reviews",
        "eval::recurrence",
    ];
    let registered = registered_ids().await;
    let offered: Vec<String> = eval::surface::catalog()
        .iter()
        .map(|spec| spec.function_id.to_string())
        .collect();
    for removed in DISCARDED {
        assert!(
            !registered.iter().any(|id| id == removed),
            "{removed} is registered: {registered:?}"
        );
        assert!(
            !offered.iter().any(|id| id == removed),
            "{removed} is offered: {offered:?}"
        );
    }
    // What is registered is exactly what the catalog describes.
    let (mut registered, mut offered) = (registered, offered);
    registered.sort();
    offered.sort();
    assert_eq!(registered, offered);

    // Nor are their state scopes: a whole analysis, its sweep, recovery and
    // deletion never touch the observation index or the review records.
    let mut h = Harness::start(World::new()).await;
    h.configure(true).await;
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    investigated(&h, &evaluation_id).await;
    runtime::sweep(&h.deps).await.unwrap();
    h.restart();
    runtime::recover(&h.deps).await.unwrap();
    runtime::delete(&h.deps, EvaluationIdRequestV1 { evaluation_id })
        .await
        .unwrap();
    let world = h.world();
    assert!(world
        .calls
        .iter()
        .any(|(function, _)| function.starts_with("state::")));
    for (function, payload) in &world.calls {
        let scope = payload["scope"].as_str().unwrap_or_default();
        assert!(
            scope != "eval_observation" && scope != "eval_suggestion",
            "{function} touched the removed scope {scope}"
        );
    }
    assert!(world
        .state
        .keys()
        .all(|(scope, _)| scope != "eval_observation" && scope != "eval_suggestion"));
}

// ---------------------------------------------------------------------------
// Execution input: terminal E2E executions admitted into the loop (KAN-442)
// ---------------------------------------------------------------------------

/// One execution as the E2E lists it: `extra` fields override the defaults.
fn execution(execution_id: &str, status: &str, extra: Value) -> Value {
    let mut entry = json!({"id": execution_id, "status": status});
    for (field, value) in extra.as_object().cloned().unwrap_or_default() {
        entry[field] = value;
    }
    entry
}

/// Makes `e2e::dashboard::execution-get` answer `entry` for its execution.
fn put_execution(h: &Harness, entry: &Value) {
    let execution_id = entry["id"].as_str().unwrap().to_string();
    h.world().executions.insert(
        execution_id,
        json!({"manifest": {"executions": [entry.clone()]}, "detail": {}}),
    );
}

async fn changed(h: &Harness, event: Value) -> ExecutionAdmissionResponseV1 {
    runtime::execution_changed(&h.deps, serde_json::from_value(event).unwrap())
        .await
        .unwrap()
}

fn admissions(h: &Harness) -> Vec<AdmissionV1> {
    h.world()
        .state
        .iter()
        .filter(|((scope, key), _)| {
            scope == state::ADMISSION_SCOPE && key != state::ADMISSION_CHECKPOINT_KEY
        })
        .map(|(_, value)| serde_json::from_value(value.clone()).unwrap())
        .collect()
}

fn admission_checkpoint(h: &Harness) -> Option<AdmissionCheckpointV1> {
    h.world()
        .state
        .get(&(
            state::ADMISSION_SCOPE.to_string(),
            state::ADMISSION_CHECKPOINT_KEY.to_string(),
        ))
        .map(|value| serde_json::from_value(value.clone()).unwrap())
}

fn execution_gets(h: &Harness, execution_id: &str) -> usize {
    h.world()
        .calls_to("e2e::dashboard::execution-get")
        .iter()
        .filter(|payload| payload["execution_id"] == execution_id)
        .count()
}

#[tokio::test]
async fn terminal_execution_event_admits_one_cycle() {
    let h = Harness::start(World::new()).await;
    put_execution(
        &h,
        &execution(
            "exec-pass",
            "passed",
            json!({"run_id": "run-1", "attempt_id": "att-1"}),
        ),
    );
    put_execution(
        &h,
        &execution("exec-fail", "failed", json!({"kind": "normal"})),
    );

    // A passed execution is admitted into one queued cycle of its own.
    let passed = changed(&h, json!({"execution_id": "exec-pass"})).await;
    assert!(passed.admitted, "{passed:?}");
    let passed_cycle = passed.cycle_id.clone().expect("admitted into a cycle");
    assert!(ids::is_cycle(&passed_cycle));
    assert_eq!(
        h.world().calls_to("e2e::dashboard::execution-get"),
        [json!({"execution_id": "exec-pass"})],
        "execution-get is re-read as the source of truth"
    );
    assert_eq!(stored_cycles(&h), std::slice::from_ref(&passed_cycle));
    let record = cycle(&h, &passed_cycle).await;
    assert_eq!(record.input, CycleInputV1::ExecutionId("exec-pass".into()));
    assert_eq!(record.status, CycleStatusV1::Queued);
    assert_eq!(statuses(&record), [CycleStatusV1::Queued]);
    assert_eq!(h.world().steps.len(), 1);

    // The claim names the execution, its run and its attempt.
    let claims = admissions(&h);
    assert_eq!(claims.len(), 1);
    let claim = &claims[0];
    assert_eq!(claim.execution_id, "exec-pass");
    assert_eq!(claim.run_id.as_deref(), Some("run-1"));
    assert_eq!(claim.attempt_id.as_deref(), Some("att-1"));
    assert_eq!(claim.kind, AdmissionKindV1::Cycle);
    assert_eq!(claim.cycle_id, passed_cycle);
    assert!(h
        .world()
        .calls_to("state::compare-and-set")
        .iter()
        .all(|payload| payload["scope"] == state::ADMISSION_SCOPE));

    // A failed normal execution is admitted too, through the event's `id`.
    let failed = changed(&h, json!({"id": "exec-fail"})).await;
    assert!(failed.admitted, "{failed:?}");
    let failed_cycle = failed.cycle_id.clone().unwrap();
    assert_ne!(failed_cycle, passed_cycle);
    let mut expected = vec![passed_cycle.clone(), failed_cycle.clone()];
    expected.sort();
    assert_eq!(stored_cycles(&h), expected);
    assert_eq!(
        cycle(&h, &failed_cycle).await.input,
        CycleInputV1::ExecutionId("exec-fail".into())
    );
    assert_eq!(admissions(&h).len(), 2);

    // Both go through the loop's run queue like any other cycle.
    assert_eq!(h.world().steps.len(), 2);
    h.drain().await;
    for cycle_id in [&passed_cycle, &failed_cycle] {
        assert_eq!(cycle(&h, cycle_id).await.status, CycleStatusV1::Running);
    }
}

#[tokio::test]
async fn running_or_non_normal_execution_is_not_admitted() {
    let h = Harness::start(World::new()).await;
    for entry in [
        execution("exec-running", "running", json!({})),
        execution("exec-queued", "queued", json!({})),
        execution("exec-cancelled", "cancelled", json!({})),
        execution("exec-replay", "passed", json!({"kind": "replay"})),
    ] {
        put_execution(&h, &entry);
    }
    // The listing answers, but without the execution the event names.
    h.world().executions.insert(
        "exec-unlisted".into(),
        json!({"manifest": {"executions": [execution("other", "passed", json!({}))]}}),
    );

    for execution_id in [
        "exec-running",
        "exec-queued",
        "exec-cancelled",
        "exec-replay",
        "exec-unlisted",
    ] {
        // The event's own status is only a hint: execution-get decides.
        let response = changed(
            &h,
            json!({"execution_id": execution_id, "status": "passed"}),
        )
        .await;
        assert!(!response.admitted, "{execution_id}: {response:?}");
        assert_eq!(response.cycle_id, None, "{execution_id}");
        assert!(!response.reason.is_empty(), "{execution_id}");
        assert_eq!(execution_gets(&h, execution_id), 1, "{execution_id}");
    }
    assert!(stored_cycles(&h).is_empty());
    assert!(admissions(&h).is_empty(), "nothing is claimed");
    assert!(h.world().steps.is_empty());
    assert!(h.world().calls_to("state::compare-and-set").is_empty());

    // Once the running execution finishes, its next event admits it.
    put_execution(&h, &execution("exec-running", "passed", json!({})));
    let finished = changed(&h, json!({"execution_id": "exec-running"})).await;
    assert!(finished.admitted, "{finished:?}");
    assert_eq!(stored_cycles(&h), [finished.cycle_id.unwrap()]);
}

#[tokio::test]
async fn redelivery_and_restart_do_not_duplicate_analysis() {
    let mut h = Harness::start(World::new()).await;
    put_execution(
        &h,
        &execution(
            "exec-r",
            "passed",
            json!({"run_id": "run-r", "attempt_id": "att-1"}),
        ),
    );
    let event = json!({"execution_id": "exec-r"});
    let first = changed(&h, event.clone()).await;
    assert!(first.admitted);
    let cycle_id = first.cycle_id.clone().unwrap();
    let created = cycle(&h, &cycle_id).await;

    // Redelivered: the claim is found and nothing new is created or queued.
    for _ in 0..2 {
        let again = changed(&h, event.clone()).await;
        assert!(!again.admitted, "{again:?}");
        assert_eq!(again.reason, "already admitted");
        assert_eq!(again.cycle_id.as_deref(), Some(cycle_id.as_str()));
    }
    // Restarted: a fresh process on the same storage finds the same claim.
    h.restart();
    let after_restart = changed(&h, event.clone()).await;
    assert!(!after_restart.admitted);
    assert_eq!(after_restart.cycle_id.as_deref(), Some(cycle_id.as_str()));
    assert_eq!(stored_cycles(&h), std::slice::from_ref(&cycle_id));
    assert_eq!(
        cycle(&h, &cycle_id).await,
        created,
        "the cycle is untouched"
    );
    assert_eq!(admissions(&h).len(), 1);
    assert_eq!(h.world().steps.len(), 1, "one step only");

    // A restart between the claim and the cycle: the claim survived, the
    // cycle and its step were lost. Redelivery completes it under the claimed
    // id, once.
    {
        let mut world = h.world();
        world
            .state
            .remove(&(state::CYCLE_SCOPE.to_string(), cycle_id.clone()));
        world.steps.clear();
    }
    h.restart();
    let completed = changed(&h, event.clone()).await;
    assert!(!completed.admitted, "the claim was already set");
    assert_eq!(completed.cycle_id.as_deref(), Some(cycle_id.as_str()));
    assert_eq!(stored_cycles(&h), std::slice::from_ref(&cycle_id));
    assert_eq!(
        cycle(&h, &cycle_id).await.input,
        CycleInputV1::ExecutionId("exec-r".into())
    );
    assert_eq!(h.world().steps.len(), 1);
    changed(&h, event.clone()).await;
    assert_eq!(stored_cycles(&h), std::slice::from_ref(&cycle_id));
    assert_eq!(h.world().steps.len(), 1);
    assert_eq!(admissions(&h).len(), 1);

    // Another attempt of the same execution is another admission.
    put_execution(
        &h,
        &execution(
            "exec-r",
            "failed",
            json!({"run_id": "run-r", "attempt_id": "att-2"}),
        ),
    );
    let retried = changed(&h, event).await;
    assert!(retried.admitted, "{retried:?}");
    assert_ne!(retried.cycle_id.as_deref(), Some(cycle_id.as_str()));
    assert_eq!(admissions(&h).len(), 2);
    assert_eq!(stored_cycles(&h).len(), 2);
}

#[tokio::test]
async fn missed_event_is_recovered_by_sweep_checkpoint() {
    let mut h = Harness::start(World::new()).await;
    let early = execution("exec-a", "passed", json!({"finished_at": 1_000}));
    let open = execution("exec-b", "running", json!({"started_at": 2_000}));
    let late = execution("exec-c", "failed", json!({"finished_at": 3_000}));
    for entry in [&early, &open, &late] {
        put_execution(&h, entry);
    }

    // The E2E is down: the sweep still succeeds and moves no checkpoint.
    runtime::sweep(&h.deps).await.unwrap();
    assert!(stored_cycles(&h).is_empty());
    assert_eq!(admission_checkpoint(&h), None);

    // No event ever arrived: the sweep admits both terminal executions and
    // its checkpoint stops before the one still running.
    h.world().executions_list = Some(json!({"executions": [early, open, late]}));
    runtime::sweep(&h.deps).await.unwrap();
    let cycles = stored_cycles(&h);
    assert_eq!(cycles.len(), 2);
    let mut admitted: Vec<_> = admissions(&h)
        .into_iter()
        .map(|claim| claim.execution_id)
        .collect();
    admitted.sort();
    assert_eq!(admitted, ["exec-a", "exec-c"]);
    assert_eq!(execution_gets(&h, "exec-b"), 0, "an open execution waits");
    assert_eq!(admission_checkpoint(&h).unwrap().since, 2_000);
    assert_eq!(h.world().steps.len(), 2);

    // After a restart, an event naming no execution runs the same sweep: what
    // is behind the checkpoint is not read again, nothing is admitted twice.
    h.restart();
    let swept = changed(&h, json!({})).await;
    assert!(!swept.admitted, "{swept:?}");
    assert_eq!(execution_gets(&h, "exec-a"), 1, "behind the checkpoint");
    assert_eq!(stored_cycles(&h), cycles);
    assert_eq!(admissions(&h).len(), 2);
    assert_eq!(h.world().steps.len(), 2);
    assert_eq!(admission_checkpoint(&h).unwrap().since, 2_000);

    // The running execution finishes and its event is lost too.
    let finished = execution("exec-b", "passed", json!({"finished_at": 4_000}));
    put_execution(&h, &finished);
    h.world().executions_list = Some(json!({"executions": [early, finished, late]}));
    runtime::sweep(&h.deps).await.unwrap();
    assert_eq!(stored_cycles(&h).len(), 3);
    assert_eq!(admissions(&h).len(), 3);
    assert_eq!(h.world().steps.len(), 3);
    assert_eq!(admission_checkpoint(&h).unwrap().since, 4_000);
    runtime::sweep(&h.deps).await.unwrap();
    assert_eq!(stored_cycles(&h).len(), 3, "a later sweep admits nothing");
}

#[tokio::test]
async fn candidate_validation_execution_returns_to_original_cycle() {
    let h = Harness::start(World::new()).await;
    let origin = start_cycle(&h, json!({"objective": "Fewer rediscoveries"}))
        .await
        .unwrap()
        .cycle_id;
    h.drain().await;
    let before = cycle(&h, &origin).await;
    put_execution(
        &h,
        &execution(
            "exec-val",
            "passed",
            json!({"run_id": "run-v", "eval_cycle_id": origin}),
        ),
    );
    put_execution(
        &h,
        &execution(
            "exec-val-meta",
            "failed",
            json!({"metadata": {"cycle_id": origin}}),
        ),
    );

    // The validation run returns to its cycle and starts no other.
    let validated = changed(&h, json!({"execution_id": "exec-val"})).await;
    assert!(validated.admitted, "{validated:?}");
    assert_eq!(validated.cycle_id.as_deref(), Some(origin.as_str()));
    assert_eq!(stored_cycles(&h), std::slice::from_ref(&origin));
    assert!(h.world().steps.is_empty(), "no new cycle is queued");
    let claims = admissions(&h);
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].kind, AdmissionKindV1::CandidateValidation);
    assert_eq!(claims[0].cycle_id, origin);
    let record = cycle(&h, &origin).await;
    assert_eq!(record.input, before.input);
    assert_eq!(record.status, before.status);
    assert_eq!(record.history.len(), before.history.len() + 1);
    let returned = record.history.last().unwrap().detail.clone().unwrap();
    assert!(returned.contains("exec-val"), "{returned}");
    assert!(returned.contains("run-v"), "{returned}");
    assert!(returned.ends_with("passed"), "{returned}");

    // Redelivered, it is recorded once.
    let again = changed(&h, json!({"execution_id": "exec-val"})).await;
    assert!(!again.admitted);
    assert_eq!(again.cycle_id.as_deref(), Some(origin.as_str()));
    assert_eq!(cycle(&h, &origin).await, record);

    // The cycle id carried in the metadata is recognized the same way.
    let meta = changed(&h, json!({"execution_id": "exec-val-meta"})).await;
    assert!(meta.admitted, "{meta:?}");
    assert_eq!(meta.cycle_id.as_deref(), Some(origin.as_str()));
    assert_eq!(stored_cycles(&h), std::slice::from_ref(&origin));
    assert_eq!(
        cycle(&h, &origin).await.history.len(),
        record.history.len() + 1
    );

    // A validation of a cycle eval does not know starts nothing.
    put_execution(
        &h,
        &execution(
            "exec-stray",
            "passed",
            json!({"eval_cycle_id": "cycle_unknown"}),
        ),
    );
    let stray = changed(&h, json!({"execution_id": "exec-stray"})).await;
    assert!(!stray.admitted, "{stray:?}");
    assert_eq!(stray.cycle_id, None);
    assert_eq!(stored_cycles(&h), [origin]);
    assert_eq!(admissions(&h).len(), 2);
    assert!(h.world().steps.is_empty());
}

// ---------------------------------------------------------------------------
// Durable E2E evidence and the deterministic first pass (KAN-459)
// ---------------------------------------------------------------------------

/// A native `execution-get` report captured from the E2E.
fn captured_report(name: &str) -> Value {
    let text = match name {
        "passed" => include_str!("fixtures/e2e/execution-passed.json"),
        "failed-infra" => include_str!("fixtures/e2e/execution-failed-infra.json"),
        "sparse" => include_str!("fixtures/e2e/execution-sparse.json"),
        other => panic!("no captured report {other}"),
    };
    serde_json::from_str(text).unwrap()
}

/// The captured passed report with the root transcript it preserved.
fn passed_report() -> Value {
    let mut report = captured_report("passed");
    report["transcripts"] = json!([{"session_id": ROOT, "entries": rediscovery_transcript()}]);
    report
}

/// Serves `report` as the `execution-get` answer of the execution it lists.
fn put_report(h: &Harness, report: &Value) -> String {
    let execution_id = report["manifest"]["executions"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();
    h.world()
        .executions
        .insert(execution_id.clone(), report.clone());
    execution_id
}

fn stored_evidence(h: &Harness) -> Vec<ExecutionEvidenceV1> {
    h.world()
        .state
        .iter()
        .filter(|((scope, _), _)| scope == state::EVIDENCE_SCOPE)
        .map(|(_, value)| serde_json::from_value(value.clone()).unwrap())
        .collect()
}

fn evidence_of(h: &Harness, execution_id: &str) -> ExecutionEvidenceV1 {
    let mut found: Vec<_> = stored_evidence(h)
        .into_iter()
        .filter(|evidence| evidence.execution_id == execution_id)
        .collect();
    assert_eq!(found.len(), 1, "one stored copy of {execution_id}");
    found.remove(0)
}

fn classes(evidence: &ExecutionEvidenceV1) -> Vec<FindingClassV1> {
    let mut classes: Vec<_> = evidence
        .findings
        .iter()
        .map(|finding| finding.class)
        .collect();
    classes.sort();
    classes.dedup();
    classes
}

fn signals(evidence: &ExecutionEvidenceV1) -> Vec<String> {
    let mut signals: Vec<_> = evidence
        .findings
        .iter()
        .map(|finding| finding.signal.clone())
        .collect();
    signals.sort();
    signals.dedup();
    signals
}

/// Calls that reach a model, the analyst or a new E2E campaign.
fn model_or_campaign_calls(h: &Harness, from: usize) -> Vec<String> {
    h.world().calls[from..]
        .iter()
        .map(|(function, _)| function.clone())
        .filter(|function| {
            function.starts_with("harness::")
                || function.starts_with("judge::")
                || function.starts_with("router::")
                || function == "e2e::dashboard::execution-start"
        })
        .collect()
}

fn evidence_writes(h: &Harness) -> usize {
    h.world()
        .calls
        .iter()
        .filter(|(function, payload)| {
            function.starts_with("state::") && payload["scope"] == state::EVIDENCE_SCOPE
        })
        .filter(|(function, _)| function != "state::get")
        .count()
}

#[tokio::test]
async fn analysis_survives_teardown_and_retention_from_imported_reports() {
    let mut h = Harness::start(World::new()).await;
    let report = passed_report();
    let execution_id = put_report(&h, &report);
    let admitted = changed(&h, json!({"execution_id": execution_id})).await;
    assert!(admitted.admitted, "{admitted:?}");

    // The native report is copied whole into the loop scope, keyed by the
    // admission key, with its digest and the execution's identity.
    let claim = admissions(&h).remove(0);
    let imported = evidence_of(&h, "exec-cap");
    assert_eq!(imported.admission_key, claim.admission_key);
    assert!(h.world().state.contains_key(&(
        state::EVIDENCE_SCOPE.to_string(),
        claim.admission_key.clone()
    )));
    assert_eq!(imported.bundle, report);
    assert_eq!(imported.bundle_digest, ids::sha256_json(&report));
    let manifest = &report["manifest"];
    let identity = &imported.identity;
    assert_eq!(
        identity.scenario_digest,
        Some(ids::sha256_json(&manifest["scenario"]))
    );
    assert_eq!(
        identity.contract_digest,
        Some(ids::sha256_json(&manifest["contract"]))
    );
    assert_eq!(
        identity.artifacts_digest,
        Some(ids::sha256_json(&manifest["artifacts"]))
    );
    assert_eq!(
        identity.stack_digest,
        Some(ids::sha256_json(&manifest["stack"]))
    );
    assert_eq!(identity.model.as_deref(), Some("task-model"));
    assert_eq!(identity.provider.as_deref(), Some("p"));
    assert_eq!(identity.revision.as_deref(), Some("4f2c9e1"));
    assert!(imported.analyzed);
    assert!(signals(&imported).contains(&"repeated_contract_discovery".to_string()));

    // Teardown and retention: the containers are gone and the E2E no longer
    // has the execution. The stored copy and its analysis are unaffected.
    {
        let mut world = h.world();
        world.executions.clear();
        world.executions_down = true;
    }
    h.restart();
    assert_eq!(evidence_of(&h, "exec-cap"), imported);

    // Copied before retention, analyzed after it: the first pass reads the
    // stored copy only, never what the source still has.
    let h = Harness::start(World::new()).await;
    let admission_key = ids::sha256_json(&("exec-cap", Some("run-cap"), Some("att-1")));
    let copy = ExecutionEvidenceV1 {
        admission_key: admission_key.clone(),
        execution_id: "exec-cap".into(),
        imported_at: 1,
        bundle_digest: ids::sha256_json(&report),
        bundle: report.clone(),
        identity: EvidenceIdentityV1::default(),
        analyzed: false,
        findings: Vec::new(),
        unknowns: Vec::new(),
        child_transcripts_complete: false,
        deepen: false,
    };
    h.world().state.insert(
        (state::EVIDENCE_SCOPE.to_string(), admission_key.clone()),
        serde_json::to_value(&copy).unwrap(),
    );
    // What the source keeps after retention: no transcript, no coverage.
    let mut retained = captured_report("passed");
    retained.as_object_mut().unwrap().remove("coverage");
    put_report(&h, &retained);
    let calls_before = h.world().calls.len();
    let late = changed(&h, json!({"execution_id": "exec-cap"})).await;
    assert!(late.admitted, "{late:?}");
    let analyzed = evidence_of(&h, "exec-cap");
    assert_eq!(analyzed.admission_key, admission_key);
    assert_eq!(analyzed.bundle, report, "the first copy is kept");
    assert_eq!(analyzed.imported_at, 1);
    assert!(analyzed.analyzed);
    assert_eq!(
        signals(&analyzed),
        ["coverage_gap", "repeated_contract_discovery"],
        "findings come from the stored transcript and coverage"
    );
    assert!(analyzed.deepen);
    assert_eq!(
        model_or_campaign_calls(&h, calls_before),
        Vec::<String>::new()
    );
}

#[tokio::test]
async fn first_pass_classifies_without_model_calls() {
    let h = Harness::start(World::new()).await;
    let calls_before = h.world().calls.len();
    put_report(&h, &passed_report());
    put_report(&h, &captured_report("failed-infra"));
    put_execution(
        &h,
        &execution(
            "exec-defect",
            "failed",
            json!({"failure": {"kind": "assertion mismatch: no receipt in the reply"}}),
        ),
    );
    put_execution(&h, &execution("exec-mute", "failed", json!({})));
    put_report(
        &h,
        &json!({
            "manifest": {"executions": [execution("exec-clean", "passed", json!({}))]},
            "detail": {},
            "transcripts": [{"session_id": CHILD, "entries": [
                user("e_spawn_1", "Inspect the profile."),
                assistant_text("e_t_child_0_assistant", "Done."),
            ]}],
        }),
    );
    for execution_id in [
        "exec-cap",
        "exec-infra",
        "exec-defect",
        "exec-mute",
        "exec-clean",
    ] {
        let response = changed(&h, json!({"execution_id": execution_id})).await;
        assert!(response.admitted, "{execution_id}: {response:?}");
    }

    // A diagnostics signal and a coverage gap: pertinent, so deepening is flagged.
    let cap = evidence_of(&h, "exec-cap");
    assert_eq!(
        classes(&cap),
        [
            FindingClassV1::ImprovementOpportunity,
            FindingClassV1::EvaluationGap
        ]
    );
    let gap = cap
        .findings
        .iter()
        .find(|finding| finding.class == FindingClassV1::EvaluationGap)
        .unwrap();
    assert_eq!(gap.signal, "coverage_gap");
    assert!(gap.observation.contains("never asserted"), "{gap:?}");
    assert!(cap.deepen);

    // A failure the environment caused: not pertinent, no deepening.
    let infra = evidence_of(&h, "exec-infra");
    assert_eq!(
        classes(&infra),
        [FindingClassV1::InfrastructureOrEvaluatorFailure]
    );
    assert!(infra.findings[0].observation.contains("teardown timeout"));
    assert!(!infra.deepen);

    // A failure the agent caused is a defect.
    let defect = evidence_of(&h, "exec-defect");
    assert_eq!(classes(&defect), [FindingClassV1::Defect]);
    assert!(defect.deepen);

    // A failure with no cause and nothing to read is insufficient evidence.
    let mute = evidence_of(&h, "exec-mute");
    assert_eq!(classes(&mute), [FindingClassV1::InsufficientEvidence]);
    assert!(!mute.deepen);

    // A pass whose transcript shows nothing wrong is expected behaviour.
    let clean = evidence_of(&h, "exec-clean");
    assert_eq!(classes(&clean), [FindingClassV1::ExpectedBehavior]);
    assert!(!clean.deepen);

    for evidence in [&cap, &infra, &defect, &mute, &clean] {
        assert!(evidence.analyzed, "{}", evidence.execution_id);
        assert_eq!(
            evidence.deepen,
            evidence
                .findings
                .iter()
                .any(|finding| finding.class.is_pertinent()),
            "{}",
            evidence.execution_id
        );
    }
    // The first pass called no model, no analyst and started no campaign.
    assert_eq!(
        model_or_campaign_calls(&h, calls_before),
        Vec::<String>::new()
    );
    assert!(h.world().calls_to("judge::evaluate").is_empty());
    assert!(h.world().calls_to("harness::send").is_empty());
    assert!(h
        .world()
        .calls_to("e2e::dashboard::execution-start")
        .is_empty());
}

#[tokio::test]
async fn missing_information_stays_unknown() {
    let h = Harness::start(World::new()).await;
    put_report(&h, &captured_report("sparse"));
    assert!(
        changed(&h, json!({"execution_id": "exec-sparse"}))
            .await
            .admitted
    );
    let sparse = evidence_of(&h, "exec-sparse");
    assert_eq!(sparse.identity, EvidenceIdentityV1::default());
    for field in [
        "scenario",
        "contract",
        "artifacts",
        "stack",
        "model",
        "provider",
        "revision",
        "transcripts",
        "child transcripts",
    ] {
        assert!(
            sparse
                .unknowns
                .iter()
                .any(|unknown| unknown.starts_with(&format!("{field}:"))),
            "{field} is unknown: {:?}",
            sparse.unknowns
        );
    }
    assert!(!sparse.child_transcripts_complete);
    // A pass with nothing preserved is not taken as expected behaviour.
    assert_eq!(classes(&sparse), [FindingClassV1::InsufficientEvidence]);
    assert!(!sparse.deepen);

    // Only the root transcript was preserved: child coverage stays unknown.
    put_report(&h, &passed_report());
    changed(&h, json!({"execution_id": "exec-cap"})).await;
    let partial = evidence_of(&h, "exec-cap");
    assert!(!partial.child_transcripts_complete);
    assert!(partial
        .unknowns
        .iter()
        .any(|unknown| unknown.starts_with("child transcripts:")));
    assert!(!partial
        .unknowns
        .iter()
        .any(|unknown| unknown.starts_with("scenario:") || unknown.starts_with("model:")));

    // Full coverage only when the report itself says every child transcript
    // was preserved.
    let mut complete = passed_report();
    complete["manifest"]["executions"][0]["id"] = json!("exec-full");
    complete["coverage"]["child_transcripts"] = json!("complete");
    put_report(&h, &complete);
    changed(&h, json!({"execution_id": "exec-full"})).await;
    let full = evidence_of(&h, "exec-full");
    assert!(full.child_transcripts_complete);
    assert!(!full
        .unknowns
        .iter()
        .any(|unknown| unknown.starts_with("child transcripts:")));

    // A completeness claim without any transcript is not believed.
    let mut claimed = captured_report("sparse");
    claimed["manifest"]["executions"][0]["id"] = json!("exec-claimed");
    claimed["child_transcripts_complete"] = json!(true);
    put_report(&h, &claimed);
    changed(&h, json!({"execution_id": "exec-claimed"})).await;
    assert!(!evidence_of(&h, "exec-claimed").child_transcripts_complete);
}

#[tokio::test]
async fn reimport_does_not_duplicate_findings() {
    let mut h = Harness::start(World::new()).await;
    // The same transcript bundled twice yields each finding once.
    let mut report = passed_report();
    let transcript = report["transcripts"][0].clone();
    report["transcripts"] = json!([transcript.clone(), transcript]);
    put_report(&h, &report);
    let event = json!({"execution_id": "exec-cap"});
    assert!(changed(&h, event.clone()).await.admitted);
    let first = evidence_of(&h, "exec-cap");
    let mut fingerprints: Vec<_> = first
        .findings
        .iter()
        .map(|finding| finding.fingerprint.clone())
        .collect();
    let found = fingerprints.len();
    fingerprints.sort();
    fingerprints.dedup();
    assert_eq!(fingerprints.len(), found, "{:?}", first.findings);
    assert_eq!(
        signals(&first),
        ["coverage_gap", "repeated_contract_discovery"]
    );
    let writes = evidence_writes(&h);

    // Redelivered and restarted: the analyzed copy is neither re-imported
    // nor re-analyzed, even if the source changed meanwhile.
    for _ in 0..2 {
        assert!(!changed(&h, event.clone()).await.admitted);
    }
    h.restart();
    let mut changed_source = report.clone();
    changed_source["coverage"]["gaps"] = json!(["another gap", "yet another gap"]);
    put_report(&h, &changed_source);
    assert!(!changed(&h, event.clone()).await.admitted);
    assert_eq!(evidence_of(&h, "exec-cap"), first);
    assert_eq!(stored_evidence(&h).len(), 1);
    assert_eq!(evidence_writes(&h), writes, "nothing is written again");

    // A restart between the copy and its analysis: redelivery analyzes the
    // stored copy once, with the same findings.
    {
        let mut pending = first.clone();
        pending.analyzed = false;
        pending.findings.clear();
        pending.unknowns.clear();
        pending.deepen = false;
        h.world().state.insert(
            (
                state::EVIDENCE_SCOPE.to_string(),
                first.admission_key.clone(),
            ),
            serde_json::to_value(&pending).unwrap(),
        );
    }
    h.restart();
    assert!(!changed(&h, event.clone()).await.admitted);
    let reanalyzed = evidence_of(&h, "exec-cap");
    assert_eq!(reanalyzed, first);
    changed(&h, event).await;
    assert_eq!(evidence_of(&h, "exec-cap"), first);
    assert_eq!(stored_evidence(&h).len(), 1);
}

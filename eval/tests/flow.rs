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
use eval::{ids, review, state, validation};
use harness::functions::metrics::{SessionMetricsResponseV1, SessionUsageTotalsV1};
use harness::types::content::ContentBlock;
use harness::types::event::StopReason;
use harness::types::message::{
    empty_assistant, AgentMessage, FunctionResultMessage, FunctionResultRoleTag,
};
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
    /// The cost every `router::complete` reply reports; `None` is unknown.
    sample_cost_usd: Option<f64>,
    /// Every `router::complete` call fails, as when the bus gives up on a
    /// provider that may still bill.
    samples_fail: bool,
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
            sample_cost_usd: Some(0.001),
            samples_fail: false,
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
            // Context assembly that changes nothing, and a provider count.
            "context::assemble" => json!({
                "system_prompt": data["system_prompt"], "messages": data["messages"],
                "token_count": 100, "usable": 100_000, "effective_max_output_tokens": 1_000,
                "applied": {}
            }),
            "router::count_tokens" => json!({"tokens": 100, "estimator": "provider",
                "model": data["model"], "provider": "p"}),
            // A model that re-reads the contract only while the registry
            // notice is in its context.
            "router::complete" => {
                if self.samples_fail {
                    return Some(Err("the provider never answered".into()));
                }
                let notice = data["messages"].to_string().contains("registry changed");
                let mut reply = empty_assistant("p", "task-model");
                reply.content = if notice {
                    vec![ContentBlock::FunctionCall {
                        id: "c3".into(),
                        function_id: "agent_trigger".into(),
                        arguments: json!({"function": INFO, "description": "re-read",
                            "payload": {"function_id": "crm::profile"}}),
                    }]
                } else {
                    vec![ContentBlock::text("Scheduled once; receipt R-1.")]
                };
                let mut usage = json!({"input": 12, "output": 4});
                if let Some(cost) = self.sample_cost_usd {
                    usage["cost_usd"] = json!(cost);
                }
                json!({"message": reply, "provider": "p", "model": "task-model", "usage": usage})
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
        let mut deps = Deps::new(iii.clone(), EvalEvents::register(&iii));
        // Whoever runs the tests is not an author the assertions know.
        deps.host_user = None;
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
        self.deps.host_user = None;
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
    let EvalResultResponseV1 { record, assets, .. } = h.result(&third.evaluation_id).await;
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
    let EvalResultResponseV1 { record, assets, .. } = h.result(&evaluation_id).await;
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
        "the default Jev answer is needs_investigation"
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
    assert_eq!(rejection.reason, RejectionReasonV1::AtCapacity);
    let triage = monitor.triage.unwrap();
    assert!(triage.available);
    assert_eq!(triage.models, ["jev-test-1"]);
    assert!(!h.world().calls_to("judge::models::list").is_empty());
}

const SCENARIO: &str = "tool_contract_recovery";

/// An analysis that finished with one suggestion whose plan names `SCENARIO`.
async fn analyzed_with_suggestion(mut world: World) -> (Harness, String) {
    let mut planned = suggestion(&format!("e_{TURN}_c2"));
    planned["validation"]["scenario_id"] = json!(SCENARIO);
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
                    "github::pr::list", "engine::functions::info"],
                "deny": ["eval::*", "e2e::dashboard::execution-*"],
                "expose": "agent_trigger"
            })
        );
        let prompt = send["options"]["system_prompt"].as_str().unwrap();
        for id in [
            "coder::search",
            "coder::tree",
            "coder::read-file",
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
async fn the_daily_cost_cap_stops_automatic_admission_and_only_that() {
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
    // Still under the cap: automatic observation admits another turn.
    assert_eq!(
        h.end_turn(ROOT, "t_before").await.outcome,
        WakeOutcomeV1::Admitted
    );

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

    // Capped: a new turn is not admitted, and the reason is recorded.
    let records = h.records();
    let refused = h.end_turn(ROOT, "t_next").await;
    assert_eq!(refused.outcome, WakeOutcomeV1::CostCap);
    assert_eq!(refused.evaluation_id, None);
    assert_eq!(h.records(), records);
    let rejection = runtime::monitor_state(&h.deps, MonitorStateRequestV1::default())
        .await
        .unwrap()
        .last_rejection
        .unwrap();
    assert_eq!(
        (rejection.turn_id.as_str(), rejection.reason),
        ("t_next", RejectionReasonV1::CostCap)
    );
    // A redelivered event of an admitted turn is still reused, not refused.
    assert_eq!(h.end_turn(ROOT, TURN).await.outcome, WakeOutcomeV1::Reused);
    // A manual analysis is the user's choice: never refused by the cap.
    let manual = reanalyze(&h).await;
    assert!(manual.starts_with("eval_"));

    // Raising the cap resumes automatic observation.
    configure_capped(&h, json!(5)).await.unwrap();
    assert!(!cost_block(&h).await.capped);
    assert_eq!(
        h.end_turn(ROOT, "t_next").await.outcome,
        WakeOutcomeV1::Admitted
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
    assert_eq!(
        h.end_turn(ROOT, "t_after").await.outcome,
        WakeOutcomeV1::CostCap
    );
    // A manual analysis is still the user's choice.
    assert!(reanalyze(&h).await.starts_with("eval_"));
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
async fn admission_records_the_harness_version_and_collection_the_signals() {
    let h = Harness::start(World::new()).await;
    h.configure(true).await;
    // Another namespace runs another Harness: only this one's counts.
    let namespace = h.iii.namespace().unwrap_or_else(|| "default".into());
    h.world().workers = Some(json!({"workers": [
        {"name": "harness", "namespace": "elsewhere", "version": "9.9.9"},
        {"name": "eval", "namespace": namespace, "version": "0.2.16"},
        {"name": "harness", "namespace": namespace, "version": "1.8.42"},
    ]}));
    let evaluation_id = h.end_turn(ROOT, TURN).await.evaluation_id.unwrap();
    let admitted = h.result(&evaluation_id).await.record;
    assert_eq!(admitted.harness_version.as_deref(), Some("1.8.42"));
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

    // An engine that cannot list its workers never blocks an admission.
    h.world().workers = None;
    runtime::cancel(
        &h.deps,
        EvaluationIdRequestV1 {
            evaluation_id: evaluation_id.clone(),
        },
    )
    .await
    .unwrap();
    let again = reanalyze(&h).await;
    let record = h.result(&again).await.record;
    assert_eq!(record.harness_version, None);
    assert!(serde_json::to_value(&record)
        .unwrap()
        .get("harness_version")
        .is_none());
}

#[tokio::test]
async fn plan_cohorts_fill_each_scenarios_own_measures() {
    let mut world = World::new();
    world.executions.insert(
        "plan-base".into(),
        json!({"manifest": {"executions": [{"id": "plan-base", "status": "completed"}]},
            "detail": {
                "scenario_metrics": [{"scenario_id": SCENARIO, "run_count": 2.0,
                    "behavior_sha256": "sha256:aa", "contract_fingerprint": "fnv1a32:1",
                    "averages": {"tokens": 6513.5}, "samples": {"tokens": 2.0}}],
                "plan_execution": {"measurements": {"cohorts": [
                    {"scenario_id": SCENARIO,
                     "aggregate": {"completed_runs": 2, "planned_runs": 3.0, "pass_rate": 0.5,
                         "mean_score": 80.5, "cost": {"total_usd": 0.5},
                         "total_tokens_consumed": 1000,
                         "robustness": {"median_wall_time_ms": 1500.0}},
                     "consumption": {"p50_function_calls": 7.0}},
                    {"scenario_id": "another", "aggregate": {"pass_rate": 0.0}}]}}}}),
    );
    world.executions.insert(
        "plan-cand".into(),
        json!({"manifest": {"executions": [{"id": "plan-cand", "status": "completed"}]},
            "detail": {"scenario_metrics": [{"scenario_id": SCENARIO, "run_count": 1}]}}),
    );
    let (h, evaluation_id) = analyzed_with_suggestion(world).await;
    let preview = runtime::attach_validation(
        &h.deps,
        serde_json::from_value(
            json!({"evaluation_id": evaluation_id, "suggestion_index": 0,
            "baseline_execution_id": "plan-base", "candidate_execution_id": "plan-cand",
            "dry_run": true}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
    let scenario = &preview.link.baseline.scenarios[0];
    assert_eq!(scenario.pass_rate, Some(0.5));
    assert_eq!(scenario.mean_score, Some(80.5));
    assert_eq!(
        (scenario.completed_runs, scenario.planned_runs),
        (Some(2), Some(3))
    );
    assert_eq!(scenario.cost_usd_per_run, Some(0.25));
    assert_eq!(scenario.total_tokens_per_run, Some(500.0));
    assert_eq!(scenario.median_wall_time_ms, Some(1500.0));
    assert_eq!(scenario.p50_function_calls, Some(7.0));
    // What the execution always reported is kept.
    assert_eq!(scenario.run_count, 2);
    assert_eq!(scenario.measures["tokens"].average, Some(6513.5));
    // Without a cohort nothing is invented.
    let bare = serde_json::to_value(&preview.link.candidate.scenarios[0]).unwrap();
    for field in [
        "pass_rate",
        "mean_score",
        "completed_runs",
        "planned_runs",
        "cost_usd_per_run",
        "total_tokens_per_run",
        "median_wall_time_ms",
        "p50_function_calls",
    ] {
        assert!(bare.get(field).is_none(), "{field}: {bare}");
    }
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
// Review, validation start and recurrence
// ---------------------------------------------------------------------------

const PATTERN: &str = "repeated_contract_discovery:engine::functions::info";

fn git_in(dir: &std::path::Path, args: &[&str]) -> String {
    let output = std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@t",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_string()
}

/// A clone of the workers repository: `main` and `feat` are on the remote
/// (as remote-tracking branches), `local` only here.
struct Repo {
    dir: std::path::PathBuf,
    base: String,
    feat: String,
}

impl Repo {
    /// `origin_main: false` leaves the clone without the branch the default
    /// baseline is taken from.
    fn new(origin_main: bool) -> Self {
        let dir = std::env::temp_dir().join(format!("eval-git-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir_all(&dir).unwrap();
        git_in(&dir, &["init", "-q"]);
        let commit = |name: &str| {
            std::fs::write(dir.join("file.txt"), name).unwrap();
            git_in(&dir, &["add", "."]);
            git_in(&dir, &["commit", "-q", "-m", name]);
            git_in(&dir, &["rev-parse", "HEAD"])
        };
        let base = commit("base");
        git_in(&dir, &["checkout", "-q", "-b", "feat"]);
        let feat = commit("feat");
        git_in(&dir, &["checkout", "-q", "-b", "local"]);
        commit("local");
        git_in(&dir, &["update-ref", "refs/remotes/origin/feat", &feat]);
        if origin_main {
            git_in(&dir, &["update-ref", "refs/remotes/origin/main", &base]);
            git_in(
                &dir,
                &[
                    "symbolic-ref",
                    "refs/remotes/origin/HEAD",
                    "refs/remotes/origin/main",
                ],
            );
        }
        git_in(&dir, &["branch", "-q", "main", &base]);
        Self { dir, base, feat }
    }

    fn path(&self) -> String {
        self.dir.to_string_lossy().into_owned()
    }
}

impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

const HARNESS_STACK: &str =
    "# the harness template\niii: latest\ntemplate: harness\ncontainers:\n  \
    harness:\n    worker: package://harness\n    version: latest\n  harness-e2e:\n    worker: \
    package://harness-e2e\n    version: 0.17.2\nstartup_timeout: 5m\n";

fn stacks() -> Value {
    json!({"stacks": [
        {"id": "default", "template": null, "yaml": "iii: latest\ncontainers: {}\n"},
        {"id": "harness-template", "template": "harness", "yaml": HARNESS_STACK}]})
}

fn clean_transcript() -> Vec<Value> {
    vec![
        user("e_idem_task", "Schedule the follow-up."),
        info_call(0, "c1"),
        info_result(
            "c1",
            json!({"function_id": "crm::profile", "request_schema": {"type": "object"}}),
        ),
        assistant_text(&format!("e_{TURN}_2_assistant"), "Scheduled once."),
    ]
}

/// One run as `execution-get` reports it, with the transcript the detectors
/// read: `repeats` re-fetches a contract already in context.
fn e2e_run_record(run_id: &str, repeats: bool, completion: &str, technical: &str) -> Value {
    json!({"run_id": run_id, "session_id": format!("e2e_{run_id}"), "status": "passed",
        "completion": completion, "technical": technical, "wall_time_ms": 1000,
        "cost": {"total_usd": 0.01}, "efficiency": {"total_tokens": 100, "function_calls": 4},
        "transcript": {"messages": if repeats { rediscovery_transcript() } else { clean_transcript() }}})
}

fn runs_bundle(id: &str, runs: Vec<Value>) -> Value {
    json!({"manifest": {"executions": [{"id": id, "status": "passed", "label": id}]},
        "detail": {"availability": "available",
            "scenario_metrics": [{"scenario_id": SCENARIO, "run_count": runs.len(),
                "behavior_sha256": "sha256:aa", "contract_fingerprint": "fnv1a32:1"}],
            "reports": [{"scenario_id": SCENARIO, "subject_id": "m", "available": true,
                "report": {"subject": {"model": "deepseek-flash", "provider": "deepseek"},
                    "system_under_test": {"harness_version": "1.8.43", "engine_version": "e",
                        "e2e_revision": "r"},
                    "scenarios": [{"scenario_id": SCENARIO, "runs": runs}]}}]}})
}

/// `count` complete runs, all repeating a contract or none of them.
fn runs(prefix: &str, count: usize, repeats: bool) -> Vec<Value> {
    (0..count)
        .map(|at| e2e_run_record(&format!("{prefix}{at}"), repeats, "completed", "valid"))
        .collect()
}

/// A baseline whose runs repeat the contract fetch and a candidate whose do not.
fn pair_world(world: &mut World) {
    world
        .executions
        .insert("plan-1".into(), runs_bundle("plan-1", runs("b", 3, true)));
    world
        .executions
        .insert("plan-2".into(), runs_bundle("plan-2", runs("c", 3, false)));
    world
        .executions
        .insert("exec-b".into(), runs_bundle("exec-b", runs("b", 3, true)));
    world
        .executions
        .insert("exec-c".into(), runs_bundle("exec-c", runs("c", 3, false)));
}

fn executions_state(baseline: &str, candidate: &str) -> Value {
    json!({"executions": [{"id": "plan-1", "state": baseline}, {"id": "plan-2", "state": candidate}]})
}

fn start_request(evaluation_id: &str, candidate_ref: &str) -> StartValidationRequestV1 {
    serde_json::from_value(json!({
        "evaluation_id": evaluation_id, "suggestion_index": 0, "scenario_id": SCENARIO,
        "candidate_ref": candidate_ref, "runs": 3,
        "model": "deepseek-flash", "provider": "deepseek", "by": "ana",
        "criterion": {"metric": "signal_per_run", "pattern": PATTERN, "direction": "decrease",
                      "min_effect": 0.5, "min_runs": 3}}))
    .unwrap()
}

async fn start(
    h: &Harness,
    evaluation_id: &str,
    edit: impl FnOnce(&mut Value),
) -> Result<SuggestionReviewV1, EvalError> {
    let mut request = serde_json::to_value(json!({
        "evaluation_id": evaluation_id, "suggestion_index": 0, "scenario_id": SCENARIO,
        "candidate_ref": "feat", "runs": 3, "model": "deepseek-flash", "provider": "deepseek",
        "by": "ana",
        "criterion": {"metric": "signal_per_run", "pattern": PATTERN, "direction": "decrease",
                      "min_effect": 0.5, "min_runs": 3}}))
    .unwrap();
    edit(&mut request);
    match validation::start_validation(&h.deps, serde_json::from_value(request).unwrap()).await? {
        StartValidationResponseV1::Started(row) => Ok(*row),
        StartValidationResponseV1::Resolved(_) => panic!("a dry run answered a start"),
    }
}

async fn review_with(
    h: &Harness,
    evaluation_id: &str,
    body: Value,
) -> Result<SuggestionReviewV1, EvalError> {
    let mut request = json!({"evaluation_id": evaluation_id, "suggestion_index": 0, "by": "ana"});
    request
        .as_object_mut()
        .unwrap()
        .extend(body.as_object().unwrap().clone());
    review::review(&h.deps, serde_json::from_value(request).unwrap()).await
}

async fn attach(h: &Harness, evaluation_id: &str, baseline: &str, candidate: &str) {
    runtime::attach_validation(
        &h.deps,
        serde_json::from_value(
            json!({"evaluation_id": evaluation_id, "suggestion_index": 0,
            "baseline_execution_id": baseline, "candidate_execution_id": candidate}),
        )
        .unwrap(),
    )
    .await
    .unwrap();
}

fn stored_reviews(h: &Harness) -> usize {
    h.world()
        .state
        .keys()
        .filter(|(scope, _)| scope == state::REVIEW_SCOPE)
        .count()
}

async fn row(h: &Harness, evaluation_id: &str) -> SuggestionReviewV1 {
    state::get_review(&h.deps.iii, evaluation_id, 0)
        .await
        .unwrap()
        .expect("the row is stored")
}

fn criterion_json(metric: &str, min_runs: u32) -> Value {
    let mut criterion = json!({"metric": metric, "direction": "decrease", "min_effect": 0.5,
        "min_runs": min_runs});
    if metric == "signal_per_run" {
        criterion["pattern"] = json!(PATTERN);
    }
    criterion
}

#[tokio::test]
async fn starting_a_validation_pins_both_commits_and_the_sweep_attaches_the_pair() {
    let repo = Repo::new(true);
    let mut world = World::new();
    world.stacks = Some(stacks());
    pair_world(&mut world);
    let (h, evaluation_id) = analyzed_with_suggestion(world).await;

    // An untouched suggestion reads as `new`, with the patterns its evidence
    // cites; nothing is stored until somebody acts.
    let fresh = h.result(&evaluation_id).await.reviews;
    assert_eq!(fresh.len(), 1);
    assert_eq!(fresh[0].lifecycle.status, LifecycleStatusV1::New);
    assert_eq!(fresh[0].patterns, [PATTERN]);
    assert_eq!(fresh[0].scenario_id.as_deref(), Some(SCENARIO));
    assert_eq!(
        fresh[0].title,
        "Registry notice triggers redundant contract discovery"
    );
    assert_eq!(stored_reviews(&h), 0);

    h.configure_code(Some(&repo.path())).await.unwrap();
    let StartValidationResponseV1::Started(started) =
        validation::start_validation(&h.deps, start_request(&evaluation_id, "feat"))
            .await
            .unwrap()
    else {
        panic!("a start answered a dry run")
    };
    let started = *started;
    {
        let world = h.world();
        let calls = world.calls_to("e2e::dashboard::execution-start");
        assert_eq!(calls.len(), 2);
        for (call, side, commit) in [
            (&calls[0], "baseline", &repo.base),
            (&calls[1], "candidate", &repo.feat),
        ] {
            assert_eq!(call["label"], format!("eval {evaluation_id} S1 {side}"));
            let parameters = &call["parameters"];
            assert_eq!(parameters["scenarios"], json!([SCENARIO]));
            assert_eq!(
                (
                    &parameters["runs"],
                    &parameters["where"],
                    &parameters["model"]
                ),
                (&json!(3), &json!("docker"), &json!("deepseek-flash"))
            );
            assert_eq!(parameters["provider"], "deepseek");
            let stack = &parameters["stack"];
            assert!(stack["name"].as_str().unwrap().contains(side), "{stack}");
            let yaml: serde_yaml::Value =
                serde_yaml::from_str(stack["yaml"].as_str().unwrap()).unwrap();
            let harness = &yaml["containers"]["harness"];
            assert_eq!(harness["worker"], "package://harness");
            assert_eq!(harness["commit"], commit.as_str());
            assert_eq!(harness["repository"], "iii-hq/workers");
            assert!(harness.get("version").is_none());
            assert_eq!(yaml["containers"]["harness-e2e"]["version"], "0.17.2");
            assert_eq!(yaml["template"], "harness");
        }
        assert_eq!(world.calls_to("e2e::dashboard::stacks-list").len(), 1);
    }
    let run = started.run.clone().unwrap();
    assert_eq!(run.state, ValidationRunStateV1::Running);
    assert_eq!(run.baseline_execution_id.as_deref(), Some("plan-1"));
    assert_eq!(run.candidate_execution_id.as_deref(), Some("plan-2"));
    assert_eq!(
        (run.baseline_commit.as_str(), run.candidate_commit.as_str()),
        (repo.base.as_str(), repo.feat.as_str()),
        "the baseline defaults to the merge base with origin/main"
    );
    assert_eq!((run.runs, run.scenario_id.as_str()), (3, SCENARIO));
    // The criterion was registered before the run began.
    let criterion = started.criterion.clone().unwrap();
    assert!(criterion.registered_at <= run.started_at);
    assert_eq!(criterion.registered_by, "ana");
    assert_eq!(criterion.scenario_id, SCENARIO);
    assert_eq!(row(&h, &evaluation_id).await, started);

    // Only one run at a time.
    let second = start(&h, &evaluation_id, |_| {}).await.unwrap_err();
    assert!(matches!(second, EvalError::Conflict(_)), "{second:?}");
    assert_eq!(
        h.world().calls_to("e2e::dashboard::execution-start").len(),
        2
    );

    // Still running: the sweep only asks for the two executions.
    h.world().executions_list = Some(executions_state("running", "completed"));
    runtime::sweep(&h.deps).await.unwrap();
    assert_eq!(
        h.world().calls_to("e2e::dashboard::executions-list"),
        [json!({"ids": ["plan-1", "plan-2"]})]
    );
    assert!(h
        .world()
        .calls_to("e2e::dashboard::execution-get")
        .is_empty());
    assert_eq!(
        row(&h, &evaluation_id).await.run.unwrap().state,
        ValidationRunStateV1::Running
    );

    // Both ended: the pair is attached and the evidence computed in code.
    h.world().executions_list = Some(executions_state("completed", "completed"));
    runtime::sweep(&h.deps).await.unwrap();
    let EvalResultResponseV1 {
        record,
        assets,
        reviews,
    } = h.result(&evaluation_id).await;
    let attached = &reviews[0];
    assert_eq!(
        attached.run.as_ref().unwrap().state,
        ValidationRunStateV1::Attached
    );
    assert_eq!(record.counters.validations, 1);
    assert_eq!(assets.validations.len(), 1);
    assert_eq!(assets.validations[0].baseline.execution_id, "plan-1");
    let evidence = attached.evidence.as_ref().unwrap();
    assert_eq!(evidence.scenario_id, SCENARIO);
    assert_eq!(
        evidence.computed_outcome,
        ValidationOutcomeV1::ValidatedImprovement
    );
    assert_eq!(
        (evidence.baseline.n, evidence.baseline.mean),
        (3, Some(1.0))
    );
    assert_eq!(
        (evidence.candidate.n, evidence.candidate.mean),
        (3, Some(0.0))
    );
    assert!(
        evidence.reason.contains("baseline 1 → candidate 0"),
        "{}",
        evidence.reason
    );
    assert_eq!(evidence.baseline.runs.len(), 3);
    assert_eq!(
        evidence.baseline.runs[0].signals,
        Some(BTreeMap::from([(PATTERN.to_string(), 1)]))
    );
    assert_eq!(
        evidence.candidate.runs[0].signals,
        Some(BTreeMap::from([(PATTERN.to_string(), 0)]))
    );

    // The pair is attached once: a later sweep asks nothing more.
    let asked = h.world().calls_to("e2e::dashboard::execution-get").len();
    runtime::sweep(&h.deps).await.unwrap();
    assert_eq!(
        h.world().calls_to("e2e::dashboard::execution-get").len(),
        asked
    );
    assert_eq!(
        h.world().calls_to("e2e::dashboard::executions-list").len(),
        2
    );

    // The person's verdict follows, against the criterion registered first.
    let verdict = review_with(
        &h,
        &evaluation_id,
        json!({"action": "set_verdict", "outcome": "validated_improvement",
               "rationale": "3.0 → 0.0 with the same invariants",
               "controls_checked": ["a contract that really changes is still re-fetched"]}),
    )
    .await
    .unwrap()
    .verdict
    .unwrap();
    assert_eq!(verdict.outcome, ValidationOutcomeV1::ValidatedImprovement);
    assert_eq!(verdict.criterion_snapshot, Some(criterion));
    assert_eq!(verdict.by, "ana");
}

#[tokio::test]
async fn a_validation_that_cannot_start_is_refused_before_anything_is_spent() {
    let repo = Repo::new(true);
    let bare = Repo::new(false);
    let mut world = World::new();
    world.stacks = Some(stacks());
    let (h, evaluation_id) = analyzed_with_suggestion(world).await;
    let refused = |error: EvalError, prefix: &str| {
        assert!(error.to_string().contains(prefix), "{prefix}: {error}");
    };

    // No codebase directory: no way to resolve a commit.
    refused(
        start(&h, &evaluation_id, |_| {}).await.unwrap_err(),
        "code_repository_required:",
    );
    h.configure_code(Some(&repo.path())).await.unwrap();

    for (what, edit, prefix) in [
        (
            "unknown ref",
            Box::new(|r: &mut Value| r["candidate_ref"] = json!("nope")) as Box<dyn Fn(&mut Value)>,
            "git_ref_invalid:",
        ),
        (
            "an option",
            Box::new(|r| r["candidate_ref"] = json!("--upload-pack=x")),
            "git_ref_invalid:",
        ),
        (
            "unpushed candidate",
            Box::new(|r| r["candidate_ref"] = json!("local")),
            "commit_not_pushed:",
        ),
        (
            "unpushed baseline",
            Box::new(|r| r["baseline_ref"] = json!("local")),
            "commit_not_pushed: the baseline",
        ),
        (
            "candidate already in main",
            Box::new(|r| r["candidate_ref"] = json!("main")),
            "git_ref_invalid: baseline and candidate are the same commit",
        ),
        (
            "no runs",
            Box::new(|r| r["runs"] = json!(0)),
            "runs must be between 1 and 20",
        ),
        (
            "too many runs",
            Box::new(|r| r["runs"] = json!(21)),
            "runs must be between 1 and 20",
        ),
        (
            "criterion beyond the runs",
            Box::new(|r| r["runs"] = json!(2)),
            "needs 3 completed runs",
        ),
        (
            "no scenario",
            Box::new(|r| r["scenario_id"] = json!(" ")),
            "scenario_id is required",
        ),
        (
            "no author",
            Box::new(|r| r["by"] = json!(" ")),
            "`by` is required",
        ),
        (
            "no model",
            Box::new(|r| r["model"] = json!(" ")),
            "model is required",
        ),
        (
            "no criterion",
            Box::new(|r| {
                r.as_object_mut().unwrap().remove("criterion");
            }),
            "criterion is required",
        ),
        (
            "no such suggestion",
            Box::new(|r| r["suggestion_index"] = json!(4)),
            "does not exist",
        ),
        (
            "a pattern the suggestion does not cite",
            Box::new(|r| r["criterion"]["pattern"] = json!("repeated_tool_error:x")),
            "needs a pattern the suggestion cites",
        ),
    ] {
        let error = start(&h, &evaluation_id, |r| edit(r)).await.unwrap_err();
        assert!(error.to_string().contains(prefix), "{what}: {error}");
    }
    // A clone without origin/main has no default baseline.
    h.configure_code(Some(&bare.path())).await.unwrap();
    refused(
        start(&h, &evaluation_id, |_| {}).await.unwrap_err(),
        "has no merge base",
    );
    h.configure_code(Some(&repo.path())).await.unwrap();

    // An E2E that cannot say how to build the stack.
    h.world().stacks = Some(json!({"stacks": [{"id": "default", "template": null, "yaml": ""}]}));
    refused(
        start(&h, &evaluation_id, |_| {}).await.unwrap_err(),
        "e2e_unavailable:",
    );
    h.world().stacks = None;
    refused(
        start(&h, &evaluation_id, |_| {}).await.unwrap_err(),
        "e2e_unavailable:",
    );

    // An analysis that has not finished.
    h.world().stacks = Some(stacks());
    let pending = reanalyze(&h).await;
    let error = start(&h, &pending, |_| {}).await.unwrap_err();
    assert!(matches!(error, EvalError::Conflict(_)), "{error:?}");

    // None of that registered a criterion or reached the E2E.
    assert_eq!(stored_reviews(&h), 0);
    assert!(h
        .world()
        .calls_to("e2e::dashboard::execution-start")
        .is_empty());

    // The E2E refuses the baseline: the criterion stays registered, the run failed.
    h.world().start_failure = Some((0, "the E2E is busy: another execution is in progress"));
    let busy = start(&h, &evaluation_id, |_| {}).await.unwrap_err();
    refused(busy, "e2e_busy:");
    let failed = row(&h, &evaluation_id).await;
    let run = failed.run.unwrap();
    assert_eq!(run.state, ValidationRunStateV1::Failed);
    assert!(run.error.unwrap().starts_with("e2e_busy:"));
    assert_eq!(
        (run.baseline_execution_id, run.candidate_execution_id),
        (None, None)
    );
    assert!(failed.criterion.is_some());

    // The E2E refuses the candidate after taking the baseline: it is named.
    {
        let mut world = h.world();
        world.starts = 0;
        world.start_failure = Some((1, "handler error: invalid stack"));
    }
    let half = start(&h, &evaluation_id, |_| {}).await.unwrap_err();
    refused(half, "e2e_unavailable:");
    let run = row(&h, &evaluation_id).await.run.unwrap();
    assert_eq!(run.state, ValidationRunStateV1::Failed);
    assert_eq!(run.baseline_execution_id.as_deref(), Some("plan-1"));
    assert!(run
        .error
        .unwrap()
        .contains("baseline execution plan-1 was already started"));
    // A failed run can be started again with the same criterion.
    h.world().start_failure = None;
    let again = start(&h, &evaluation_id, |_| {}).await.unwrap();
    assert_eq!(again.run.unwrap().state, ValidationRunStateV1::Running);
}

/// What the dialog sends while the person is still typing: the refs, the
/// scenario and the runs, no model and no criterion.
async fn dry_run(
    h: &Harness,
    evaluation_id: &str,
    edit: impl FnOnce(&mut Value),
) -> Result<ValidationResolutionV1, EvalError> {
    let mut request = json!({"evaluation_id": evaluation_id, "suggestion_index": 0,
        "scenario_id": SCENARIO, "candidate_ref": "feat", "runs": 3, "dry_run": true});
    edit(&mut request);
    match validation::start_validation(&h.deps, serde_json::from_value(request).unwrap()).await? {
        StartValidationResponseV1::Resolved(resolution) => Ok(resolution),
        StartValidationResponseV1::Started(_) => panic!("a start answered a dry run"),
    }
}

#[tokio::test]
async fn a_dry_run_resolves_the_refs_like_a_start_and_changes_nothing() {
    let repo = Repo::new(true);
    let mut world = World::new();
    world.stacks = Some(stacks());
    let (h, evaluation_id) = analyzed_with_suggestion(world).await;
    let before = h.world().calls.len();
    let refused = |error: EvalError, prefix: &str| {
        assert!(error.to_string().contains(prefix), "{prefix}: {error}");
    };

    refused(
        dry_run(&h, &evaluation_id, |_| {}).await.unwrap_err(),
        "code_repository_required:",
    );
    h.configure_code(Some(&repo.path())).await.unwrap();

    // The baseline defaults to the merge base with origin/main, which holds it.
    let resolved = dry_run(&h, &evaluation_id, |_| {}).await.unwrap();
    assert_eq!(
        resolved,
        ValidationResolutionV1 {
            baseline: ResolvedCommitV1 {
                commit: repo.base.clone(),
                short: repo.base[..12].into(),
                branch: "origin/main".into(),
            },
            candidate: ResolvedCommitV1 {
                commit: repo.feat.clone(),
                short: repo.feat[..12].into(),
                branch: "origin/feat".into(),
            },
            warnings: vec![],
        }
    );
    // A commit typed in full resolves the same way.
    let by_sha = dry_run(&h, &evaluation_id, |r| {
        r["candidate_ref"] = json!(repo.feat)
    })
    .await
    .unwrap();
    assert_eq!(by_sha, resolved);

    // The same refusals a start gives, before anything is spent.
    for (what, edit, prefix) in [
        (
            "unknown ref",
            Box::new(|r: &mut Value| r["candidate_ref"] = json!("nope")) as Box<dyn Fn(&mut Value)>,
            "git_ref_invalid: the candidate ref `nope`",
        ),
        (
            "not pushed",
            Box::new(|r| r["candidate_ref"] = json!("local")),
            "commit_not_pushed: the candidate commit",
        ),
        (
            "baseline not pushed",
            Box::new(|r| r["baseline_ref"] = json!("local")),
            "commit_not_pushed: the baseline commit",
        ),
        (
            "baseline unknown",
            Box::new(|r| r["baseline_ref"] = json!("nope")),
            "git_ref_invalid: the baseline ref `nope`",
        ),
        (
            "same commit",
            Box::new(|r| r["candidate_ref"] = json!("main")),
            "git_ref_invalid: baseline and candidate are the same commit",
        ),
        (
            "runs",
            Box::new(|r| r["runs"] = json!(21)),
            "runs must be between 1 and 20",
        ),
        (
            "no scenario",
            Box::new(|r| r["scenario_id"] = json!(" ")),
            "scenario_id is required",
        ),
        (
            "a criterion that needs more runs",
            Box::new(|r| {
                r["criterion"] = criterion_json("signal_per_run", 5);
            }),
            "the criterion needs 5 completed runs",
        ),
    ] {
        let error = dry_run(&h, &evaluation_id, |r| edit(r)).await.unwrap_err();
        assert!(error.to_string().contains(prefix), "{what}: {error}");
    }

    // A baseline that is not behind the candidate is allowed, with a warning.
    let diverged = dry_run(&h, &evaluation_id, |r| {
        r["candidate_ref"] = json!("main");
        r["baseline_ref"] = json!("feat");
    })
    .await
    .unwrap();
    assert_eq!(diverged.baseline.branch, "origin/feat");
    assert_eq!(diverged.warnings.len(), 1);
    assert!(
        diverged.warnings[0].contains("is not an ancestor of the candidate"),
        "{:?}",
        diverged.warnings
    );

    // Nothing was registered, recorded or asked of the E2E.
    assert_eq!(stored_reviews(&h), 0);
    assert!(h.world().calls[before..]
        .iter()
        .all(|(function, _)| !function.starts_with("e2e::")));
}

#[tokio::test]
async fn a_start_the_e2e_never_answers_stays_in_progress_instead_of_inviting_a_duplicate() {
    let repo = Repo::new(true);
    let mut world = World::new();
    world.stacks = Some(stacks());
    // The baseline is accepted; the candidate's answer never arrives.
    world.unanswered_start = Some(1);
    let (mut h, evaluation_id) = analyzed_with_suggestion(world).await;
    h.configure_code(Some(&repo.path())).await.unwrap();
    h.deps.start_timeout_ms = 300;

    let error = start(&h, &evaluation_id, |_| {}).await.unwrap_err();
    assert!(matches!(error, EvalError::Unanswered(_)), "{error:?}");
    let text = error.to_string();
    assert!(text.contains("e2e_start_unconfirmed:"), "{text}");
    assert!(
        text.contains(&format!("eval {evaluation_id} S1 candidate")),
        "it names what to look for in the E2E: {text}"
    );
    assert!(text.contains("baseline execution plan-1"), "{text}");
    // The candidate may be running: the run is not failed, and what is known
    // is kept.
    let run = row(&h, &evaluation_id).await.run.unwrap();
    assert_eq!(run.state, ValidationRunStateV1::Starting);
    assert_eq!(run.baseline_execution_id.as_deref(), Some("plan-1"));
    assert_eq!((run.candidate_execution_id, run.error), (None, None));

    // So a second click cannot start a second pair.
    let again = start(&h, &evaluation_id, |_| {}).await.unwrap_err();
    assert!(matches!(again, EvalError::Conflict(_)), "{again:?}");
    assert_eq!(
        h.world().calls_to("e2e::dashboard::execution-start").len(),
        2
    );
    runtime::sweep(&h.deps).await.unwrap();
    assert_eq!(
        row(&h, &evaluation_id).await.run.unwrap().state,
        ValidationRunStateV1::Starting,
        "a fresh start is left alone"
    );

    // Minutes later the sweep gives it up and the person, who looked in the
    // E2E, can start again.
    let mut stale = row(&h, &evaluation_id).await;
    stale.run.as_mut().unwrap().started_at = ids::now_ms() - 10 * 60 * 1_000;
    state::put_review(&h.deps.iii, &stale).await.unwrap();
    runtime::sweep(&h.deps).await.unwrap();
    let failed = row(&h, &evaluation_id).await;
    assert_eq!(
        failed.run.as_ref().unwrap().state,
        ValidationRunStateV1::Failed
    );
    assert!(failed
        .run
        .unwrap()
        .error
        .unwrap()
        .contains("never answered"));
    let restarted = start(&h, &evaluation_id, |_| {}).await.unwrap();
    assert_eq!(restarted.run.unwrap().state, ValidationRunStateV1::Running);
    assert_eq!(
        restarted.first_run_at,
        Some(stale.run.unwrap().started_at),
        "the replaced run's executions still count as results that may have been seen"
    );
}

#[tokio::test]
async fn a_verdict_of_improvement_needs_the_criterion_first_and_the_runs_to_match() {
    let mut world = World::new();
    pair_world(&mut world);
    let (mut h, first) = analyzed_with_suggestion(world).await;
    let settle = || tokio::time::sleep(Duration::from_millis(5));

    // Lifecycle: who moved it and when, with what each status needs.
    let moved = review_with(
        &h,
        &first,
        json!({"action": "set_lifecycle", "status": "accepted",
        "note": "worth a try"}),
    )
    .await
    .unwrap();
    assert_eq!(moved.lifecycle.status, LifecycleStatusV1::Accepted);
    assert_eq!(moved.lifecycle.history[0].by, "ana");
    assert_eq!(
        moved.lifecycle.history[0].note.as_deref(),
        Some("worth a try")
    );
    assert_eq!(
        moved.patterns,
        [PATTERN],
        "the row copies what outlives the analysis"
    );
    // Without a name the host's user is credited, not the opaque worker id the
    // engine stamped on the call.
    h.deps.host_user = Some("layon".into());
    let by_host = review::review(
        &h.deps,
        serde_json::from_value(json!({"evaluation_id": first, "suggestion_index": 0,
            "action": "set_lifecycle", "status": "in_progress", "pr": "#1300",
            "_caller_worker_id": "fee30d6b-1890-4a8e-9d51-0c6e5d1f3a77"}))
        .unwrap(),
    )
    .await
    .unwrap();
    h.deps.host_user = None;
    assert_eq!(by_host.lifecycle.history[1].by, "layon");
    for (body, why) in [
        (json!({"action": "set_lifecycle"}), "no status"),
        (
            json!({"action": "set_lifecycle", "status": "new"}),
            "back to new",
        ),
        (
            json!({"action": "set_lifecycle", "status": "rejected"}),
            "no reason",
        ),
        (
            json!({"action": "set_lifecycle", "status": "shipped", "pr": "#1", "version": "latest"}),
            "version",
        ),
        (json!({"action": "set_criterion"}), "no criterion"),
        (
            json!({"action": "set_verdict", "outcome": "no_improvement"}),
            "no rationale",
        ),
        (
            json!({"action": "set_lifecycle", "status": "accepted", "by": " "}),
            "no author",
        ),
    ] {
        let mut request = json!({"evaluation_id": first, "suggestion_index": 0});
        request
            .as_object_mut()
            .unwrap()
            .extend(body.as_object().unwrap().clone());
        // The author comes from `by` unless a body overrides it.
        if !request.as_object().unwrap().contains_key("by") {
            request["by"] = json!("ana");
        }
        let error = review::review(&h.deps, serde_json::from_value(request).unwrap())
            .await
            .unwrap_err();
        assert!(
            matches!(error, EvalError::InvalidRequest(_)),
            "{why}: {error:?}"
        );
    }
    for (id, index) in [("eval_nobody", 0), (first.as_str(), 3)] {
        let mut request = json!({"evaluation_id": id, "suggestion_index": index, "by": "ana",
            "action": "set_lifecycle", "status": "accepted"});
        request["suggestion_index"] = json!(index);
        assert!(
            review::review(&h.deps, serde_json::from_value(request).unwrap())
                .await
                .is_err()
        );
    }
    assert_eq!(
        row(&h, &first).await.lifecycle.history.len(),
        2,
        "refusals change nothing"
    );

    // 1. No criterion at all.
    let verdict = |outcome: &str| {
        json!({"action": "set_verdict", "outcome": outcome, "rationale": "the runs show it",
               "controls_checked": ["control one"]})
    };
    let refusal = review_with(&h, &first, verdict("validated_improvement"))
        .await
        .unwrap_err();
    assert!(
        refusal
            .to_string()
            .contains("verdict_refused: no criterion"),
        "{refusal}"
    );
    // Other outcomes need no criterion.
    let recorded = review_with(&h, &first, verdict("inconclusive"))
        .await
        .unwrap();
    assert_eq!(recorded.verdict.unwrap().criterion_snapshot, None);

    // 2. The pair is attached before the criterion exists: the evidence is
    // computed but cannot validate, however good it looks.
    attach(&h, &first, "exec-b", "exec-c").await;
    let with_evidence = row(&h, &first).await;
    let evidence = with_evidence.evidence.as_ref().unwrap();
    assert_eq!(evidence.computed_outcome, ValidationOutcomeV1::Inconclusive);
    assert!(
        evidence.reason.contains("no criterion"),
        "{}",
        evidence.reason
    );
    assert_eq!((evidence.baseline.n, evidence.candidate.n), (3, 3));
    settle().await;
    let late = review_with(
        &h,
        &first,
        json!({"action": "set_criterion", "criterion": criterion_json("signal_per_run", 3)}),
    )
    .await
    .unwrap();
    assert_eq!(
        late.evidence.unwrap().computed_outcome,
        ValidationOutcomeV1::ValidatedImprovement
    );
    let refusal = review_with(&h, &first, verdict("validated_improvement"))
        .await
        .unwrap_err();
    assert!(
        refusal
            .to_string()
            .contains("registered after the first E2E execution started"),
        "{refusal}"
    );
    // Results exist, so the criterion no longer changes.
    let frozen = review_with(
        &h,
        &first,
        json!({"action": "set_criterion", "criterion": criterion_json("cost_usd", 3)}),
    )
    .await
    .unwrap_err();
    assert!(frozen.to_string().contains("criterion_frozen:"), "{frozen}");
    // The same pair attached again keeps one link and its first time.
    let attached_at = h.result(&first).await.assets.validations[0].attached_at;
    attach(&h, &first, "exec-b", "exec-c").await;
    let validations = h.result(&first).await.assets.validations;
    assert_eq!(validations.len(), 1);
    assert_eq!(validations[0].attached_at, attached_at);

    // 3. A criterion registered first, but the runs are fewer than it needs.
    let second = reanalyze(&h).await;
    investigated(&h, &second).await;
    review_with(
        &h,
        &second,
        json!({"action": "set_criterion", "criterion": criterion_json("signal_per_run", 5)}),
    )
    .await
    .unwrap();
    settle().await;
    attach(&h, &second, "exec-b", "exec-c").await;
    let few = row(&h, &second).await;
    assert_eq!(
        few.evidence.unwrap().computed_outcome,
        ValidationOutcomeV1::Inconclusive
    );
    let refusal = review_with(&h, &second, verdict("validated_improvement"))
        .await
        .unwrap_err();
    assert!(
        refusal
            .to_string()
            .contains("the baseline has 3 completed run(s), the criterion needs 5"),
        "{refusal}"
    );

    // 4. A criterion first and enough completed runs: recorded with its snapshot.
    let third = reanalyze(&h).await;
    investigated(&h, &third).await;
    review_with(
        &h,
        &third,
        json!({"action": "set_criterion", "criterion": criterion_json("signal_per_run", 3)}),
    )
    .await
    .unwrap();
    settle().await;
    attach(&h, &third, "exec-b", "exec-c").await;
    let done = review_with(&h, &third, verdict("validated_improvement"))
        .await
        .unwrap();
    assert_eq!(
        done.verdict.as_ref().unwrap().outcome,
        ValidationOutcomeV1::ValidatedImprovement
    );
    assert_eq!(done.verdict.unwrap().criterion_snapshot, done.criterion);

    // The list's summary counts each analysis's suggestions by status.
    let listed = review::reviews(&h.deps, ReviewsRequestV1::default())
        .await
        .unwrap();
    let summary = |id: &str| {
        listed
            .summaries
            .iter()
            .find(|summary| summary.evaluation_id == id)
            .unwrap()
            .clone()
    };
    assert_eq!(listed.summaries.len(), 3);
    assert_eq!(listed.reviews.len(), 3);
    let first_summary = summary(&first);
    assert_eq!(
        (
            first_summary.suggestions,
            first_summary.in_progress,
            first_summary.new
        ),
        (1, 1, 0)
    );
    // A criterion is not a lifecycle change: the suggestion still needs review.
    assert_eq!((summary(&second).suggestions, summary(&second).new), (1, 1));
    let one = review::reviews(
        &h.deps,
        ReviewsRequestV1 {
            evaluation_id: Some(first.clone()),
        },
    )
    .await
    .unwrap();
    assert_eq!((one.reviews.len(), one.summaries.len()), (1, 1));
    // An untouched suggestion counts as new.
    h.world()
        .state
        .retain(|(scope, _), _| scope != state::REVIEW_SCOPE);
    let untouched = review::reviews(&h.deps, ReviewsRequestV1::default())
        .await
        .unwrap();
    assert!(untouched.reviews.is_empty());
    assert!(untouched.summaries.iter().all(|summary| summary.new == 1));
}

/// The same turn admitted by a live event again (its index gone, as after
/// retention): another automatic analysis.
async fn admitted_again(h: &Harness) -> String {
    let key = ids::observation_key(ROOT, TURN);
    state::delete_observation(&h.deps.iii, &key).await.unwrap();
    h.end_turn(ROOT, TURN).await.evaluation_id.unwrap()
}

#[tokio::test]
async fn recurrence_compares_the_analyses_before_and_from_the_shipped_version() {
    let mut world = World::new();
    world.workers = Some(json!({"workers": [
        {"name": "harness", "namespace": "default", "version": "1.8.42"}]}));
    let namespace = {
        // The worker's own namespace, as admission reads it.
        let probe = Harness::start(World::new()).await;
        probe.iii.namespace().unwrap_or_else(|| "default".into())
    };
    let versions = |version: Option<&str>| {
        version.map(|version| {
            json!({"workers": [{"name": "harness", "namespace": namespace, "version": version}]})
        })
    };
    world.workers = versions(Some("1.8.42"));
    let (h, first) = analyzed_with_suggestion(world).await;
    assert_eq!(
        h.result(&first).await.record.signals,
        BTreeMap::from([(PATTERN.to_string(), 1)])
    );

    // Before anything shipped there is nothing to compare.
    let unshipped = review::recurrence(
        &h.deps,
        RecurrenceRequestV1 {
            evaluation_id: first.clone(),
            suggestion_index: 0,
        },
    )
    .await
    .unwrap_err();
    assert!(
        unshipped.to_string().contains("recurrence_unavailable:"),
        "{unshipped}"
    );
    review_with(
        &h,
        &first,
        json!({"action": "set_lifecycle", "status": "accepted"}),
    )
    .await
    .unwrap();
    let unversioned = review_with(
        &h,
        &first,
        json!({"action": "set_lifecycle", "status": "shipped",
        "pr": "#1300"}),
    )
    .await
    .unwrap();
    assert_eq!(unversioned.lifecycle.version, None);
    assert!(review::recurrence(
        &h.deps,
        RecurrenceRequestV1 {
            evaluation_id: first.clone(),
            suggestion_index: 0
        }
    )
    .await
    .is_err());
    review_with(
        &h,
        &first,
        json!({"action": "set_lifecycle", "status": "shipped",
        "version": "1.8.43"}),
    )
    .await
    .unwrap();

    // Another analysis on the same old version that still finds the pattern.
    let old = admitted_again(&h).await;
    investigated(&h, &old).await;
    // Then analyses on the release: the transcript no longer repeats.
    h.world().workers = versions(Some("1.8.43"));
    h.world().entries.insert(ROOT.into(), clean_transcript());
    let released = admitted_again(&h).await;
    investigated(&h, &released).await;
    let later = admitted_again(&h).await;
    investigated(&h, &later).await;
    // A manual analysis may be of a session that ran on an older version, and
    // a second analysis of one turn would count its signals twice: it is
    // left out whatever version is installed.
    let manual = reanalyze(&h).await;
    investigated(&h, &manual).await;
    assert_eq!(h.result(&manual).await.record.harness_version, None);
    // An engine that did not say its version.
    h.world().workers = None;
    let unknown = admitted_again(&h).await;
    investigated(&h, &unknown).await;

    let found = review::recurrence(
        &h.deps,
        RecurrenceRequestV1 {
            evaluation_id: first.clone(),
            suggestion_index: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(found.version, "1.8.43");
    assert_eq!(found.without_version, 2);
    assert_eq!((found.before.analyses, found.from_version.analyses), (2, 2));
    let before = &found.before.patterns[0];
    assert_eq!(before.pattern, PATTERN);
    assert_eq!(
        (
            before.occurrences,
            before.analyses_with,
            before.per_analysis
        ),
        (2, 2, Some(1.0))
    );
    let from = &found.from_version.patterns[0];
    assert_eq!(
        (from.occurrences, from.analyses_with, from.per_analysis),
        (0, 0, Some(0.0))
    );

    // The row outlives the analysis it came from.
    runtime::delete(
        &h.deps,
        EvaluationIdRequestV1 {
            evaluation_id: first.clone(),
        },
    )
    .await
    .unwrap();
    let kept = review::reviews(
        &h.deps,
        ReviewsRequestV1 {
            evaluation_id: Some(first.clone()),
        },
    )
    .await
    .unwrap();
    assert_eq!(kept.reviews.len(), 1);
    assert!(kept.summaries.is_empty(), "no analysis left to summarize");
    let still = review::recurrence(
        &h.deps,
        RecurrenceRequestV1 {
            evaluation_id: first,
            suggestion_index: 0,
        },
    )
    .await
    .unwrap();
    assert_eq!(
        still.before.analyses, 1,
        "the deleted analysis is gone from the count"
    );
}

#[tokio::test]
async fn the_sweep_fails_runs_that_cannot_finish_and_retries_the_ones_the_e2e_dropped() {
    let mut world = World::new();
    pair_world(&mut world);
    let (h, evaluation_id) = analyzed_with_suggestion(world).await;
    let base = h.result(&evaluation_id).await.reviews.remove(0);
    let ago = ids::now_ms() - 10 * 60 * 1_000;
    let put = |state: ValidationRunStateV1, started_at: i64, ids: bool| {
        let mut row = base.clone();
        row.criterion = Some(CriterionV1 {
            metric: CriterionMetricV1::SignalPerRun,
            pattern: Some(PATTERN.into()),
            direction: DirectionV1::Decrease,
            min_effect: 0.5,
            min_runs: 3,
            scenario_id: SCENARIO.into(),
            registered_at: started_at,
            registered_by: "ana".into(),
        });
        row.run = Some(ValidationRunV1 {
            baseline_execution_id: ids.then(|| "plan-1".to_string()),
            candidate_execution_id: ids.then(|| "plan-2".to_string()),
            baseline_commit: "a".repeat(40),
            candidate_commit: "b".repeat(40),
            scenario_id: SCENARIO.into(),
            runs: 3,
            model: "deepseek-flash".into(),
            provider: "deepseek".into(),
            started_at,
            finished_at: None,
            state,
            error: None,
        });
        row
    };
    let store = |row: SuggestionReviewV1| {
        let deps = h.deps.clone();
        async move { state::put_review(&deps.iii, &row).await.unwrap() }
    };
    let run_state = || async { row(&h, &evaluation_id).await.run.unwrap() };

    // A start that lost its answer long ago fails; a fresh one is left alone.
    store(put(ValidationRunStateV1::Starting, ids::now_ms(), false)).await;
    runtime::sweep(&h.deps).await.unwrap();
    assert_eq!(run_state().await.state, ValidationRunStateV1::Starting);
    store(put(ValidationRunStateV1::Starting, ago, false)).await;
    runtime::sweep(&h.deps).await.unwrap();
    let lost = run_state().await;
    assert_eq!(lost.state, ValidationRunStateV1::Failed);
    assert!(lost.error.unwrap().contains("the start did not finish"));

    // An execution the E2E no longer lists. Startup recovery never waits on
    // the E2E: the first sweep does.
    store(put(ValidationRunStateV1::Running, ago, true)).await;
    h.world().executions_list = Some(json!({"executions": [{"id": "plan-1", "state": "running"}]}));
    runtime::recover(&h.deps).await.unwrap();
    assert!(h
        .world()
        .calls_to("e2e::dashboard::executions-list")
        .is_empty());
    runtime::sweep(&h.deps).await.unwrap();
    let gone = run_state().await;
    assert_eq!(gone.state, ValidationRunStateV1::Failed);
    assert!(gone
        .error
        .unwrap()
        .contains("e2e_execution_not_found: the candidate execution plan-2"));

    // An E2E that is down when the pair is attached: tried again, and said so.
    store(put(ValidationRunStateV1::Running, ago, true)).await;
    {
        let mut world = h.world();
        world.executions_list = Some(executions_state("completed", "completed"));
        world.executions_down = true;
    }
    runtime::sweep(&h.deps).await.unwrap();
    let waiting = run_state().await;
    assert_eq!(waiting.state, ValidationRunStateV1::Finished);
    assert!(
        waiting.error.as_ref().unwrap().contains("e2e_unavailable"),
        "{:?}",
        waiting.error
    );
    h.world().executions_down = false;
    runtime::sweep(&h.deps).await.unwrap();
    let done = run_state().await;
    assert_eq!(
        (done.state, done.error),
        (ValidationRunStateV1::Attached, None)
    );

    // Down for good: the sweep keeps trying for a while, then gives up and
    // says why, so the person can attach by hand or start again.
    h.world().executions_down = true;
    let finished_ago = |minutes: i64| {
        let mut row = put(ValidationRunStateV1::Finished, ago, true);
        row.run.as_mut().unwrap().finished_at = Some(ids::now_ms() - minutes * 60 * 1_000);
        row
    };
    store(finished_ago(5)).await;
    runtime::sweep(&h.deps).await.unwrap();
    assert_eq!(run_state().await.state, ValidationRunStateV1::Finished);
    store(finished_ago(31)).await;
    runtime::sweep(&h.deps).await.unwrap();
    let given_up = run_state().await;
    assert_eq!(given_up.state, ValidationRunStateV1::Failed);
    let error = given_up.error.unwrap();
    assert!(error.starts_with("attach_gave_up:"), "{error}");
    assert!(
        error.contains("e2e_unavailable"),
        "the last error is kept: {error}"
    );
    h.world().executions_down = false;

    // Executions that ended badly are attached too: the evidence says why it
    // cannot judge, instead of the pair being dropped.
    h.world().executions.insert(
        "plan-1".into(),
        runs_bundle("plan-1", {
            let mut broken = runs("b", 1, true);
            broken.push(e2e_run_record(
                "b-infra",
                true,
                "incomplete",
                "infrastructure_error",
            ));
            broken.push(e2e_run_record(
                "b-infra2",
                true,
                "incomplete",
                "infrastructure_error",
            ));
            broken
        }),
    );
    h.world()
        .executions
        .insert("plan-2".into(), runs_bundle("plan-2", vec![]));
    store(put(ValidationRunStateV1::Running, ago, true)).await;
    h.world().executions_list = Some(executions_state("failed", "cancelled"));
    runtime::sweep(&h.deps).await.unwrap();
    let ended = row(&h, &evaluation_id).await;
    assert_eq!(ended.run.unwrap().state, ValidationRunStateV1::Attached);
    let evidence = ended.evidence.unwrap();
    assert_eq!(evidence.computed_outcome, ValidationOutcomeV1::Inconclusive);
    assert!(
        evidence
            .reason
            .contains("2 of 3 baseline runs did not complete"),
        "{}",
        evidence.reason
    );
    assert_eq!(
        evidence
            .baseline
            .runs
            .iter()
            .filter(|run| !run.completed)
            .count(),
        2,
        "incomplete runs are listed, not dropped"
    );
    assert_eq!(
        evidence.baseline.runs[1].technical.as_deref(),
        Some("infrastructure_error")
    );

    // An execution the E2E forgot between the end and the attach: no retry.
    h.world().executions.remove("plan-2");
    store(put(ValidationRunStateV1::Finished, ago, true)).await;
    runtime::sweep(&h.deps).await.unwrap();
    let forgotten = run_state().await;
    assert_eq!(forgotten.state, ValidationRunStateV1::Failed);
    assert!(forgotten
        .error
        .unwrap()
        .contains("e2e_execution_not_found(candidate)"));

    // An analysis deleted while its run was going.
    store(put(ValidationRunStateV1::Finished, ago, true)).await;
    runtime::delete(
        &h.deps,
        EvaluationIdRequestV1 {
            evaluation_id: evaluation_id.clone(),
        },
    )
    .await
    .unwrap();
    runtime::sweep(&h.deps).await.unwrap();
    assert_eq!(run_state().await.state, ValidationRunStateV1::Failed);
}

// ---------------------------------------------------------------------------
// Reproduction at the decision point
// ---------------------------------------------------------------------------

async fn reproduce(h: &Harness, request: Value) -> Result<ReproduceResponseV1, EvalError> {
    eval::reproduce::reproduce(&h.deps, serde_json::from_value(request).unwrap()).await
}

/// Waits for the reproduction's background task.
async fn settled(h: &Harness, evaluation_id: &str, id: &str) -> ReproductionV1 {
    for _ in 0..200 {
        let current = row(h, evaluation_id)
            .await
            .reproductions
            .into_iter()
            .find(|reproduction| reproduction.id == id)
            .unwrap();
        if current.state != ReproductionStateV1::Running {
            return current;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the reproduction never finished");
}

/// The Harness's turn record of `turn_id`, with fields beyond the options.
fn turn_record(turn_id: &str) -> Value {
    json!({"turn_id": turn_id, "session_id": ROOT, "status": "completed", "step": 7,
        "turn_count": 3, "options": {"model": "task-model", "provider": "p",
            "system_prompt": "You are an agent.", "functions": {"expose": "agent_trigger"}},
        "function_contract_ledger": {"crm::profile": {"generation": 4}},
        "context_snapshot": {"prompt_surface_digest": "sha256:x",
            "categories": {"hook_guidance": 0}}})
}

/// An analysis whose suggestion has a replayable decision point, made while
/// the Harness holds `record` as the session's latest turn.
async fn replayable(mut world: World, record: Value) -> (Harness, String) {
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
async fn a_replay_reproduces_the_signal_and_the_proposed_change_removes_it() {
    let (h, evaluation_id) = replayable(World::new(), turn_record(TURN)).await;
    let result = h.result(&evaluation_id).await;
    assert_eq!(result.assets.capture.as_ref().unwrap().turn_id, TURN);
    let suggestion = &result.assets.investigation.as_ref().unwrap().suggestions[0];
    assert!(suggestion.check.is_some(), "{}", suggestion.limitations);

    // Nothing is spent or stored by a dry run.
    let preview = reproduce(
        &h,
        json!({"evaluation_id": evaluation_id, "suggestion_index": 0, "dry_run": true}),
    )
    .await
    .unwrap()
    .preview
    .unwrap();
    assert_eq!(preview.original.signal, Some(true));
    assert_eq!(preview.original.calls[0].target, INFO);
    assert!(h.world().calls_to("router::complete").is_empty());

    // The base: the model saw the notice and re-reads the contract.
    let base = reproduce(
        &h,
        json!({"evaluation_id": evaluation_id, "suggestion_index": 0, "samples": 3, "by": "ana"}),
    )
    .await
    .unwrap()
    .reproduction_id
    .unwrap();
    let base = settled(&h, &evaluation_id, &base).await;
    assert_eq!(
        base.state,
        ReproductionStateV1::Completed,
        "{:?}",
        base.error
    );
    assert_eq!(base.samples.len(), 3);
    assert!(base.samples.iter().all(|reply| reply.signal == Some(true)));
    assert!((base.cost_usd.unwrap() - 0.003).abs() < 1e-9);
    assert!(base.fidelity.is_some());

    // The proposed change removes the notice: no reply re-reads it.
    let change = reproduce(
        &h,
        json!({"evaluation_id": evaluation_id, "suggestion_index": 0, "samples": 3,
            "change": {"kind": "proposed"}, "by": "ana"}),
    )
    .await
    .unwrap()
    .reproduction_id
    .unwrap();
    let change = settled(&h, &evaluation_id, &change).await;
    assert_eq!(change.change_kind, ReproductionChangeKindV1::Proposed);
    assert!(change
        .samples
        .iter()
        .all(|reply| reply.signal == Some(false)));
    // Sampling never ran a function.
    assert!(h.world().calls_to(INFO).is_empty());

    // More replies join the same reproduction.
    reproduce(
        &h,
        json!({"evaluation_id": evaluation_id, "suggestion_index": 0, "samples": 2,
            "extend": change.id, "by": "ana"}),
    )
    .await
    .unwrap();
    let extended = settled(&h, &evaluation_id, &change.id).await;
    assert_eq!(extended.samples.len(), 5);

    // A change whose text is not where it says is refused before spending.
    let calls = h.world().calls_to("router::complete").len();
    let refused = reproduce(
        &h,
        json!({"evaluation_id": evaluation_id, "suggestion_index": 0, "by": "ana",
            "change": {"kind": "custom", "edits": [{"target": format!("e_{TURN}_c1"),
                "find": "not in the result", "replace": "x"}]}}),
    )
    .await;
    assert!(
        matches!(refused, Err(EvalError::InvalidRequest(_))),
        "{refused:?}"
    );
    assert_eq!(h.world().calls_to("router::complete").len(), calls);
}

#[tokio::test]
async fn only_console_chats_are_analyzed_automatically_and_the_rest_by_hand() {
    // What session-manager holds for each session the monitor must not observe
    // on its own, as measured: an E2E run, a sentinel investigation and the
    // monitor's own session (both stamped `console` by the console), an E2E
    // from before the kind existed, and a scripted session with no surface.
    let cases = [
        ("e2e", json!({"e2e_run_id": "r1", "e2e_scenario": "kanban"})),
        (
            "automation",
            json!({"sentinel": true, "sentinel_group_id": "grp_1", "surface": "console"}),
        ),
        (
            "automation",
            json!({"origin": "eval_monitor", "source_session_id": "s", "surface": "console"}),
        ),
        (
            "user",
            json!({"e2e_run_id": "r1", "e2e_execution_kind": "harness_turn", "surface": "console"}),
        ),
        ("user", json!({"parent_session_id": "p", "depth": 1})),
        ("user", json!({})),
        ("user", json!({"surface": "slack"})),
    ];
    for (kind, metadata) in cases {
        let mut world = World::new();
        world.sessions.insert(ROOT.into(), (kind, metadata.clone()));
        let h = Harness::start(world).await;
        h.configure(true).await;
        let skipped = h.end_turn(ROOT, TURN).await;
        assert_eq!(
            skipped.outcome,
            WakeOutcomeV1::NotUserChat,
            "{kind} {metadata}"
        );
        assert!(skipped.evaluation_id.is_none());
        assert_eq!(h.records(), 0, "{kind} {metadata}: nothing admitted");
        // A manual analysis still covers it.
        let manual = runtime::analyze_session(
            &h.deps,
            serde_json::from_value(json!({"session_id": ROOT})).unwrap(),
        )
        .await
        .unwrap();
        assert!(!manual.reused, "{kind} {metadata}");
        assert_eq!(h.records(), 1, "{kind} {metadata}");
    }
    // A plain console chat, with the keys the console writes.
    let mut world = World::new();
    world.sessions.insert(
        ROOT.into(),
        (
            "user",
            json!({"surface": "console", "model": "anthropic::m", "fs_scope": {"root": "/w"},
                "agent_profile": {"id": "default"}}),
        ),
    );
    let h = Harness::start(world).await;
    h.configure(true).await;
    assert_eq!(
        h.end_turn(ROOT, TURN).await.outcome,
        WakeOutcomeV1::Admitted
    );
}

#[tokio::test]
async fn replay_spend_is_reported_but_never_caps_automatic_observation() {
    let (h, evaluation_id) = replayable(World::new(), turn_record(TURN)).await;
    // A cap each of the three samples (0.001) would pass together.
    configure_capped(&h, json!(0.002)).await.unwrap();
    let id = reproduce(
        &h,
        json!({"evaluation_id": evaluation_id, "suggestion_index": 0, "samples": 3, "by": "ana"}),
    )
    .await
    .unwrap()
    .reproduction_id
    .unwrap();
    let replay = settled(&h, &evaluation_id, &id).await;
    assert_eq!(
        replay.state,
        ReproductionStateV1::Completed,
        "{:?}",
        replay.error
    );

    let cost = cost_block(&h).await;
    assert!((cost.today_replay_usd - 0.003).abs() < 1e-9, "{cost:?}");
    assert!((cost.today_usd - 0.003).abs() < 1e-9, "{cost:?}");
    // The analysis reported no cost: unknown, so the capture bucket is empty
    // and says so, instead of the replay filling it.
    assert_eq!(
        (
            cost.today_capture_usd,
            cost.today_unknown,
            cost.today_replay_unknown
        ),
        (0.0, 1, 0)
    );
    assert_eq!((cost.cap_usd, cost.capped), (Some(0.002), false));
    assert_eq!(
        h.end_turn(ROOT, "t_after_replay").await.outcome,
        WakeOutcomeV1::Admitted,
        "replays spent more than the cap, and observation went on"
    );

    // What an analysis spends still counts against the cap.
    h.world().cost_usd = Some(0.06);
    let reanalysis = reanalyze(&h).await;
    investigated(&h, &reanalysis).await;
    let cost = cost_block(&h).await;
    assert_eq!((cost.today_capture_usd, cost.capped), (0.06, true));
    assert!((cost.today_replay_usd - 0.003).abs() < 1e-9, "{cost:?}");
    assert!((cost.today_usd - 0.063).abs() < 1e-9, "{cost:?}");
    assert_eq!(
        h.end_turn(ROOT, "t_after_capture").await.outcome,
        WakeOutcomeV1::CostCap
    );
}

#[tokio::test]
async fn a_replay_sample_without_a_cost_stays_unknown() {
    let mut world = World::new();
    world.sample_cost_usd = None;
    let (h, evaluation_id) = replayable(world, turn_record(TURN)).await;
    let id = reproduce(
        &h,
        json!({"evaluation_id": evaluation_id, "suggestion_index": 0, "samples": 3, "by": "ana"}),
    )
    .await
    .unwrap()
    .reproduction_id
    .unwrap();
    let replay = settled(&h, &evaluation_id, &id).await;
    assert_eq!(replay.samples.len(), 3);
    assert_eq!((replay.cost_usd, replay.cost_unknown_samples), (None, 3));
    let cost = cost_block(&h).await;
    assert_eq!(
        (cost.today_replay_usd, cost.today_replay_unknown),
        (0.0, 3),
        "unknown, not a free replay"
    );
}

#[tokio::test]
async fn a_failed_replay_sample_is_unknown_not_free() {
    let mut world = World::new();
    world.samples_fail = true;
    let (h, evaluation_id) = replayable(world, turn_record(TURN)).await;
    let id = reproduce(
        &h,
        json!({"evaluation_id": evaluation_id, "suggestion_index": 0, "samples": 3, "by": "ana"}),
    )
    .await
    .unwrap()
    .reproduction_id
    .unwrap();
    let replay = settled(&h, &evaluation_id, &id).await;
    assert_eq!(replay.state, ReproductionStateV1::Failed);
    assert_eq!((replay.cost_usd, replay.cost_unknown_samples), (None, 3));
    let cost = cost_block(&h).await;
    assert_eq!(
        (cost.today_replay_usd, cost.today_replay_unknown),
        (0.0, 3),
        "the provider may have billed what the bus gave up on"
    );
}

#[tokio::test]
async fn the_capture_is_the_analyzed_turns_whole_record_even_after_another_turn_ran() {
    let (h, evaluation_id) = replayable(World::new(), turn_record(TURN)).await;
    // The Harness keeps only a session's latest turn: the next one replaces it.
    h.world().state.insert(
        (state::HARNESS_TURN_SCOPE.into(), ROOT.into()),
        turn_record("t_next"),
    );
    let capture = h.result(&evaluation_id).await.assets.capture.unwrap();
    assert_eq!(capture.record, Some(turn_record(TURN)));
    assert_eq!(capture.record_omitted, None);
    // What a reproduction reads is still there, from the copy.
    assert_eq!(capture.options, turn_record(TURN)["options"]);
    let preview = reproduce(
        &h,
        json!({"evaluation_id": evaluation_id, "suggestion_index": 0, "dry_run": true}),
    )
    .await
    .unwrap()
    .preview
    .unwrap();
    assert_eq!(preview.original.signal, Some(true));
}

#[tokio::test]
async fn a_turn_record_too_big_for_the_assets_is_left_out_and_says_why() {
    let mut record = turn_record(TURN);
    // Outside the options: the part a reproduction reads stays small.
    record["result"] = json!("x".repeat(2 * 1024 * 1024));
    let (h, evaluation_id) = replayable(World::new(), record).await;
    let result = h.result(&evaluation_id).await;
    assert_eq!(result.record.status, EvalStatusV1::Completed);
    let capture = result.assets.capture.unwrap();
    assert_eq!(capture.record, None);
    let why = capture.record_omitted.unwrap();
    assert!(why.contains("2097152-byte limit"), "{why}");
    assert_eq!(capture.options, turn_record(TURN)["options"]);
    assert_eq!(capture.turn_id, TURN);
}

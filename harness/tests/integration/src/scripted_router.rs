//! The scripted router: a strict, deterministic implementation of the fixed
//! `router::*` functions. It runs
//! in-process as its own worker connection; the real llm-router is absent so
//! function ownership is unambiguous.
//!
//! `router::chat` consumes generations in ordinal order. An unexpected call
//! or matcher failure records `contract_failure` evidence, streams nothing,
//! and returns a bus error; an unused generation is detected by the runner
//! floor via `generations_consumed`.
//!
//! Deviation from the spec text (folded into the spec amendment): absent or
//! empty `provider` on `models::get` resolves by id alone — that is
//! llm-router's actual contract (`llm-router/src/catalog/queries.rs:36`) and
//! the mandatory context-manager depends on it for token budgeting.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, Mutex};

use iii_sdk::channel::{ChannelWriter, StreamChannelRef};
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;
use crate::matcher::{evaluate, FieldMatchResult};
use crate::types::script::{
    ModelFixtureV1, RouterDispatchV1, RouterScriptV1, ScriptedGenerationV1, ServeEffectV1,
};

/// One `router::chat` call's evidence, stored raw (never normalized).
#[derive(Debug, Clone, serde::Serialize)]
pub struct RouterCallEvidence {
    /// Ordinal of the generation this call was checked against; absent when
    /// the script was already exhausted.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ordinal: Option<u64>,
    pub outcome: CallOutcome,
    /// The raw request payload as received (writer_ref included).
    pub request: Value,
    pub field_results: Vec<FieldMatchResult>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CallOutcome {
    Matched,
    MatchFailed,
    UnexpectedCall,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct RouterAbortEvidence {
    pub request_id: String,
    pub aborted: bool,
}

struct GateState {
    arrivals: usize,
    released: bool,
}

struct State {
    script: RouterScriptV1,
    /// Number of generations consumed so far.
    next: usize,
    /// In match-any mode, the consumed generation indexes are not necessarily
    /// a prefix of the ordinally sorted script.
    consumed: BTreeSet<usize>,
    calls: Vec<RouterCallEvidence>,
    aborts: Vec<RouterAbortEvidence>,
    /// request_id → aborted-once flag, for `router::abort` semantics.
    live: BTreeMap<String, bool>,
    gates: BTreeMap<String, GateState>,
    gate_notify: Arc<tokio::sync::Notify>,
    /// Serve-time captures (name → value) taken from matched requests; frames
    /// reference them as `[[cap:<name>]]`.
    captured: BTreeMap<String, String>,
}

pub struct ScriptedRouter {
    client: Client,
    state: Arc<Mutex<State>>,
}

impl ScriptedRouter {
    /// Connect a dedicated worker and register the six fixed router ids for
    /// the given (already placeholder-expanded) script.
    pub async fn start(ws_url: &str, mut script: RouterScriptV1) -> anyhow::Result<Self> {
        script.generations.sort_by_key(|g| g.ordinal);

        let client = Client::connect(ws_url, "integration-scripted-router");
        let state = Arc::new(Mutex::new(State {
            script,
            next: 0,
            consumed: BTreeSet::new(),
            calls: Vec::new(),
            aborts: Vec::new(),
            live: BTreeMap::new(),
            gates: BTreeMap::new(),
            gate_notify: Arc::new(tokio::sync::Notify::new()),
            captured: BTreeMap::new(),
        }));

        let router = ScriptedRouter { client, state };
        router.register_all();
        Ok(router)
    }

    fn register_all(&self) {
        let iii = self.client.inner();
        let address = iii.address().to_string();

        {
            let state = self.state.clone();
            let address = address.clone();
            let client = iii.clone();
            iii.register_function(
                "router::chat",
                with_router_contract(
                    RegisterFunction::new_async(move |input: Value| {
                        let state = state.clone();
                        let address = address.clone();
                        let client = client.clone();
                        async move { chat(state, address, client, input).await }
                    }),
                    "router::chat",
                )
                .metadata(json!({ "internal": true })),
            );
        }
        {
            let state = self.state.clone();
            iii.register_function(
                "router::abort",
                with_router_contract(
                    RegisterFunction::new_async(move |input: Value| {
                        let state = state.clone();
                        async move {
                            let request_id = input
                                .get("request_id")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string();
                            let mut state = state.lock().expect("router state");
                            let aborted = match state.live.get_mut(&request_id) {
                                Some(already @ false) => {
                                    *already = true;
                                    true
                                }
                                _ => false,
                            };
                            state.aborts.push(RouterAbortEvidence {
                                request_id,
                                aborted,
                            });
                            Ok::<Value, Error>(json!({ "aborted": aborted }))
                        }
                    }),
                    "router::abort",
                )
                .metadata(json!({ "internal": true })),
            );
        }
        {
            let state = self.state.clone();
            iii.register_function(
                "router::models::list",
                with_router_contract(
                    RegisterFunction::new_async(move |input: Value| {
                        let state = state.clone();
                        async move {
                            let model = fixture_model(&state);
                            let provider = input.get("provider").and_then(Value::as_str);
                            let capability = input.get("capability").and_then(Value::as_str);
                            let matches_provider =
                                provider.is_none_or(|p| p.is_empty() || p == model.provider);
                            let matches_capability =
                                capability.is_none_or(|c| model_supports(&model, c));
                            let models: Vec<ModelFixtureV1> =
                                if matches_provider && matches_capability {
                                    vec![model]
                                } else {
                                    vec![]
                                };
                            Ok::<Value, Error>(json!({ "models": models }))
                        }
                    }),
                    "router::models::list",
                ),
            );
        }
        {
            let state = self.state.clone();
            iii.register_function(
                "router::models::get",
                with_router_contract(
                    RegisterFunction::new_async(move |input: Value| {
                        let state = state.clone();
                        async move {
                            let model = fixture_model(&state);
                            let provider = input.get("provider").and_then(Value::as_str);
                            let id = input.get("id").and_then(Value::as_str).unwrap_or_default();
                            let provider_ok =
                                provider.is_none_or(|p| p.is_empty() || p == model.provider);
                            if provider_ok && id == model.id {
                                Ok::<Value, Error>(json!({ "model": model }))
                            } else {
                                Ok(Value::Null)
                            }
                        }
                    }),
                    "router::models::get",
                ),
            );
        }
        {
            let state = self.state.clone();
            iii.register_function(
                "router::models::supports",
                with_router_contract(
                    RegisterFunction::new_async(move |input: Value| {
                        let state = state.clone();
                        async move {
                            let model = fixture_model(&state);
                            let provider = input
                                .get("provider")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let id = input.get("id").and_then(Value::as_str).unwrap_or_default();
                            let capability = input
                                .get("capability")
                                .and_then(Value::as_str)
                                .unwrap_or_default();
                            let supported = (provider.is_empty() || provider == model.provider)
                                && id == model.id
                                && model_supports(&model, capability);
                            Ok::<Value, Error>(json!({ "supported": supported }))
                        }
                    }),
                    "router::models::supports",
                ),
            );
        }
        iii.register_function(
            "router::system_prompt::get",
            with_router_contract(
                RegisterFunction::new_async(move |_input: Value| async move {
                    // `system_prompt` omitted: the harness falls back to its
                    // checked-in built-in prompt.
                    Ok::<Value, Error>(json!({ "provider": "scripted" }))
                }),
                "router::system_prompt::get",
            ),
        );
    }

    /// Number of generations consumed so far.
    pub fn generations_consumed(&self) -> u64 {
        self.state.lock().expect("router state").next as u64
    }

    pub fn total_generations(&self) -> u64 {
        self.state
            .lock()
            .expect("router state")
            .script
            .generations
            .len() as u64
    }

    /// Full call evidence for `router-calls.json`.
    pub fn evidence(&self) -> Value {
        let state = self.state.lock().expect("router state");
        json!({
            "calls": state.calls,
            "aborts": state.aborts,
            "generations_consumed": state.next,
            "generations_total": state.script.generations.len(),
        })
    }

    /// Wait until `count` scripted calls have entered the named gate.
    pub async fn wait_for_gate(
        &self,
        name: &str,
        count: usize,
        deadline: Deadline,
    ) -> anyhow::Result<()> {
        wait_for_gate_state(&self.state, name, count, deadline).await
    }

    /// Release one named gate. Releasing an absent gate is a caller error: it
    /// would turn a fixture typo into a test that proceeds without its hard
    /// synchronization point.
    pub fn release_gate(&self, name: &str) -> anyhow::Result<()> {
        let notify = {
            let mut state = self.state.lock().expect("router state");
            let gate = state.gates.get_mut(name).ok_or_else(|| {
                anyhow::anyhow!("scripted router gate {name:?} was never entered")
            })?;
            gate.released = true;
            Arc::clone(&state.gate_notify)
        };
        notify.notify_waiters();
        Ok(())
    }

    /// Unblock all gates during teardown, including after an intervention
    /// failure, so no scripted-router handler remains parked on shutdown.
    pub fn release_all_gates(&self) {
        let notify = {
            let mut state = self.state.lock().expect("router state");
            for gate in state.gates.values_mut() {
                gate.released = true;
            }
            Arc::clone(&state.gate_notify)
        };
        notify.notify_waiters();
    }

    /// True when any call failed to match or arrived unexpected.
    pub fn contract_failed(&self) -> bool {
        self.state
            .lock()
            .expect("router state")
            .calls
            .iter()
            .any(|c| c.outcome != CallOutcome::Matched)
    }

    pub async fn shutdown(&self) {
        self.release_all_gates();
        self.client.shutdown().await;
    }
}

fn with_router_contract(registration: RegisterFunction, function_id: &str) -> RegisterFunction {
    let surface = router_surface(function_id);
    registration
        .description(surface.description)
        .request_format(surface.request_schema)
        .response_format(surface.response_schema)
}

struct RouterFunctionSurface {
    description: String,
    request_schema: Value,
    response_schema: Value,
}

fn router_surface(function_id: &str) -> RouterFunctionSurface {
    router_surfaces()
        .into_iter()
        .find(|(id, _)| id == function_id)
        .map(|(_, surface)| surface)
        .unwrap_or_else(|| panic!("no scripted-router golden for {function_id}"))
}

fn router_surfaces() -> Vec<(String, RouterFunctionSurface)> {
    [
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../llm-router/tests/golden/schemas/router.chat.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../llm-router/tests/golden/schemas/router.abort.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../llm-router/tests/golden/schemas/router.models.list.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../llm-router/tests/golden/schemas/router.models.get.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../llm-router/tests/golden/schemas/router.models.supports.json"
        )),
        include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../llm-router/tests/golden/schemas/router.system_prompt.get.json"
        )),
    ]
    .into_iter()
    .map(router_surface_from_golden)
    .collect()
}

fn router_surface_from_golden(raw: &str) -> (String, RouterFunctionSurface) {
    let value: Value = serde_json::from_str(raw).expect("checked-in function golden is JSON");
    let function_id = value
        .get("function_id")
        .and_then(Value::as_str)
        .expect("router golden must declare function_id")
        .to_string();
    let surface = RouterFunctionSurface {
        description: value
            .get("description")
            .and_then(Value::as_str)
            .expect("router golden must declare a description")
            .to_string(),
        request_schema: value
            .get("request_schema")
            .cloned()
            .expect("router golden must declare a request schema"),
        response_schema: value
            .get("response_schema")
            .cloned()
            .expect("router golden must declare a response schema"),
    };
    (function_id, surface)
}

fn fixture_model(state: &Arc<Mutex<State>>) -> ModelFixtureV1 {
    state.lock().expect("router state").script.model.clone()
}

/// Mirror of `model_supports` (`llm-router/src/catalog/queries.rs:7`):
/// unknown capability or model → false.
fn model_supports(model: &ModelFixtureV1, capability: &str) -> bool {
    match capability {
        "tools" => model.supports_tools == Some(true),
        "vision" => model.supports_vision == Some(true),
        "cache" => model.supports_cache == Some(true),
        "structured_output" => model.supports_structured_output == Some(true),
        "thinking" | "thinking:low" | "thinking:medium" | "thinking:high" => {
            model.supports_thinking == Some(true)
        }
        "thinking:xhigh" => model.supports_xhigh == Some(true),
        _ => false,
    }
}

async fn wait_for_gate_state(
    state: &Arc<Mutex<State>>,
    name: &str,
    count: usize,
    deadline: Deadline,
) -> anyhow::Result<()> {
    anyhow::ensure!(count > 0, "gate arrival count must be positive");
    let notify = {
        let state = state.lock().expect("router state");
        Arc::clone(&state.gate_notify)
    };

    loop {
        // Create the future before reading arrivals. `notify_waiters` only
        // wakes futures that already exist; the state check below handles an
        // arrival that happened before this future was created.
        let notified = notify.notified();
        let arrivals = {
            let state = state.lock().expect("router state");
            state.gates.get(name).map_or(0, |gate| gate.arrivals)
        };
        if arrivals >= count {
            return Ok(());
        }
        deadline
            .timeout(format!("scripted router gate {name}"), notified)
            .await?;
    }
}

async fn chat(
    state: Arc<Mutex<State>>,
    address: String,
    client: Arc<IIIClient>,
    input: Value,
) -> Result<Value, Error> {
    // Match under the lock; stream outside it.
    let (generation, writer_ref, request_id) = {
        let mut state = state.lock().expect("router state");
        let candidates: Vec<usize> = match state.script.dispatch {
            RouterDispatchV1::Ordinal => {
                if state.next < state.script.generations.len() {
                    vec![state.next]
                } else {
                    Vec::new()
                }
            }
            RouterDispatchV1::MatchAny => state
                .script
                .generations
                .iter()
                .enumerate()
                .filter_map(|(index, _)| (!state.consumed.contains(&index)).then_some(index))
                .collect(),
        };

        if candidates.is_empty() {
            state.calls.push(RouterCallEvidence {
                ordinal: None,
                outcome: CallOutcome::UnexpectedCall,
                request: input.clone(),
                field_results: Vec::new(),
            });
            return Err(Error::Handler(
                "integration/unexpected_call: router::chat after script exhausted".into(),
            ));
        };

        let mut selected = None;
        let mut first_failure = None;
        for index in candidates {
            let generation = state.script.generations[index].clone();
            let field_results: Vec<FieldMatchResult> = generation
                .match_
                .fields()
                .iter()
                .map(|(field, matcher)| evaluate(field, matcher, input.get(*field)))
                .collect();
            if field_results.iter().all(|result| result.passed) {
                selected = Some((index, generation, field_results));
                break;
            }
            if first_failure.is_none() {
                first_failure = Some((index, generation, field_results));
            }
        }

        let Some((index, generation, field_results)) = selected else {
            let (_index, generation, field_results) = first_failure.expect("candidate exists");
            let failed: Vec<&FieldMatchResult> = field_results
                .iter()
                .filter(|result| !result.passed)
                .collect();
            let detail = failed
                .iter()
                .map(|r| format!("{}: {}", r.field, r.detail))
                .collect::<Vec<_>>()
                .join("; ");
            state.calls.push(RouterCallEvidence {
                ordinal: Some(generation.ordinal),
                outcome: CallOutcome::MatchFailed,
                request: input.clone(),
                field_results,
            });
            return Err(Error::Handler(format!(
                "integration/match_failed: generation {}: {detail}",
                generation.ordinal
            )));
        };

        let writer_ref: StreamChannelRef =
            serde_json::from_value(input.get("writer_ref").cloned().unwrap_or(Value::Null))
                .map_err(|e| Error::Handler(format!("integration/writer_ref: {e}")))?;
        let request_id = input
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        state.next += 1;
        state.consumed.insert(index);
        state.live.insert(request_id.clone(), false);
        state.calls.push(RouterCallEvidence {
            ordinal: Some(generation.ordinal),
            outcome: CallOutcome::Matched,
            request: input.clone(),
            field_results,
        });
        // Serve-time captures: pull runtime values (e.g. a `sub_…` id from a
        // function_result in the history) out of the matched request so later
        // frames can echo them via `[[cap:<name>]]`.
        if !generation.captures.is_empty() {
            let raw = serde_json::to_string(&input)
                .map_err(|e| Error::Handler(format!("integration/capture_serialize: {e}")))?;
            for capture in &generation.captures {
                let regex = regex::Regex::new(&capture.pattern).map_err(|e| {
                    Error::Handler(format!("integration/capture_regex {}: {e}", capture.name))
                })?;
                let value = regex
                    .captures(&raw)
                    .and_then(|c| c.get(1))
                    .map(|m| m.as_str().to_string())
                    .ok_or_else(|| {
                        Error::Handler(format!(
                            "integration/capture_missed: generation {} capture {} found no \
                             match in the request",
                            generation.ordinal, capture.name
                        ))
                    })?;
                state.captured.insert(capture.name.clone(), value);
            }
        }
        (generation, writer_ref, request_id)
    };

    if let Some(gate) = generation.gate.as_ref() {
        await_gate(&state, &gate.name).await;
    }

    // Serve-time side effect, BEFORE streaming or failing: the harness is now
    // blocked awaiting this generation with its turn `Running`, so an awaited
    // steer parks in the turn's queue before the outcome proceeds.
    if let Some(effect) = &generation.on_serve {
        perform_serve_effect(&client, effect).await?;
    }

    // Scripted failure: the generation is consumed and recorded as matched;
    // the error is the scripted subject behavior, not a contract violation.
    if let Some(message) = &generation.failure {
        state.lock().expect("router state").live.remove(&request_id);
        return Err(Error::Handler(format!(
            "integration/scripted_failure: {message}"
        )));
    }

    let stream_result =
        stream_frames(&address, &writer_ref, &generation, &state, &request_id).await;

    state.lock().expect("router state").live.remove(&request_id);

    stream_result?;
    // Response only after terminal streaming has been relayed.
    serde_json::to_value(&generation.response)
        .map_err(|e| Error::Handler(format!("integration/response_serialize: {e}")))
}

async fn await_gate(state: &Arc<Mutex<State>>, name: &str) {
    let notify = {
        let mut state = state.lock().expect("router state");
        let notify = Arc::clone(&state.gate_notify);
        let gate = state.gates.entry(name.to_string()).or_insert(GateState {
            arrivals: 0,
            released: false,
        });
        gate.arrivals += 1;
        notify
    };
    // `wait_for_gate` observes arrivals through this notification. Keep the
    // state mutation and notification separate so no lock is held while
    // waking the runner.
    notify.notify_waiters();
    loop {
        // Register before checking the shared state so a release cannot land
        // between the check and creation of the future.
        let notified = notify.notified();
        let released = {
            let state = state.lock().expect("router state");
            state.gates.get(name).is_some_and(|gate| gate.released)
        };
        if released {
            return;
        }
        notified.await;
        // The state is re-checked on every notification. `Notify` is used as a
        // wake-up only, so release_all_gates and a named release share the same
        // race-free state transition.
    }
}

/// Run a generation's serve-time side effect. The steer is *awaited*:
/// `harness::send` enqueues the parked row before it returns, so once this
/// resolves the message is durably queued and the subsequent generation
/// outcome (frames or scripted failure) drives the turn into finalize with
/// that row present. Outer timeout as in `Client::call_with_timeout`: the
/// SDK timeout covers the engine round-trip, but a connection that never
/// establishes can park the future — and with it this `chat` handler.
async fn perform_serve_effect(client: &IIIClient, effect: &ServeEffectV1) -> Result<(), Error> {
    let steer = &effect.steer;
    let outer = std::time::Duration::from_millis(DEFAULT_CALL_TIMEOUT_MS + 5_000);
    match tokio::time::timeout(
        outer,
        client.trigger(TriggerRequest {
            function_id: "harness::send".to_string(),
            payload: json!({
                "session_id": steer.session_id,
                "message": steer.message,
            }),
            action: None,
            timeout_ms: Some(DEFAULT_CALL_TIMEOUT_MS),
        }),
    )
    .await
    {
        Ok(Ok(_)) => Ok(()),
        Ok(Err(e)) => Err(Error::Handler(format!(
            "integration/serve_effect harness::send: {e}"
        ))),
        Err(_) => Err(Error::Handler(format!(
            "integration/serve_effect harness::send: no response within {}ms",
            outer.as_millis()
        ))),
    }
}

async fn stream_frames(
    address: &str,
    writer_ref: &StreamChannelRef,
    generation: &ScriptedGenerationV1,
    state: &Arc<Mutex<State>>,
    request_id: &str,
) -> Result<(), Error> {
    let writer = ChannelWriter::new(address, writer_ref);
    let captured = state.lock().expect("router state").captured.clone();
    for frame in &generation.frames {
        if request_aborted(state, request_id) {
            writer.close().await?;
            return Err(Error::Handler(format!(
                "integration/aborted: request {request_id}"
            )));
        }
        let mut text = serde_json::to_string(frame)
            .map_err(|e| Error::Handler(format!("integration/frame_serialize: {e}")))?;
        // Late-bind serve-time captures. `[[cap:…]]` (not `{{…}}`) because
        // placeholder expansion is strict and runs before any capture exists.
        if text.contains("[[cap:") {
            for (name, value) in &captured {
                text = text.replace(&format!("[[cap:{name}]]"), value);
            }
            if let Some(start) = text.find("[[cap:") {
                let tail: String = text[start..].chars().take(48).collect();
                return Err(Error::Handler(format!(
                    "integration/capture_unresolved: frame references an uncaptured value \
                     near {tail:?}"
                )));
            }
        }
        writer.send_message(&text).await?;
    }
    if request_aborted(state, request_id) {
        writer.close().await?;
        return Err(Error::Handler(format!(
            "integration/aborted: request {request_id}"
        )));
    }
    writer.close().await?;
    Ok(())
}

fn request_aborted(state: &Arc<Mutex<State>>, request_id: &str) -> bool {
    state
        .lock()
        .expect("router state")
        .live
        .get(request_id)
        .copied()
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::types::script::{RouterDispatchV1, SchemaVersion1};

    fn state() -> Arc<Mutex<State>> {
        Arc::new(Mutex::new(State {
            script: RouterScriptV1 {
                schema_version: SchemaVersion1::V1,
                scenario_id: "test".to_string(),
                model: ModelFixtureV1 {
                    id: "fixture-model".to_string(),
                    provider: "scripted".to_string(),
                    context_window: 1,
                    max_output_tokens: 1,
                    supports_thinking: Some(false),
                    supports_xhigh: None,
                    supports_tools: Some(false),
                    supports_vision: Some(false),
                    supports_cache: Some(false),
                    supports_structured_output: Some(false),
                },
                dispatch: RouterDispatchV1::Ordinal,
                generations: Vec::new(),
            },
            next: 0,
            consumed: BTreeSet::new(),
            calls: Vec::new(),
            aborts: Vec::new(),
            live: BTreeMap::new(),
            gates: BTreeMap::new(),
            gate_notify: Arc::new(tokio::sync::Notify::new()),
            captured: BTreeMap::new(),
        }))
    }

    #[tokio::test]
    async fn gate_arrival_notifies_waiters() {
        let state = state();
        let notify = Arc::clone(&state.lock().expect("router state").gate_notify);
        let notified = notify.notified();
        let gate_state = Arc::clone(&state);
        let gate = tokio::spawn(async move { await_gate(&gate_state, "test-gate").await });

        tokio::time::timeout(Duration::from_secs(1), notified)
            .await
            .expect("gate arrival should wake waiters");

        gate.abort();
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn gate_waiter_and_release_check_shared_state_after_wakeup() {
        let state = state();
        let waiter_state = Arc::clone(&state);
        let waiter = tokio::spawn(async move {
            wait_for_gate_state(
                &waiter_state,
                "test-gate",
                1,
                Deadline::after(Duration::from_secs(1)),
            )
            .await
        });

        tokio::task::yield_now().await;

        let arrival_state = Arc::clone(&state);
        let arrival = tokio::spawn(async move { await_gate(&arrival_state, "test-gate").await });

        waiter
            .await
            .expect("gate waiter task should finish")
            .expect("gate arrival should be observed");

        let notify = {
            let mut state = state.lock().expect("router state");
            state
                .gates
                .get_mut("test-gate")
                .expect("gate should exist")
                .released = true;
            Arc::clone(&state.gate_notify)
        };
        notify.notify_waiters();

        tokio::time::timeout(Duration::from_secs(1), arrival)
            .await
            .expect("gate release should wake the router")
            .expect("router gate task should finish");
    }
}

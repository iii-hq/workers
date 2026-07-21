//! The scripted router: a strict, deterministic implementation of the fixed
//! `router::*` functions. It runs
//! in-process as its own worker connection; the real llm-router is absent so
//! function ownership is unambiguous.
//!
//! `router::chat` consumes generations in ordinal order. An unexpected call
//! or matcher failure records `contract_failure` evidence, streams nothing,
//! and returns a bus error; an unused expectation is detected by the grader
//! via `generations_consumed`.
//!
//! Deviation from the spec text (folded into the spec amendment): absent or
//! empty `provider` on `models::get` resolves by id alone — that is
//! llm-router's actual contract (`llm-router/src/catalog/queries.rs:36`) and
//! the mandatory context-manager depends on it for token budgeting.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use iii_sdk::channel::{ChannelWriter, StreamChannelRef};
use iii_sdk::errors::Error;
use iii_sdk::RegisterFunction;
use serde_json::{json, Value};

use crate::client::Client;
use crate::matcher::{evaluate, FieldMatchResult};
use crate::types::script::{ModelFixtureV1, RouterScriptV1, ScriptedGenerationV1};

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

struct State {
    script: RouterScriptV1,
    /// Generations sorted by ordinal; index of the next one to consume.
    next: usize,
    calls: Vec<RouterCallEvidence>,
    /// request_id → aborted-once flag, for `router::abort` semantics.
    live: BTreeMap<String, bool>,
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
            calls: Vec::new(),
            live: BTreeMap::new(),
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
            iii.register_function(
                "router::chat",
                with_router_contract(
                    RegisterFunction::new_async(move |input: Value| {
                        let state = state.clone();
                        let address = address.clone();
                        async move { chat(state, address, input).await }
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
            "generations_consumed": state.next,
            "generations_total": state.script.generations.len(),
        })
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
        self.client.shutdown().await;
    }
}

fn with_router_contract(registration: RegisterFunction, function_id: &str) -> RegisterFunction {
    let contract = crate::readiness::router_contract(function_id);
    registration
        .description(
            contract
                .description
                .expect("router golden must declare a description"),
        )
        .request_format(
            contract
                .request_schema
                .expect("router golden must declare a request schema"),
        )
        .response_format(
            contract
                .response_schema
                .expect("router golden must declare a response schema"),
        )
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

async fn chat(state: Arc<Mutex<State>>, address: String, input: Value) -> Result<Value, Error> {
    // Match under the lock; stream outside it.
    let (generation, writer_ref, request_id) = {
        let mut state = state.lock().expect("router state");
        let index = state.next;
        let generation = state.script.generations.get(index).cloned();

        let Some(generation) = generation else {
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

        let field_results: Vec<FieldMatchResult> = generation
            .match_
            .fields()
            .iter()
            .map(|(field, matcher)| evaluate(field, matcher, input.get(*field)))
            .collect();
        let failed: Vec<&FieldMatchResult> = field_results.iter().filter(|r| !r.passed).collect();

        if !failed.is_empty() {
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
        }

        let writer_ref: StreamChannelRef =
            serde_json::from_value(input.get("writer_ref").cloned().unwrap_or(Value::Null))
                .map_err(|e| Error::Handler(format!("integration/writer_ref: {e}")))?;
        let request_id = input
            .get("request_id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        state.next = index + 1;
        state.live.insert(request_id.clone(), false);
        state.calls.push(RouterCallEvidence {
            ordinal: Some(generation.ordinal),
            outcome: CallOutcome::Matched,
            request: input.clone(),
            field_results,
        });
        (generation, writer_ref, request_id)
    };

    let stream_result =
        stream_frames(&address, &writer_ref, &generation, &state, &request_id).await;

    state.lock().expect("router state").live.remove(&request_id);

    stream_result?;
    // Response only after terminal streaming has been relayed.
    serde_json::to_value(&generation.response)
        .map_err(|e| Error::Handler(format!("integration/response_serialize: {e}")))
}

async fn stream_frames(
    address: &str,
    writer_ref: &StreamChannelRef,
    generation: &ScriptedGenerationV1,
    state: &Arc<Mutex<State>>,
    request_id: &str,
) -> Result<(), Error> {
    let writer = ChannelWriter::new(address, writer_ref);
    for frame in &generation.frames {
        if request_aborted(state, request_id) {
            writer.close().await?;
            return Err(Error::Handler(format!(
                "integration/aborted: request {request_id}"
            )));
        }
        let text = serde_json::to_string(frame)
            .map_err(|e| Error::Handler(format!("integration/frame_serialize: {e}")))?;
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

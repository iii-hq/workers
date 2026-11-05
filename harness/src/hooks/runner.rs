//! The in-path hook chain (harness.md § Chain, hold, and failure semantics).
//!
//! The harness calls a point's hook function with the hook input; the
//! function's deliver hook ([`deliver`]) runs the chain over the observers
//! that matched the call: in priority order, each called through its observer
//! and each seeing the input as mutated by the previous; the first
//! `deny`/`hold` short-circuits. Failures (an error, a timeout, a call the
//! engine refuses such as `OVERFLOW` or `OBSERVER_NOT_ACTIVE`) resolve by
//! `on_error` (fail-closed `pre_*` deny, fail-open `post_*` skip). The merged
//! outcome ([`HookChainOutcome`]) is what the harness gets back. Mutations are
//! recorded into the written entry's `origin` via the returned annotations.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use globset::{Glob, GlobSetBuilder};
use iii_sdk::{Call, CallRequest, IIIClient, Observer};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::{ordered, HookBinding, HookPoint, HookRegistry};
use crate::types::content::ContentBlock;
use crate::types::turn::TurnRecord;

/// Bounds the harness's call of a hook function. Each target call inside the
/// chain has its own `timeout_ms`; this only catches a chain that never
/// answers.
const CHAIN_TIMEOUT_MS: u64 = 600_000;

/// Mutations a `continue` hook may return.
#[derive(Default)]
struct HookMutations {
    system_prompt: Option<String>,
    append_messages: Vec<Value>,
    arguments: Option<Value>,
    content: Option<Vec<ContentBlock>>,
    details: Option<Value>,
    is_error: Option<bool>,
    annotations: Map<String, Value>,
}

enum HookOutcome {
    Continue(HookMutations),
    Deny(String),
    // A holding hook may mutate as it holds (approval-gate: park the call AND
    // stamp validated context onto it) — its mutations must not be dropped.
    Hold(HookMutations),
}

/// What the chain decided.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChainDecision {
    #[default]
    Continue,
    Deny,
    Hold,
}

/// The merged outcome a hook function returns to the harness.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HookChainOutcome {
    pub decision: ChainDecision,
    /// `deny`: why.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// `hold`: the target function of the holding observer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub held_by: Option<String>,
    /// on-pre-generate: the system prompt, when a hook changed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// on-pre-generate: messages appended by the chain.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub append_messages: Vec<Value>,
    /// on-pre-call: the arguments as the chain left them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<Value>,
    /// on-post-call: `{ content, details, is_error }` as the chain left it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// on-post-turn deny: the corrective prompt, from the validator's
    /// `retry_prompt`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    pub annotations: Map<String, Value>,
}

impl HookChainOutcome {
    fn deny(reason: String) -> Self {
        Self {
            decision: ChainDecision::Deny,
            reason: Some(reason),
            ..Self::default()
        }
    }
}

/// Outcome of the `pre_generate` chain.
pub enum PreGenerateOutcome {
    Continue {
        system_prompt: Option<String>,
        append_messages: Vec<Value>,
        annotations: Map<String, Value>,
    },
    Deny(String),
}

/// Outcome of the `pre_call` chain.
pub enum PreCallOutcome {
    Continue {
        arguments: Value,
        annotations: Map<String, Value>,
    },
    Deny(String),
    Hold {
        held_by: String,
        /// Arguments as mutated by the chain up to (not including) the holder,
        /// checkpointed so a release executes the mutated call (issue #506).
        arguments: Value,
        annotations: Map<String, Value>,
    },
}

pub enum PostCallOutcome {
    Result {
        result: crate::call::ResultData,
        annotations: Map<String, Value>,
    },
    Hold {
        held_by: String,
        annotations: Map<String, Value>,
    },
}

impl HookRegistry {
    fn envelope(&self, point: HookPoint, record: &TurnRecord, step: u64) -> Value {
        let mut v = serde_json::json!({
            "point": point.input_name(),
            "session_id": record.session_id,
            "turn_id": record.turn_id,
            "step": step,
            "depth": record.depth,
        });
        if let Some(m) = &record.options.metadata {
            v["metadata"] = m.clone();
        }
        v
    }

    /// What static `inject_prompt` observers are selected against for a
    /// turn's step: its pre_generate hook input, whose `generate` carries
    /// what is known before assembly (`model`, `provider`, `expose`) and not
    /// the assembled prompt, messages or tools those observers feed.
    pub fn static_injection_input(&self, record: &TurnRecord, step: u64) -> Value {
        let mut input = self.envelope(HookPoint::PreGenerate, record, step);
        input["generate"] = serde_json::json!({
            "model": record.options.model,
            "provider": record.options.provider.clone().unwrap_or_default(),
            "expose": record
                .options
                .functions
                .as_ref()
                .map(|functions| functions.expose)
                .unwrap_or_default(),
        });
        input
    }

    /// Call `point`'s hook function with `input` and read the merged
    /// outcome. `None` when nobody observes the point: the chain is empty.
    /// A failed call resolves by the point's default `on_error`.
    async fn call_point(&self, point: HookPoint, input: Value) -> Option<HookChainOutcome> {
        if !self.iii.is_observed(point.function_id()) {
            return None;
        }
        let request = CallRequest {
            function_id: point.function_id().to_string(),
            payload: input,
            action: None,
            timeout_ms: Some(CHAIN_TIMEOUT_MS),
        };
        let failure = match self.iii.call(request).await {
            Ok(value) => match serde_json::from_value::<HookChainOutcome>(value) {
                Ok(outcome) => return Some(outcome),
                Err(error) => format!("{} answered no chain outcome: {error}", point.function_id()),
            },
            Err(error) => format!("{} failed: {error}", point.function_id()),
        };
        if point.default_fail_closed() {
            tracing::warn!(point = point.input_name(), %failure, "hook chain failed closed (deny)");
            Some(HookChainOutcome::deny(failure))
        } else {
            tracing::warn!(point = point.input_name(), %failure, "hook chain failed open (skip)");
            None
        }
    }

    /// `pre_turn`: veto only. `Err(reason)` ends the turn.
    pub async fn run_pre_turn(&self, record: &TurnRecord, step: u64) -> Result<(), String> {
        let input = self.envelope(HookPoint::PreTurn, record, step);
        match self.call_point(HookPoint::PreTurn, input).await {
            Some(outcome) if outcome.decision == ChainDecision::Deny => Err(outcome
                .reason
                .unwrap_or_else(|| "denied by hook".to_string())),
            _ => Ok(()),
        }
    }

    /// `pre_generate`: extend the system prompt and append bounded messages, or
    /// veto. Each hook sees the prompt/messages as mutated by the previous.
    /// Observers with a static `inject_prompt` are not called: the turn loop
    /// applied them before assembly ([`with_static_prompts`]).
    #[allow(clippy::too_many_arguments)]
    pub async fn run_pre_generate(
        &self,
        record: &TurnRecord,
        step: u64,
        base_system_prompt: Option<String>,
        base_messages: &[Value],
        functions_generation: u64,
        tools: &[crate::types::model::AgentFunction],
    ) -> PreGenerateOutcome {
        let mut input = self.envelope(HookPoint::PreGenerate, record, step);
        input["generate"] = serde_json::json!({
            "system_prompt": base_system_prompt.clone().unwrap_or_default(),
            "messages": base_messages,
            "model": record.options.model,
            "provider": record.options.provider.clone().unwrap_or_default(),
            "functions_generation": functions_generation,
            "tools": tools,
            // Hooks always receive the concrete decision tools, so the
            // exposure mode is not inferable from `tools`; injectors need
            // it to phrase call instructions the model can actually follow.
            "expose": record
                .options
                .functions
                .as_ref()
                .map(|functions| functions.expose)
                .unwrap_or_default(),
        });
        match self.call_point(HookPoint::PreGenerate, input).await {
            None => PreGenerateOutcome::Continue {
                system_prompt: base_system_prompt,
                append_messages: Vec::new(),
                annotations: Map::new(),
            },
            Some(outcome) if outcome.decision == ChainDecision::Deny => PreGenerateOutcome::Deny(
                outcome
                    .reason
                    .unwrap_or_else(|| "denied by hook".to_string()),
            ),
            Some(outcome) => PreGenerateOutcome::Continue {
                system_prompt: outcome.system_prompt.or(base_system_prompt),
                append_messages: outcome.append_messages,
                annotations: outcome.annotations,
            },
        }
    }

    /// `post_generate`: observe only (usage accounting, safety logging).
    pub async fn run_post_generate(&self, record: &TurnRecord, step: u64, message: Value) {
        let mut input = self.envelope(HookPoint::PostGenerate, record, step);
        input["generated"] = serde_json::json!({ "message": message });
        let _ = self.call_point(HookPoint::PostGenerate, input).await;
    }

    /// `pre_call`: deny / hold / rewrite arguments. Only observers whose
    /// `functions` globs match the called function are consulted.
    /// `resume_after` is the hook-held release path (harness.md §
    /// `function::resolve`): the chain resumes AFTER the named holder (see
    /// [`chain_slice`]), over the checkpointed arguments it already mutated.
    pub async fn run_pre_call(
        &self,
        record: &TurnRecord,
        step: u64,
        call_id: &str,
        function_id: &str,
        arguments: &Value,
        resume_after: Option<&str>,
    ) -> PreCallOutcome {
        let mut input = self.envelope(HookPoint::PreCall, record, step);
        input["call"] = serde_json::json!({
            "id": call_id,
            "function_id": function_id,
            "arguments": arguments,
        });
        if let Some(holder) = resume_after {
            input[RESUME_AFTER] = Value::String(holder.to_string());
        }
        let Some(outcome) = self.call_point(HookPoint::PreCall, input).await else {
            return PreCallOutcome::Continue {
                arguments: arguments.clone(),
                annotations: Map::new(),
            };
        };
        match outcome.decision {
            ChainDecision::Deny => PreCallOutcome::Deny(
                outcome
                    .reason
                    .unwrap_or_else(|| "denied by hook".to_string()),
            ),
            ChainDecision::Hold => PreCallOutcome::Hold {
                held_by: outcome.held_by.unwrap_or_default(),
                arguments: outcome.arguments.unwrap_or_else(|| arguments.clone()),
                annotations: outcome.annotations,
            },
            ChainDecision::Continue => PreCallOutcome::Continue {
                arguments: outcome.arguments.unwrap_or_else(|| arguments.clone()),
                annotations: outcome.annotations,
            },
        }
    }

    /// `post_call`: rewrite result content / details / is_error (redaction,
    /// truncation). Returns the (possibly rewritten) result + annotations.
    pub async fn run_post_call(
        &self,
        record: &TurnRecord,
        step: u64,
        call_id: &str,
        function_id: &str,
        arguments: &Value,
        mut result: crate::call::ResultData,
    ) -> PostCallOutcome {
        let mut input = self.envelope(HookPoint::PostCall, record, step);
        input["call"] = serde_json::json!({
            "id": call_id,
            "function_id": function_id,
            "arguments": arguments,
        });
        input["result"] = serde_json::json!({
            "function_call_id": call_id,
            "function_id": function_id,
            "content": result.content,
            "is_error": result.is_error,
            "details": result.details,
        });
        let Some(outcome) = self.call_point(HookPoint::PostCall, input).await else {
            return PostCallOutcome::Result {
                result,
                annotations: Map::new(),
            };
        };
        if outcome.decision == ChainDecision::Hold {
            return PostCallOutcome::Hold {
                held_by: outcome.held_by.unwrap_or_default(),
                annotations: outcome.annotations,
            };
        }
        if let Some(chained) = outcome.result {
            if let Some(content) = chained
                .get("content")
                .and_then(|c| serde_json::from_value(c.clone()).ok())
            {
                result.content = content;
            }
            if let Some(details) = chained.get("details") {
                result.details = details.clone();
            }
            if let Some(is_error) = chained.get("is_error").and_then(Value::as_bool) {
                result.is_error = is_error;
            }
        }
        PostCallOutcome::Result {
            result,
            annotations: outcome.annotations,
        }
    }

    /// `post_turn`: gate completion on the contract-valid result. Observers
    /// are scoped by `sessions` globs. A template observer (`payload` set)
    /// calls a plain composition function (fp::pipe) with the result injected
    /// at `result_into` and reads its receipt as the verdict; an input
    /// observer speaks the ordinary hook contract (`deny` blocks). A deny
    /// re-prompts the turn through the validation-retry budget; `prompt`
    /// (from the observer's `retry_prompt`) replaces the generic nudge
    /// verbatim.
    pub async fn run_post_turn(
        &self,
        record: &TurnRecord,
        step: u64,
        result: &Value,
    ) -> Result<(), PostTurnDeny> {
        let mut input = self.envelope(HookPoint::PostTurn, record, step);
        input["result"] = result.clone();
        match self.call_point(HookPoint::PostTurn, input).await {
            Some(outcome) if outcome.decision == ChainDecision::Deny => Err(PostTurnDeny {
                reason: outcome
                    .reason
                    .unwrap_or_else(|| "denied by hook".to_string()),
                prompt: outcome.prompt,
            }),
            _ => Ok(()),
        }
    }
}

/// The input field carrying the holder a released `pre_call` resumes after.
/// The chain reads it and removes it before calling any target.
const RESUME_AFTER: &str = "resume_after";

/// How the chain calls one observer's target.
#[async_trait]
pub(crate) trait HookInvoker: Send + Sync {
    async fn invoke_value(
        &self,
        binding: &HookBinding,
        point: &str,
        payload: Value,
    ) -> Result<Value, String>;
}

/// The production invoker: a call through the binding's observer, bounded by
/// its `timeout_ms`.
struct ThroughObservers {
    iii: Arc<IIIClient>,
    observers: HashMap<String, Observer>,
}

#[async_trait]
impl HookInvoker for ThroughObservers {
    async fn invoke_value(
        &self,
        binding: &HookBinding,
        point: &str,
        payload: Value,
    ) -> Result<Value, String> {
        let Some(observer) = self.observers.get(&binding.observer_id) else {
            return Err(format!("hook {} has no active observer", binding.target));
        };
        let baggage = [("iii.tag.kind", "harness.hook"), ("iii.hook.point", point)];
        let request = CallRequest::observer(observer, payload)
            .timeout(Duration::from_millis(binding.timeout_ms));
        let fut = iii_helpers::observability::run_with_baggage(&baggage, self.iii.call(request));
        let bounded = tokio::time::timeout(
            Duration::from_millis(binding.timeout_ms.saturating_add(1_000)),
            fut,
        )
        .await;
        match bounded {
            Ok(Ok(value)) => Ok(value),
            Ok(Err(e)) => Err(format!("hook {} failed: {e}", binding.target)),
            Err(_) => Err(format!("hook {} timed out", binding.target)),
        }
    }
}

/// The deliver hook of every hook function: run `point`'s chain over the
/// observers whose filters match the call, and return the merged outcome.
pub(super) async fn deliver(
    iii: Arc<IIIClient>,
    point: HookPoint,
    call: Call,
    observers: Vec<Observer>,
) -> Value {
    let matched: Vec<Observer> = observers
        .into_iter()
        .filter(|obs| obs.matches(&call))
        .collect();
    let bindings = ordered(
        matched
            .iter()
            .filter_map(|obs| match HookBinding::from_observer(point, obs) {
                Ok(binding) => Some(binding),
                Err(error) => {
                    tracing::warn!(observer = %obs.id(), %error, "hook observer skipped");
                    None
                }
            })
            .collect(),
    );
    let invoker = ThroughObservers {
        iii,
        observers: matched
            .into_iter()
            .map(|obs| (obs.id().to_string(), obs))
            .collect(),
    };
    let outcome = run_chain(point, &invoker, bindings, call.input).await;
    serde_json::to_value(outcome).unwrap_or(Value::Null)
}

/// Run `point`'s chain over `bindings` (already in chain order) for `input`.
pub(crate) async fn run_chain(
    point: HookPoint,
    invoker: &dyn HookInvoker,
    bindings: Vec<HookBinding>,
    mut input: Value,
) -> HookChainOutcome {
    let session_id = input
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let bindings: Vec<HookBinding> = bindings
        .into_iter()
        .filter(|binding| sessions_match(binding, &session_id))
        .collect();
    let resume_after = input
        .as_object_mut()
        .and_then(|map| map.remove(RESUME_AFTER))
        .and_then(|holder| holder.as_str().map(str::to_string));
    match point {
        HookPoint::PreTurn => chain_pre_turn(invoker, &bindings, input).await,
        HookPoint::PreGenerate => chain_pre_generate(invoker, &bindings, input).await,
        HookPoint::PostGenerate => {
            for binding in &bindings {
                let _ = invoke(invoker, binding, input.clone()).await;
            }
            HookChainOutcome::default()
        }
        HookPoint::PreCall => {
            chain_pre_call(invoker, bindings, input, resume_after.as_deref()).await
        }
        HookPoint::PostCall => chain_post_call(invoker, &bindings, input).await,
        HookPoint::PostTurn => chain_post_turn(invoker, &bindings, input).await,
    }
}

async fn chain_pre_turn(
    invoker: &dyn HookInvoker,
    bindings: &[HookBinding],
    input: Value,
) -> HookChainOutcome {
    for binding in bindings {
        if let HookOutcome::Deny(reason) = invoke(invoker, binding, input.clone()).await {
            return HookChainOutcome::deny(reason);
        }
    }
    HookChainOutcome::default()
}

async fn chain_pre_generate(
    invoker: &dyn HookInvoker,
    bindings: &[HookBinding],
    mut input: Value,
) -> HookChainOutcome {
    let base_messages: Vec<Value> = input["generate"]["messages"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let mut system_prompt: Option<String> = None;
    let mut appended: Vec<Value> = Vec::new();
    let mut annotations = Map::new();
    for binding in bindings {
        // Already in the assembled prompt (`with_static_prompts`).
        if binding.inject_prompt.is_some() {
            continue;
        }
        if let Some(prompt) = &system_prompt {
            input["generate"]["system_prompt"] = Value::String(prompt.clone());
        }
        let mut messages = base_messages.clone();
        messages.extend(appended.iter().cloned());
        input["generate"]["messages"] = Value::Array(messages);
        match invoke(invoker, binding, input.clone()).await {
            HookOutcome::Continue(m) => {
                if m.system_prompt.is_some() {
                    system_prompt = m.system_prompt;
                }
                appended.extend(m.append_messages);
                merge(&mut annotations, m.annotations);
            }
            HookOutcome::Deny(reason) => return HookChainOutcome::deny(reason),
            HookOutcome::Hold(_) => {}
        }
    }
    HookChainOutcome {
        system_prompt,
        append_messages: appended,
        annotations,
        ..HookChainOutcome::default()
    }
}

async fn chain_pre_call(
    invoker: &dyn HookInvoker,
    bindings: Vec<HookBinding>,
    mut input: Value,
    resume_after: Option<&str>,
) -> HookChainOutcome {
    let function_id = input["call"]["function_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let mut args = input["call"]["arguments"].clone();
    let mut annotations = Map::new();
    for binding in chain_slice(bindings, &function_id, resume_after) {
        input["call"]["arguments"] = args.clone();
        match invoke(invoker, &binding, input.clone()).await {
            HookOutcome::Continue(m) => {
                if let Some(a) = m.arguments {
                    args = a;
                }
                merge(&mut annotations, m.annotations);
            }
            HookOutcome::Deny(reason) => return HookChainOutcome::deny(reason),
            HookOutcome::Hold(m) => {
                // Apply the holding hook's own rewrite before parking, so
                // the release runs with the full chain's effective args.
                if let Some(a) = m.arguments {
                    args = a;
                }
                merge(&mut annotations, m.annotations);
                return HookChainOutcome {
                    decision: ChainDecision::Hold,
                    held_by: Some(binding.target.clone()),
                    arguments: Some(args),
                    annotations,
                    ..HookChainOutcome::default()
                };
            }
        }
    }
    HookChainOutcome {
        arguments: Some(args),
        annotations,
        ..HookChainOutcome::default()
    }
}

async fn chain_post_call(
    invoker: &dyn HookInvoker,
    bindings: &[HookBinding],
    mut input: Value,
) -> HookChainOutcome {
    let function_id = input["call"]["function_id"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let mut annotations = Map::new();
    for binding in bindings {
        if !functions_match(binding, &function_id) {
            continue;
        }
        match invoke(invoker, binding, input.clone()).await {
            HookOutcome::Continue(m) => {
                if let Some(c) = m.content {
                    input["result"]["content"] = json!(c);
                }
                if let Some(d) = m.details {
                    input["result"]["details"] = d;
                }
                if let Some(e) = m.is_error {
                    input["result"]["is_error"] = Value::Bool(e);
                }
                merge(&mut annotations, m.annotations);
            }
            // Deny is not valid at post_call; fail-open like existing
            // post-call error handling.
            HookOutcome::Deny(_) => {}
            HookOutcome::Hold(_) => {
                return HookChainOutcome {
                    decision: ChainDecision::Hold,
                    held_by: Some(binding.target.clone()),
                    annotations,
                    ..HookChainOutcome::default()
                };
            }
        }
    }
    HookChainOutcome {
        result: Some(input["result"].take()),
        annotations,
        ..HookChainOutcome::default()
    }
}

async fn chain_post_turn(
    invoker: &dyn HookInvoker,
    bindings: &[HookBinding],
    input: Value,
) -> HookChainOutcome {
    let result = input["result"].clone();
    for binding in bindings {
        if let Some(template) = &binding.payload {
            // Template mode: no hook input, no mutations — a verdict.
            let payload = crate::functions::observer_deliver::inject_at(
                template.clone(),
                binding.result_into.as_deref().unwrap_or("/value"),
                result.clone(),
            );
            match invoker
                .invoke_value(binding, HookPoint::PostTurn.input_name(), payload)
                .await
            {
                Ok(response) => match validation_verdict(&response) {
                    Some(true) => {}
                    Some(false) => {
                        let reason = format!("result rejected by validator {}", binding.target);
                        return HookChainOutcome {
                            prompt: render_retry_prompt(binding, &reason, Some(&response)),
                            ..HookChainOutcome::deny(reason)
                        };
                    }
                    None => {
                        // Contract confusion, not a real verdict: keep the
                        // generic text — a task-shaped custom prompt must
                        // not mask a broken validator.
                        return HookChainOutcome::deny(format!(
                            "validator {} returned no verdict (expected `valid` or a pipe receipt)",
                            binding.target
                        ));
                    }
                },
                Err(message) => {
                    if let HookOutcome::Deny(reason) = on_error(binding, message) {
                        return HookChainOutcome::deny(reason);
                    }
                }
            }
            continue;
        }
        match invoke(invoker, binding, input.clone()).await {
            HookOutcome::Continue(_) => {}
            HookOutcome::Deny(reason) => {
                return HookChainOutcome {
                    prompt: render_retry_prompt(binding, &reason, None),
                    ..HookChainOutcome::deny(reason)
                }
            }
            HookOutcome::Hold(_) => {} // hold is not a post_turn verb
        }
    }
    HookChainOutcome::default()
}

/// Call one observer's target with the hook input, bounded by `timeout_ms`
/// and resolved by `on_error`. A redelivered step re-runs hooks — they must
/// be idempotent.
async fn invoke(invoker: &dyn HookInvoker, binding: &HookBinding, input: Value) -> HookOutcome {
    let point = input
        .get("point")
        .and_then(Value::as_str)
        .unwrap_or("unknown")
        .to_string();
    match invoker.invoke_value(binding, &point, input).await {
        Ok(value) => parse_output(value),
        Err(message) => on_error(binding, message),
    }
}

fn on_error(binding: &HookBinding, message: String) -> HookOutcome {
    if binding.fail_closed {
        tracing::warn!(target = %binding.target, %message, "hook failed closed (deny)");
        HookOutcome::Deny(message)
    } else {
        tracing::warn!(target = %binding.target, %message, "hook failed open (skip)");
        HookOutcome::Continue(HookMutations::default())
    }
}

fn parse_output(value: Value) -> HookOutcome {
    if value.is_null() {
        return HookOutcome::Continue(HookMutations::default());
    }
    match value.get("decision").and_then(Value::as_str) {
        Some("deny") => HookOutcome::Deny(
            value
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("denied by hook")
                .to_string(),
        ),
        Some("hold") => HookOutcome::Hold(parse_mutations(&value)),
        _ => HookOutcome::Continue(parse_mutations(&value)),
    }
}

/// Parse a hook's `mutations`/`annotations` — shared by the continue and hold
/// branches so a holding hook's rewrites are honored, not dropped.
fn parse_mutations(value: &Value) -> HookMutations {
    let mut muts = HookMutations::default();
    if let Some(m) = value.get("mutations") {
        muts.system_prompt = m
            .get("system_prompt")
            .and_then(Value::as_str)
            .map(str::to_string);
        if let Some(arr) = m.get("append_messages").and_then(Value::as_array) {
            muts.append_messages = arr.clone();
        }
        muts.arguments = m.get("arguments").cloned();
        if let Some(c) = m.get("content") {
            muts.content = serde_json::from_value(c.clone()).ok();
        }
        muts.details = m.get("details").cloned();
        muts.is_error = m.get("is_error").and_then(Value::as_bool);
    }
    if let Some(Value::Object(ann)) = value.get("annotations") {
        muts.annotations = ann.clone();
    }
    muts
}

fn merge(into: &mut Map<String, Value>, from: Map<String, Value>) {
    for (k, v) in from {
        into.insert(k, v);
    }
}

/// The prompt with every static `inject_prompt` observer appended in chain
/// order. The turn loop applies it before `context::assemble`, so the budget,
/// the compaction gate and the request all count that text.
pub fn with_static_prompts(
    bindings: &[HookBinding],
    system_prompt: Option<String>,
) -> Option<String> {
    bindings
        .iter()
        .filter_map(|binding| binding.inject_prompt.as_deref())
        .fold(system_prompt, append_prompt)
}

fn append_prompt(base: Option<String>, injected: &str) -> Option<String> {
    Some(match base {
        Some(base) if !base.is_empty() => format!("{base}\n\n{injected}"),
        _ => injected.to_string(),
    })
}

/// The observers a `pre_call` run consults for this called function: the
/// glob-matched chain, cut down to everything AFTER `resume_after` when
/// resuming a released hold — hooks up to and including the holder already
/// ran and mutated the checkpointed arguments, so re-running them would
/// double-apply mutations (and re-hold). If the holder no longer observes the
/// point, nothing runs: the chain shape changed under the hold and the safe
/// resume point is unknowable.
fn chain_slice(
    bindings: Vec<HookBinding>,
    function_id: &str,
    resume_after: Option<&str>,
) -> Vec<HookBinding> {
    let mut matched: Vec<HookBinding> = bindings
        .into_iter()
        .filter(|b| functions_match(b, function_id))
        .collect();
    let Some(holder) = resume_after else {
        return matched;
    };
    match matched.iter().position(|b| b.target == holder) {
        Some(i) => matched.split_off(i + 1),
        None => Vec::new(),
    }
}

/// Whether a pre/post_call observer's `functions` globs match the called
/// function (no `functions` filter → all calls).
pub(super) fn functions_match(binding: &HookBinding, function_id: &str) -> bool {
    globs_match(binding.functions.as_deref(), function_id)
}

/// Whether an observer's `sessions` globs match the turn's session (no
/// filter → every session).
pub(super) fn sessions_match(binding: &HookBinding, session_id: &str) -> bool {
    globs_match(binding.sessions.as_deref(), session_id)
}

fn globs_match(globs: Option<&[String]>, candidate: &str) -> bool {
    match globs {
        // No filter, and an empty filter, both mean "every session".
        None | Some([]) => true,
        Some(globs) => {
            let mut builder = GlobSetBuilder::new();
            for g in globs {
                if let Ok(glob) = Glob::new(g) {
                    builder.add(glob);
                }
            }
            builder
                .build()
                .map(|set| set.is_match(candidate))
                .unwrap_or(false)
        }
    }
}

/// A `post_turn` denial: `reason` goes into logs and the failure record;
/// `prompt`, when the observer set `retry_prompt`, is the corrective message
/// sent to the model VERBATIM instead of the generic wrapper.
pub struct PostTurnDeny {
    pub reason: String,
    pub prompt: Option<String>,
}

/// Render an observer's `retry_prompt`: `{reason}` = the deny reason,
/// `{value}` = the validator's measured value (fp::pipe receipt
/// `value_preview`; empty when unavailable).
fn render_retry_prompt(
    binding: &HookBinding,
    reason: &str,
    response: Option<&Value>,
) -> Option<String> {
    let template = binding.retry_prompt.as_deref()?;
    let value = response
        .and_then(|r| r.get("value_preview"))
        .and_then(Value::as_str)
        .unwrap_or("");
    Some(
        template
            .replace("{reason}", reason)
            .replace("{value}", value),
    )
}

/// Template-mode verdict: `{ valid: bool }` is authoritative. An fp::pipe
/// receipt (recognised by its `steps` array) is invalid iff a failed
/// fp::when guard short-circuited it — the receipt OMITS `short_circuited`
/// on a full run (`skip_serializing_if`), so absence on a receipt = valid.
fn validation_verdict(response: &Value) -> Option<bool> {
    if let Some(b) = response.get("valid").and_then(Value::as_bool) {
        return Some(b);
    }
    if let Some(sc) = response.get("short_circuited").and_then(Value::as_bool) {
        return Some(!sc);
    }
    if response.get("steps").map(Value::is_array).unwrap_or(false) {
        return Some(true); // pipe receipt, no short-circuit flag = full run
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn null_and_missing_decision_are_continue() {
        assert!(matches!(
            parse_output(Value::Null),
            HookOutcome::Continue(_)
        ));
        assert!(matches!(
            parse_output(json!({ "mutations": { "system_prompt": "x" } })),
            HookOutcome::Continue(_)
        ));
    }

    #[test]
    fn static_prompt_injection_appends_without_a_leading_separator() {
        assert_eq!(
            append_prompt(Some("base".into()), "guidance").as_deref(),
            Some("base\n\nguidance")
        );
        assert_eq!(append_prompt(None, "guidance").as_deref(), Some("guidance"));
    }

    #[test]
    fn deny_and_hold_parse() {
        match parse_output(json!({ "decision": "deny", "reason": "nope" })) {
            HookOutcome::Deny(r) => assert_eq!(r, "nope"),
            _ => panic!("expected deny"),
        }
        match parse_output(json!({ "decision": "hold" })) {
            HookOutcome::Hold(m) => assert!(m.arguments.is_none()),
            _ => panic!("expected hold"),
        }
        // Legacy hooks may still send pending_timeout_ms; it is ignored.
        assert!(matches!(
            parse_output(json!({ "decision": "hold", "pending_timeout_ms": 1000 })),
            HookOutcome::Hold(_)
        ));
    }

    #[test]
    fn hold_keeps_mutations_and_annotations() {
        let out = parse_output(json!({
            "decision": "hold",
            "mutations": { "arguments": { "value": "expected+approved" } },
            "annotations": { "approved_by": "gate" }
        }));
        match out {
            HookOutcome::Hold(m) => {
                assert_eq!(m.arguments, Some(json!({ "value": "expected+approved" })));
                assert_eq!(m.annotations.get("approved_by"), Some(&json!("gate")));
            }
            _ => panic!("expected hold"),
        }
    }

    #[test]
    fn continue_mutations_parse() {
        let out = parse_output(json!({
            "decision": "continue",
            "mutations": {
                "arguments": { "x": 1 },
                "content": [{ "type": "text", "text": "redacted" }],
                "is_error": true
            },
            "annotations": { "approved_by": "u_1" }
        }));
        match out {
            HookOutcome::Continue(m) => {
                assert_eq!(m.arguments, Some(json!({ "x": 1 })));
                assert!(m.content.is_some());
                assert_eq!(m.is_error, Some(true));
                assert_eq!(m.annotations.get("approved_by"), Some(&json!("u_1")));
            }
            _ => panic!("expected continue"),
        }
    }

    fn binding(target: &str, priority: i64) -> HookBinding {
        HookBinding {
            observer_id: format!("obs-{target}"),
            target: target.into(),
            inject_prompt: None,
            functions: Some(vec!["shell::*".into()]),
            sessions: None,
            payload: None,
            result_into: None,
            retry_prompt: None,
            priority,
            timeout_ms: 5000,
            fail_closed: true,
        }
    }

    #[test]
    fn pre_turn_session_scope_uses_the_same_globs_as_post_turn() {
        let mut scoped = binding("snapshot", 0);
        scoped.sessions = Some(vec!["console-123".into(), "job-*".into()]);

        assert!(sessions_match(&scoped, "console-123"));
        assert!(sessions_match(&scoped, "job-9"));
        assert!(!sessions_match(&scoped, "console-456"));
    }

    fn ids(bindings: &[HookBinding]) -> Vec<&str> {
        bindings.iter().map(|b| b.target.as_str()).collect()
    }

    #[test]
    fn chain_slice_without_resume_runs_the_matched_chain() {
        let chain = vec![binding("mutate", 10), binding("gate", 20)];
        assert_eq!(
            ids(&chain_slice(chain, "shell::run", None)),
            vec!["mutate", "gate"]
        );
    }

    #[test]
    fn chain_slice_resumes_after_the_holder() {
        let chain = vec![
            binding("mutate", 10),
            binding("gate", 20),
            binding("audit", 30),
        ];
        // The mutator and the holder already ran — only `audit` remains.
        assert_eq!(
            ids(&chain_slice(chain.clone(), "shell::run", Some("gate"))),
            vec!["audit"]
        );
        // Holder last in the chain → nothing remains.
        assert!(chain_slice(chain, "shell::run", Some("audit")).is_empty());
    }

    #[test]
    fn chain_slice_runs_nothing_when_the_holder_was_unbound() {
        let chain = vec![binding("mutate", 10), binding("audit", 30)];
        assert!(chain_slice(chain, "shell::run", Some("gone")).is_empty());
    }

    #[test]
    fn chain_slice_positions_by_the_glob_matched_chain() {
        // A holder bound to a different target never matches; the released
        // call's matched chain decides the resume point.
        let mut other = binding("gate", 20);
        other.functions = Some(vec!["fs::*".into()]);
        let chain = vec![binding("mutate", 10), other, binding("audit", 30)];
        assert!(chain_slice(chain, "shell::run", Some("gate")).is_empty());
    }

    #[test]
    fn retry_prompt_renders_placeholders() {
        let mut b = binding("check", 0);
        b.retry_prompt = Some("Only {value} rows so far ({reason}). Insert more.".into());
        let receipt = json!({ "steps": [], "short_circuited": true, "value_preview": "4" });
        assert_eq!(
            render_retry_prompt(&b, "rejected", Some(&receipt)).unwrap(),
            "Only 4 rows so far (rejected). Insert more."
        );
        // Envelope mode has no receipt: {value} renders empty.
        assert_eq!(
            render_retry_prompt(&b, "rejected", None).unwrap(),
            "Only  rows so far (rejected). Insert more."
        );
        // No template → no override.
        assert!(render_retry_prompt(&binding("check", 0), "r", None).is_none());
    }

    #[test]
    fn validation_verdict_reads_valid_then_pipe_receipt() {
        // Explicit verdict is authoritative.
        assert_eq!(validation_verdict(&json!({ "valid": true })), Some(true));
        assert_eq!(validation_verdict(&json!({ "valid": false })), Some(false));
        // fp::pipe receipt: a failed fp::when guard short-circuits = invalid.
        assert_eq!(
            validation_verdict(&json!({ "short_circuited": true })),
            Some(false)
        );
        // Full-run receipt OMITS short_circuited (skip_serializing_if) —
        // the steps array alone must read as valid (live bug: a validated
        // turn looped to budget exhaustion on exactly this shape).
        assert_eq!(
            validation_verdict(
                &json!({ "steps": [{ "function": "fp::when" }], "value_preview": "18" })
            ),
            Some(true)
        );
        // `valid` wins over a receipt carrying both.
        assert_eq!(
            validation_verdict(&json!({ "valid": false, "short_circuited": false })),
            Some(false)
        );
        // No verdict at all.
        assert_eq!(validation_verdict(&json!({ "ok": 1 })), None);
        assert_eq!(validation_verdict(&Value::Null), None);
    }

    #[test]
    fn functions_filter_matches_globs() {
        let binding = binding("gate", 0);
        assert!(functions_match(&binding, "shell::run"));
        assert!(!functions_match(&binding, "fs::read"));

        let all = HookBinding {
            functions: None,
            ..binding
        };
        assert!(functions_match(&all, "anything"));
    }

    /// A scripted invoker: each target answers from the map, and every call
    /// is recorded with the input it received.
    struct Scripted {
        answers: HashMap<String, Result<Value, String>>,
        calls: std::sync::Mutex<Vec<(String, Value)>>,
    }

    impl Scripted {
        fn new(answers: &[(&str, Result<Value, String>)]) -> Self {
            Self {
                answers: answers
                    .iter()
                    .map(|(target, answer)| (target.to_string(), answer.clone()))
                    .collect(),
                calls: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, Value)> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl HookInvoker for Scripted {
        async fn invoke_value(
            &self,
            binding: &HookBinding,
            _point: &str,
            payload: Value,
        ) -> Result<Value, String> {
            self.calls
                .lock()
                .unwrap()
                .push((binding.target.clone(), payload));
            self.answers
                .get(&binding.target)
                .cloned()
                .unwrap_or(Ok(Value::Null))
        }
    }

    fn call_input(function_id: &str, arguments: Value) -> Value {
        json!({
            "point": "pre_call",
            "session_id": "s_1",
            "turn_id": "t_1",
            "step": 0,
            "depth": 0,
            "call": { "id": "c_1", "function_id": function_id, "arguments": arguments },
        })
    }

    /// Each hook sees the arguments as the previous one left them, and the
    /// first hold returns the holder with the arguments up to and including
    /// its own rewrite.
    #[tokio::test]
    async fn pre_call_chains_mutations_and_stops_at_the_first_hold() {
        let invoker = Scripted::new(&[
            (
                "mutate",
                Ok(json!({ "mutations": { "arguments": { "x": 2 } } })),
            ),
            (
                "gate",
                Ok(json!({
                    "decision": "hold",
                    "mutations": { "arguments": { "x": 3 } },
                    "annotations": { "held": true }
                })),
            ),
            ("audit", Ok(Value::Null)),
        ]);
        let chain = vec![
            binding("mutate", 10),
            binding("gate", 20),
            binding("audit", 30),
        ];
        let outcome = run_chain(
            HookPoint::PreCall,
            &invoker,
            chain,
            call_input("shell::run", json!({ "x": 1 })),
        )
        .await;
        assert_eq!(outcome.decision, ChainDecision::Hold);
        assert_eq!(outcome.held_by.as_deref(), Some("gate"));
        assert_eq!(outcome.arguments, Some(json!({ "x": 3 })));
        assert_eq!(outcome.annotations.get("held"), Some(&json!(true)));
        let calls = invoker.calls();
        assert_eq!(calls.len(), 2, "audit never runs after the hold");
        assert_eq!(calls[1].1["call"]["arguments"], json!({ "x": 2 }));
    }

    /// A released hold resumes after the holder, and the resume marker never
    /// reaches a target.
    #[tokio::test]
    async fn a_released_pre_call_resumes_after_its_holder() {
        let invoker = Scripted::new(&[]);
        let chain = vec![
            binding("mutate", 10),
            binding("gate", 20),
            binding("audit", 30),
        ];
        let mut input = call_input("shell::run", json!({ "x": 3 }));
        input[RESUME_AFTER] = json!("gate");
        let outcome = run_chain(HookPoint::PreCall, &invoker, chain, input).await;
        assert_eq!(outcome.decision, ChainDecision::Continue);
        let calls = invoker.calls();
        assert_eq!(ids_of(&calls), vec!["audit"]);
        assert!(calls[0].1.get(RESUME_AFTER).is_none());
    }

    /// A failing fail-closed hook denies; a failing fail-open one is skipped.
    #[tokio::test]
    async fn on_error_decides_what_a_failed_hook_means() {
        let invoker = Scripted::new(&[("broken", Err("hook broken failed: OVERFLOW".into()))]);
        let closed = run_chain(
            HookPoint::PreCall,
            &invoker,
            vec![binding("broken", 0)],
            call_input("shell::run", json!({})),
        )
        .await;
        assert_eq!(closed.decision, ChainDecision::Deny);
        assert!(closed.reason.unwrap().contains("OVERFLOW"));

        let mut open = binding("broken", 0);
        open.fail_closed = false;
        let skipped = run_chain(
            HookPoint::PreCall,
            &invoker,
            vec![open],
            call_input("shell::run", json!({})),
        )
        .await;
        assert_eq!(skipped.decision, ChainDecision::Continue);
    }

    /// Observers whose `functions` globs miss the called function, or whose
    /// `sessions` globs miss the session, are not called.
    #[tokio::test]
    async fn the_chain_consults_only_matching_observers() {
        let invoker = Scripted::new(&[]);
        let mut other_session = binding("elsewhere", 0);
        other_session.sessions = Some(vec!["s_2".into()]);
        let mut fs_only = binding("fs", 0);
        fs_only.functions = Some(vec!["fs::*".into()]);
        run_chain(
            HookPoint::PreCall,
            &invoker,
            vec![binding("shell", 0), other_session, fs_only],
            call_input("shell::run", json!({})),
        )
        .await;
        assert_eq!(ids_of(&invoker.calls()), vec!["shell"]);
    }

    /// pre_generate: each hook sees the prompt as the previous left it; a
    /// static `inject_prompt` observer is never called.
    #[tokio::test]
    async fn pre_generate_threads_the_prompt_and_skips_static_injections() {
        let invoker = Scripted::new(&[
            (
                "first",
                Ok(json!({ "mutations": { "system_prompt": "base + first",
                    "append_messages": [{ "role": "user", "content": "m1" }] } })),
            ),
            ("second", Ok(Value::Null)),
        ]);
        let mut static_one = binding("static", 0);
        static_one.inject_prompt = Some("guidance".into());
        let input = json!({
            "point": "pre_generate",
            "session_id": "s_1",
            "generate": { "system_prompt": "base", "messages": [] },
        });
        let outcome = run_chain(
            HookPoint::PreGenerate,
            &invoker,
            vec![static_one, binding("first", 1), binding("second", 2)],
            input,
        )
        .await;
        assert_eq!(outcome.system_prompt.as_deref(), Some("base + first"));
        assert_eq!(outcome.append_messages.len(), 1);
        let calls = invoker.calls();
        assert_eq!(ids_of(&calls), vec!["first", "second"]);
        assert_eq!(
            calls[1].1["generate"]["system_prompt"],
            json!("base + first")
        );
        assert_eq!(
            calls[1].1["generate"]["messages"].as_array().unwrap().len(),
            1
        );
    }

    /// post_turn template mode sends the template with the result injected,
    /// and a denying receipt carries the observer's retry prompt.
    #[tokio::test]
    async fn post_turn_template_validators_answer_with_a_verdict() {
        let invoker = Scripted::new(&[(
            "fp::pipe",
            Ok(json!({ "steps": [], "short_circuited": true, "value_preview": "4" })),
        )]);
        let mut validator = binding("fp::pipe", 0);
        validator.functions = None;
        validator.payload = Some(json!({ "steps": [] }));
        validator.retry_prompt = Some("Only {value} rows.".into());
        let input = json!({ "point": "post_turn", "session_id": "s_1", "result": { "rows": 4 } });
        let outcome = run_chain(HookPoint::PostTurn, &invoker, vec![validator], input).await;
        assert_eq!(outcome.decision, ChainDecision::Deny);
        assert_eq!(outcome.prompt.as_deref(), Some("Only 4 rows."));
        assert_eq!(invoker.calls()[0].1["value"], json!({ "rows": 4 }));
    }

    fn ids_of(calls: &[(String, Value)]) -> Vec<&str> {
        calls.iter().map(|(target, _)| target.as_str()).collect()
    }
}

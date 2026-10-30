//! The six synchronous hook points (harness.md § Hooks), each a private
//! function `harness::hook::on-<point>`. Siblings observe a hook function
//! with [`HookArgs`]; at each point the harness calls the function with the
//! hook input and awaits it. The function's handler returns the hook input,
//! so its `request_format` and `response_format` are the point's input
//! ([`HookPoint::input_format`]). Its deliver hook runs the chain: the
//! observers whose `sessions` and `functions` globs match the input and whose
//! filters match the call, in priority order, each called through its
//! observer and each seeing the input as mutated by the previous. It returns
//! the merged outcome (veto / hold / mutate,
//! [`runner::HookChainOutcome`]) to the harness.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use iii_sdk::errors::Error;
use iii_sdk::{Call, IIIClient, ObserveRejection, Observer, ObserverHandle, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ON_PRE_TURN_FN: &str = "harness::hook::on-pre-turn";
pub const ON_PRE_GENERATE_FN: &str = "harness::hook::on-pre-generate";
pub const ON_POST_GENERATE_FN: &str = "harness::hook::on-post-generate";
pub const ON_PRE_CALL_FN: &str = "harness::hook::on-pre-call";
pub const ON_POST_CALL_FN: &str = "harness::hook::on-post-call";
pub const ON_POST_TURN_FN: &str = "harness::hook::on-post-turn";

/// The six hook points, in spec order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookPoint {
    PreTurn,
    PreGenerate,
    PostGenerate,
    PreCall,
    PostCall,
    PostTurn,
}

impl HookPoint {
    /// The point's hook function.
    pub fn function_id(self) -> &'static str {
        match self {
            HookPoint::PreTurn => ON_PRE_TURN_FN,
            HookPoint::PreGenerate => ON_PRE_GENERATE_FN,
            HookPoint::PostGenerate => ON_POST_GENERATE_FN,
            HookPoint::PreCall => ON_PRE_CALL_FN,
            HookPoint::PostCall => ON_POST_CALL_FN,
            HookPoint::PostTurn => ON_POST_TURN_FN,
        }
    }

    pub fn all() -> [HookPoint; 6] {
        [
            HookPoint::PreTurn,
            HookPoint::PreGenerate,
            HookPoint::PostGenerate,
            HookPoint::PreCall,
            HookPoint::PostCall,
            HookPoint::PostTurn,
        ]
    }

    /// The `HookInput.point` field value (snake_case, per the spec contract).
    pub fn input_name(self) -> &'static str {
        match self {
            HookPoint::PreTurn => "pre_turn",
            HookPoint::PreGenerate => "pre_generate",
            HookPoint::PostGenerate => "post_generate",
            HookPoint::PreCall => "pre_call",
            HookPoint::PostCall => "post_call",
            HookPoint::PostTurn => "post_turn",
        }
    }

    /// Default `on_error` policy: fail-closed for `pre_*`, fail-open for
    /// `post_*` (harness.md § Chain, hold, and failure semantics).
    /// `post_turn` is fail-closed despite the name: it GATES completion the
    /// way `pre_*` hooks gate spend — an erroring validator must nudge the
    /// bounded retry budget, never silently pass a result.
    pub fn default_fail_closed(self) -> bool {
        matches!(
            self,
            HookPoint::PreTurn | HookPoint::PreGenerate | HookPoint::PreCall | HookPoint::PostTurn
        )
    }

    fn description(self) -> &'static str {
        match self {
            HookPoint::PreTurn => "Synchronous hook: first step of a turn, before any model spend. May veto. Observe it with args { sessions?, priority?, timeout_ms?, on_error? }. Each matching observer's target is called with the hook input { point, session_id, turn_id, step, depth, metadata? } and answers { decision, reason?, mutations?, annotations? }; the harness gets back the merged outcome { decision, reason? }.",
            HookPoint::PreGenerate => "Synchronous hook: after context assembly, before generation. May extend the system prompt, append messages, or veto. Each matching observer's target is called with the hook input plus `generate` { system_prompt, messages, model, provider, functions_generation, tools, expose }, as the previous target left it; the harness gets back the merged outcome { decision, reason?, system_prompt?, append_messages?, annotations? }. An observer whose contribution is static passes it as args.inject_prompt; it joins the prompt before assembly of the turns its `sessions` globs and filters select, and its target is not called.",
            HookPoint::PostGenerate => "Synchronous hook: after the final assistant message update. Observe only: each matching observer's target is called with the hook input plus `generated` { message }; the harness gets back { decision: continue }.",
            HookPoint::PreCall => "Synchronous hook: after the allow/deny policy passes, before the called function runs. May deny, hold, or rewrite arguments. Args `functions` globs select the called functions. Each matching observer's target is called with the hook input plus `call` { id, function_id, arguments }, the arguments as the previous target left them; the harness gets back the merged outcome { decision, reason?, held_by?, arguments?, annotations? }.",
            HookPoint::PostCall => "Synchronous hook: after the called function returns, before the result is appended. May rewrite content/details/is_error. Args `functions` globs select the called functions. Each matching observer's target is called with the hook input plus `call` and `result` { function_call_id, function_id, content, is_error, details }; the harness gets back the merged outcome { decision, held_by?, result?, annotations? }.",
            HookPoint::PostTurn => "Synchronous hook: at finalize, after the output contract validated the result, before the turn completes. Deny re-prompts the turn (bounded by max_validation_retries). Args `sessions` globs scope it; args `payload`+`result_into` call a plain composition function (fp::pipe) as the validator. Each matching observer's target is called with the hook input plus the turn's `result` (or the template with the result injected); the harness gets back the merged outcome { decision, reason?, prompt? }.",
        }
    }

    /// The schema of the point's hook input: what the harness calls the hook
    /// function with, what the function returns, and so what observers of
    /// the function observe.
    pub fn input_format(self) -> Value {
        use crate::surface::schema_value;
        match self {
            HookPoint::PreTurn => schema_value::<PreTurnInput>(),
            HookPoint::PreGenerate => schema_value::<PreGenerateInput>(),
            HookPoint::PostGenerate => schema_value::<PostGenerateInput>(),
            HookPoint::PreCall => schema_value::<PreCallInput>(),
            HookPoint::PostCall => schema_value::<PostCallInput>(),
            HookPoint::PostTurn => schema_value::<PostTurnInput>(),
        }
    }
}

/// The fields every hook input carries.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HookTurn {
    /// The hook point: `pre_turn`, `pre_generate`, `post_generate`,
    /// `pre_call`, `post_call` or `post_turn`.
    pub point: String,
    pub session_id: String,
    pub turn_id: String,
    pub step: u64,
    /// 0 for a top-level turn.
    pub depth: u32,
    /// The turn's `options.metadata`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

/// `harness::hook::on-pre-turn` input.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreTurnInput {
    #[serde(flatten)]
    pub turn: HookTurn,
}

/// What generation is about to send.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GenerateInput {
    pub system_prompt: String,
    pub messages: Vec<Value>,
    pub model: String,
    pub provider: String,
    pub functions_generation: u64,
    /// The decision tools the model is offered.
    pub tools: Vec<Value>,
    /// The session's function exposure mode.
    pub expose: Value,
}

/// `harness::hook::on-pre-generate` input.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreGenerateInput {
    #[serde(flatten)]
    pub turn: HookTurn,
    pub generate: GenerateInput,
}

/// The generated assistant message.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GeneratedInput {
    pub message: Value,
}

/// `harness::hook::on-post-generate` input.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PostGenerateInput {
    #[serde(flatten)]
    pub turn: HookTurn,
    pub generated: GeneratedInput,
}

/// The function call a call hook runs around.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CallInput {
    pub id: String,
    pub function_id: String,
    pub arguments: Value,
}

/// `harness::hook::on-pre-call` input.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PreCallInput {
    #[serde(flatten)]
    pub turn: HookTurn,
    pub call: CallInput,
    /// Set when a released hold resumes the chain after this holder's
    /// target; removed before any target is called.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_after: Option<String>,
}

/// What the called function returned.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CallResultInput {
    pub function_call_id: String,
    pub function_id: String,
    pub content: Vec<Value>,
    pub is_error: bool,
    pub details: Value,
}

/// `harness::hook::on-post-call` input.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PostCallInput {
    #[serde(flatten)]
    pub turn: HookTurn,
    pub call: CallInput,
    pub result: CallResultInput,
}

/// `harness::hook::on-post-turn` input.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PostTurnInput {
    #[serde(flatten)]
    pub turn: HookTurn,
    /// The turn's result, validated by its output contract.
    pub result: Value,
}

/// The args of an observer of a `harness::hook::on-<point>` function.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HookArgs {
    /// on-pre-call / on-post-call only: called function id globs to consult on.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions: Option<Vec<String>>,
    /// session_id globs this hook applies to (omit = every session).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sessions: Option<Vec<String>>,
    /// on-post-turn only: template mode — send THIS argument object to the
    /// target instead of the hook input, with the turn's parsed result
    /// injected at `result_into`. Lets a plain composition function
    /// (`fp::pipe`) validate turns without speaking the hook contract; its
    /// receipt is read as the verdict (`valid`, or `short_circuited`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// on-post-turn template mode: JSON pointer where the result lands in
    /// `payload` (default `/value`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_into: Option<String>,
    /// on-post-turn only: custom corrective prompt sent VERBATIM when this
    /// validator denies (replaces the generic "result was not accepted"
    /// wrapper). Placeholders: `{value}` = the validator's measured value
    /// (fp::pipe receipt `value_preview`), `{reason}` = the deny reason.
    /// Validator ERRORS keep the generic text — a task-shaped prompt must
    /// not mask a broken validator.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_prompt: Option<String>,
    /// Chain order: ascending, ties broken by target function id (default 0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<i64>,
    /// Per-call timeout (default 5000ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Failure policy, `fail_open` or `fail_closed` (default fail_closed for
    /// pre-* and post-turn, fail_open for the other post-*).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on_error: Option<String>,
    /// on-pre-generate only: a static system-prompt contribution. It joins
    /// the prompt before assembly of the turns the observer's `sessions`
    /// globs and filters select, and the observer's target is not called.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inject_prompt: Option<String>,
}

/// One observer of a hook point, as the chain runs it.
#[derive(Debug, Clone)]
pub struct HookBinding {
    /// The observer's id: the chain calls its target through it.
    pub observer_id: String,
    /// The observer's target function id: the chain order's tie-break and
    /// the identity a hold names (`held_by`).
    pub target: String,
    pub inject_prompt: Option<String>,
    pub functions: Option<Vec<String>>,
    pub sessions: Option<Vec<String>>,
    pub payload: Option<Value>,
    pub result_into: Option<String>,
    pub retry_prompt: Option<String>,
    pub priority: i64,
    pub timeout_ms: u64,
    pub fail_closed: bool,
}

impl HookBinding {
    /// Parse an observer's args for `point`.
    pub fn parse(
        point: HookPoint,
        observer_id: &str,
        target: &str,
        args: &Value,
    ) -> Result<Self, String> {
        let raw = if args.is_null() {
            Value::Object(Default::default())
        } else {
            args.clone()
        };
        let args: HookArgs =
            serde_json::from_value(raw).map_err(|e| format!("invalid hook args: {e}"))?;
        let fail_closed = match args.on_error.as_deref() {
            Some("fail_closed") => true,
            Some("fail_open") => false,
            Some(other) => return Err(format!("invalid on_error `{other}`")),
            None => point.default_fail_closed(),
        };
        Ok(HookBinding {
            observer_id: observer_id.to_string(),
            target: target.to_string(),
            inject_prompt: args.inject_prompt.filter(|prompt| !prompt.is_empty()),
            functions: args.functions,
            sessions: args.sessions,
            payload: args.payload,
            result_into: args.result_into,
            retry_prompt: args.retry_prompt,
            priority: args.priority.unwrap_or(0),
            timeout_ms: args.timeout_ms.unwrap_or(5_000),
            fail_closed,
        })
    }

    pub fn from_observer(point: HookPoint, obs: &Observer) -> Result<Self, String> {
        Self::parse(point, obs.id(), obs.target(), obs.args_value())
    }
}

/// Bindings sorted into chain order: ascending priority, ties by target.
pub fn ordered(mut bindings: Vec<HookBinding>) -> Vec<HookBinding> {
    bindings.sort_by(|a, b| {
        a.priority
            .cmp(&b.priority)
            .then_with(|| a.target.cmp(&b.target))
    });
    bindings
}

/// Whether any of `bindings` targets `target_function_id` and consults on
/// calls of `function_id`.
pub fn has_function_binding(
    bindings: &[HookBinding],
    target_function_id: &str,
    function_id: &str,
) -> bool {
    bindings.iter().any(|binding| {
        binding.target == target_function_id && runner::functions_match(binding, function_id)
    })
}

/// The six hook functions plus the engine handle the harness calls them
/// with. Cloned into [`crate::deps::Deps`].
#[derive(Clone)]
pub struct HookRegistry {
    pub iii: Arc<IIIClient>,
    /// Agent-created post-turn validators: observer id → (owner session,
    /// handle). The `harness::observe` intercept records here so teardown
    /// stays owner-checked.
    /// ponytail: in-memory only — a harness restart forgets the ownership
    /// (the observer is disposed with the old connection; nothing re-creates
    /// it). Persist to state if that bites.
    owned: Arc<Mutex<HashMap<String, (String, ObserverHandle)>>>,
}

impl HookRegistry {
    /// Set each hook function's hooks, then register it, and return the
    /// registry.
    pub fn register(iii: &Arc<IIIClient>) -> Self {
        for point in HookPoint::all() {
            let id = point.function_id();
            iii.on_observer_added_for(id, move |obs: Observer| async move {
                HookBinding::from_observer(point, &obs)
                    .map(|_| ())
                    .map_err(|message| ObserveRejection::new("CONFIG_ERROR", message))
            });
            iii.on_observer_deliver_for(id, {
                let iii = iii.clone();
                move |call: Call, observers: Vec<Observer>| {
                    let iii = iii.clone();
                    async move { Ok::<Value, Error>(runner::deliver(iii, point, call, observers).await) }
                }
            });
            iii.register_function(
                id,
                RegisterFunction::new(|input: Value| Ok::<Value, Error>(input))
                    .description(point.description())
                    .request_format(point.input_format())
                    .response_format(point.input_format())
                    .args_format::<HookArgs>()
                    .private(),
            );
        }
        tracing::info!("registered the six harness::hook::on-* functions");
        Self::new(iii.clone())
    }

    /// A registry over `iii` that registers nothing.
    pub fn new(iii: Arc<IIIClient>) -> Self {
        HookRegistry {
            iii,
            owned: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// The active observers of `point`, in chain order.
    pub fn bindings(&self, point: HookPoint) -> Vec<HookBinding> {
        ordered(
            self.iii
                .observers(point.function_id())
                .iter()
                .filter_map(|obs| HookBinding::from_observer(point, obs).ok())
                .collect(),
        )
    }

    /// The `on-pre-generate` observers with a static `inject_prompt` that
    /// apply to `input`, in chain order. They are selected as the chain
    /// selects observers: `sessions` globs on `input.session_id`, and the
    /// observer's filters on the call. `input` is the turn's pre_generate
    /// hook input ([`HookRegistry::static_injection_input`]).
    pub fn static_injections(&self, input: &Value) -> Vec<HookBinding> {
        let call = Call {
            function_id: ON_PRE_GENERATE_FN.to_string(),
            input: input.clone(),
            output: Some(input.clone()),
            ..Call::default()
        };
        let session_id = input
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or_default();
        ordered(
            self.iii
                .observers(ON_PRE_GENERATE_FN)
                .iter()
                .filter(|obs| obs.matches(&call))
                .filter_map(|obs| HookBinding::from_observer(HookPoint::PreGenerate, obs).ok())
                .filter(|binding| {
                    binding.inject_prompt.is_some() && runner::sessions_match(binding, session_id)
                })
                .collect(),
        )
    }

    /// Record an agent-created post-turn validator; the observer id is what
    /// the agent gets back.
    pub fn record_owned(&self, session_id: &str, handle: ObserverHandle) -> String {
        let id = handle.id().to_string();
        self.owned
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id.clone(), (session_id.to_string(), handle));
        id
    }

    /// Owner-checked teardown for an agent-created post-turn validator.
    /// `None` = not one of ours (or not this caller's) — fall through to the
    /// binding path.
    pub async fn dispose_owned(&self, id: &str, session_id: &str) -> Option<bool> {
        let handle = {
            let mut owned = self.owned.lock().unwrap_or_else(|p| p.into_inner());
            match owned.get(id) {
                Some((owner, _)) if owner == session_id => {
                    owned.remove(id).map(|(_, handle)| handle)
                }
                _ => None,
            }
        }?;
        if let Err(error) = handle.dispose().await {
            tracing::warn!(observer = %id, %error, "post-turn validator dispose failed");
        }
        Some(true)
    }

    /// Dispose the validators still owned by a deleting session.
    pub async fn dispose_session(&self, session_id: &str) {
        let handles: Vec<ObserverHandle> = {
            let mut owned = self.owned.lock().unwrap_or_else(|p| p.into_inner());
            let ids: Vec<String> = owned
                .iter()
                .filter(|(_, (owner, _))| owner == session_id)
                .map(|(id, _)| id.clone())
                .collect();
            ids.iter()
                .filter_map(|id| owned.remove(id).map(|(_, handle)| handle))
                .collect()
        };
        for handle in handles {
            if let Err(error) = handle.dispose().await {
                tracing::warn!(observer = %handle.id(), %error, "post-turn validator dispose failed");
            }
        }
    }

    pub fn filesystem_boundary(
        &self,
        target_function_id: &str,
    ) -> crate::filesystem_scope::FilesystemBoundary {
        filesystem_boundary(&self.bindings(HookPoint::PostCall), target_function_id)
    }
}

/// The boundary a call of `target_function_id` runs under, given the live
/// post-call observers.
pub fn filesystem_boundary(
    post_call: &[HookBinding],
    target_function_id: &str,
) -> crate::filesystem_scope::FilesystemBoundary {
    // fp::pipe and fp::batch are stamped on behalf of their scoped steps:
    // give them the boundary a direct scoped call would get, or a step
    // would run under a WIDER jail than the same call made directly.
    // shell::exec represents the watch's fixed shell::*/coder::* observer set.
    let target_function_id = if crate::filesystem_scope::carries_scoped_steps(target_function_id) {
        "shell::exec"
    } else {
        target_function_id
    };
    if has_function_binding(
        post_call,
        crate::filesystem_scope::FILESYSTEM_ACCESS_WATCH_ID,
        target_function_id,
    ) {
        crate::filesystem_scope::FilesystemBoundary::Workspace
    } else {
        crate::filesystem_scope::FilesystemBoundary::ConfiguredRoots
    }
}

pub mod runner;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn binding(point: HookPoint, id: &str, target: &str, args: Value) -> HookBinding {
        HookBinding::parse(point, id, target, &args).unwrap()
    }

    #[test]
    fn bindings_order_by_priority_then_target() {
        let order: Vec<String> = ordered(vec![
            binding(HookPoint::PreCall, "a", "z::hook", json!({ "priority": 5 })),
            binding(HookPoint::PreCall, "b", "a::hook", json!({ "priority": 5 })),
            binding(HookPoint::PreCall, "c", "m::hook", json!({ "priority": 1 })),
        ])
        .into_iter()
        .map(|b| b.target)
        .collect();
        assert_eq!(order, vec!["m::hook", "a::hook", "z::hook"]);
    }

    #[test]
    fn static_prompt_injection_comes_from_the_observer_args() {
        let fp = binding(
            HookPoint::PreGenerate,
            "fp",
            "fp::inject-guidance",
            json!({ "inject_prompt": "fp guidance" }),
        );
        assert_eq!(fp.inject_prompt.as_deref(), Some("fp guidance"));

        let empty = binding(
            HookPoint::PreGenerate,
            "empty",
            "empty::hook",
            json!({ "inject_prompt": "" }),
        );
        assert!(empty.inject_prompt.is_none());
    }

    #[test]
    fn static_prompts_join_in_chain_order() {
        let bindings = ordered(vec![
            binding(
                HookPoint::PreGenerate,
                "b",
                "b::hook",
                json!({ "priority": 2, "inject_prompt": "second" }),
            ),
            binding(
                HookPoint::PreGenerate,
                "a",
                "a::hook",
                json!({ "priority": 1, "inject_prompt": "first" }),
            ),
        ]);
        let base = || Some("base".to_string());
        assert_eq!(runner::with_static_prompts(&[], base()), base());
        assert_eq!(
            runner::with_static_prompts(&bindings, base()).as_deref(),
            Some("base\n\nfirst\n\nsecond")
        );
        assert_eq!(
            runner::with_static_prompts(&bindings, None).as_deref(),
            Some("first\n\nsecond")
        );
    }

    #[test]
    fn filesystem_boundary_tracks_the_live_access_watch_observer() {
        use crate::filesystem_scope::{FilesystemBoundary, FILESYSTEM_ACCESS_WATCH_ID};
        assert_eq!(
            filesystem_boundary(&[], "shell::fs::ls"),
            FilesystemBoundary::ConfiguredRoots
        );
        let watch = vec![binding(
            HookPoint::PostCall,
            "access-watch",
            FILESYSTEM_ACCESS_WATCH_ID,
            json!({ "functions": ["shell::*", "coder::*"] }),
        )];
        assert_eq!(
            filesystem_boundary(&watch, "shell::fs::ls"),
            FilesystemBoundary::Workspace
        );
        assert_eq!(
            filesystem_boundary(&watch, "web::fetch"),
            FilesystemBoundary::ConfiguredRoots
        );
        // watch bound to shell::*/coder::* only — the pipe must still pick up
        // Workspace, or its steps would run under a wider jail than direct calls
        assert_eq!(
            filesystem_boundary(&watch, "fp::pipe"),
            FilesystemBoundary::Workspace
        );
        assert_eq!(
            filesystem_boundary(&watch, "fp::batch"),
            FilesystemBoundary::Workspace
        );
    }

    #[test]
    fn on_error_default_depends_on_point() {
        assert!(binding(HookPoint::PreCall, "a", "h", json!({})).fail_closed);
        assert!(!binding(HookPoint::PostCall, "a", "h", json!({})).fail_closed);
        assert!(binding(HookPoint::PostTurn, "a", "h", json!({})).fail_closed);
    }

    #[test]
    fn invalid_args_are_rejected() {
        assert!(HookBinding::parse(
            HookPoint::PreTurn,
            "a",
            "h",
            &json!({ "on_error": "maybe" })
        )
        .is_err());
        assert!(
            HookBinding::parse(HookPoint::PreTurn, "a", "h", &json!({ "prority": 1 })).is_err(),
            "a typo in the args is refused, never silently ignored"
        );
        assert!(HookBinding::parse(HookPoint::PreTurn, "a", "h", &Value::Null).is_ok());
    }

    #[test]
    fn every_hook_function_follows_the_on_naming() {
        for point in HookPoint::all() {
            assert!(
                point.function_id().starts_with("harness::hook::on-"),
                "{}",
                point.function_id()
            );
        }
    }
}

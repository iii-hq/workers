//! `harness::observe` / `harness::unobserve` — the agent's observe surface
//! (harness.md § Subscriptions). An agent observes a function and is woken
//! with what it returns, or has a plain function called with it, instead of
//! polling. The harness intercepts both calls (see [`invoke`]) so the trusted
//! owning session is injected and teardown stays owner-checked — the agent
//! can never supply those. Every observe becomes a durable binding whose
//! delivery either notifies the owner session (a wake) or calls a plain,
//! policy-checked function. A binding never starts an agent: `harness::spawn`
//! is not a target, and spawning is a direct call the owner makes on its own
//! turn.
//!
//! The harness creates the binding's observer itself, over its own
//! connection ([`crate::bindings::observe_binding`]): the observed function
//! with the agent's args and filters, `harness::observer::deliver` as the
//! target, the binding id as the observer id and its `__binding` metadata.
//! The source namespace is resolved here, home-first: the harness's own
//! namespace, then `default`, then Compose's, wherever the function is hosted
//! — an observer of a function in another namespace would stay `pending`
//! forever (MOT: console-04e02cb7 postmortem, `fires: 0` while the watched
//! state key was written). Observers belong to the harness connection, so the
//! durable thing is the binding record: startup re-creates every surviving
//! binding's observer from the store ([`crate::bindings::gc::run`]).

use std::time::Duration;

use iii_sdk::protocol::CallRequest;
use iii_sdk::{ObserverHandle, Source};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::bindings::{
    Binding, BindingTarget, ConditionSpec, Lifecycle, OwnerScope, ReserveOutcome, SourceSpec,
    STATE_ON_CHANGED_FN,
};
use crate::call::{self, ResultData};
use crate::clients::EngineClient;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::policy::CompiledPolicy;
use crate::subscriptions;
use crate::types::content::ContentBlock;
use crate::types::model::AgentFunction;

/// The agent tool that observes a function for the calling session. The
/// harness intercepts it so it can stamp the trusted session and aim the
/// observer at the harness delivery hop.
pub const OBSERVE_ID: &str = "harness::observe";

/// The agent tool that removes an observer the calling session created. The
/// harness intercepts it so it resolves the caller's binding, enforces
/// ownership, and disposes the observer.
pub const UNOBSERVE_ID: &str = "harness::unobserve";

/// Side-effect-free "what would the gate decide" probe. Absent when the
/// deployment runs no approval worker — see [`approval_allows_unattended`].
const APPROVAL_EVALUATE_ID: &str = "approval::evaluate";

/// Compose reports each operation's progress as calls of this function, in
/// the supervisor namespace rather than the project namespace where the
/// harness worker runs.
pub const COMPOSE_PROGRESS_FN: &str = "compose::on-operation-progress";
const COMPOSE_NAMESPACE_ENV: &str = "III_COMPOSE_NAMESPACE";

/// The cron worker's tick: recurring by definition.
const CRON_ON_TICK_FN: &str = "cron::on-tick";

/// The engine builtin that reports where a function is hosted.
const FUNCTIONS_INFO_ID: &str = "engine::functions::info";

/// How long `harness::observe` waits for its observer to become active, so a
/// refusal (`INVALID_ARGS`, `FORBIDDEN`, `NOT_OBSERVABLE`, a rejection by the
/// function's worker) reaches the agent. An observer still `pending` after
/// it is waiting for its function's worker and activates on its own.
const OBSERVE_READY_WAIT: Duration = Duration::from_secs(5);

/// Agent-facing observe contract.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(rename = "ObserveArgs")]
pub struct ObserveRequest {
    /// The function to observe: `state::on-changed`, `cron::on-tick`,
    /// `harness::on-timer`, `database::on-row-changed`, or any other
    /// function. What it returns is the event.
    pub function_id: String,
    /// Where the function is hosted; omitted: this harness's namespace, then
    /// `default`, then Compose's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Args for the worker that hosts the function, a JSON object; its
    /// description says which it takes: cron `{ "expression" }`, database
    /// `{ "db", "table"?, "ops"? }`, `harness::on-timer` `{ "in_ms" }` or
    /// `{ "at" }` (epoch ms). Omit for a function that takes none.
    // Published as an object so call reconciliation parses a stringified one
    // like it does `conditions` and `lifecycle`.
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Map<String, Value>>")]
    pub args: Value,
    /// Which calls reach this observer, a JSON object of explicit operators
    /// on the call's fields: `{ "input.scope": { "$eq": "run" } }`.
    #[serde(default)]
    #[schemars(with = "Option<serde_json::Map<String, Value>>")]
    pub filters: Value,
    /// A short human label echoed back in the notification text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// User-facing text saying what happened, shown with each delivery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<String>,
    /// Remove the observer after its first delivery; defaults to true for a
    /// wake (no `target`) and `harness::on-timer`, false for a call target and
    /// `cron::on-tick`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
    /// The function called with each event: `{ function_id, payload?,
    /// event_into? }` — the fixed payload and the JSON pointer where the
    /// event is injected into it (default `/event`). Omit it to be notified
    /// in this session instead. `harness::spawn` is never a valid target.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<BindingTarget>,
    /// Delivery-time gates, in order; each is an iii function answering
    /// `{ decision: "allow" | "skip", payload?, reason? }`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<ConditionSpec>,
    /// When the observer stops delivering; top-level `once` is the shorthand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<LifecycleRequest>,
}

/// The caller-settable half of a binding's lifecycle. Strict on purpose: an
/// unknown key here (the retired `expires_at` above all) is a dropped
/// deadline, so it must error naming the accepted fields, never pass.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LifecycleRequest {
    /// Remove the observer after its first delivery (same as top-level `once`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
    /// Lifetime delivery budget; the observer is removed on its Nth delivery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fires: Option<u64>,
    /// Relative deadline: the observer stops this many milliseconds after it
    /// was created (the only deadline form).
    // Absolute `expires_at` was retired after repeated stale-epoch guesses
    // (verify-wake-fix-1/-4: models sent months-old round numbers);
    // `deny_unknown_fields` turns it into a field-naming error instead of a
    // silently dropped deadline.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_ms: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ObserveResponse {
    pub observer_id: String,
    /// The effective `once` flag applied (after the per-function default).
    pub once: bool,
    /// Advisory only — the observer was CREATED, but its wiring looks
    /// suspicious (e.g. a filter naming a key someone already wrote). Read it
    /// and fix the wiring if it applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Agent-facing unobserve contract. The id is the `observer_id`
/// `harness::observe` returned.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(rename = "UnobserveArgs")]
pub struct UnobserveRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct UnobserveResponse {
    pub removed: bool,
}

/// The contract agents get for the intercepted observe tools — shown as the
/// native tool AND overlaid onto `engine::functions::info/list` responses, so
/// discovery describes exactly what the intercept accepts.
pub const OBSERVE_TOOL_DESC: &str =
    "Observe a function for the current harness session: each time it runs, omit `target` to be \
     notified in this session with what it returned, or name a non-harness `target.function_id` \
     to call with it. Pass the function's `args` when it takes them and `filters` on its \
     `input.<field>` to choose calls. Returns `{ observer_id, once }`; a `note` means the observer \
     was created but its wiring looks suspicious. Call-target results are discarded and do not \
     return to this session.";
pub const UNOBSERVE_TOOL_DESC: &str =
    "Remove an observer the current harness session created (`id` is the observer_id \
     harness::observe returned).";

/// The intercept's request schema for a control id, for discovery overlays.
pub fn control_contract(function_id: &str) -> Option<(&'static str, Value)> {
    match function_id {
        OBSERVE_ID => Some((
            OBSERVE_TOOL_DESC,
            crate::surface::schema_value::<ObserveRequest>(),
        )),
        UNOBSERVE_ID => Some((
            UNOBSERVE_TOOL_DESC,
            crate::surface::schema_value::<UnobserveRequest>(),
        )),
        _ => None,
    }
}

/// What the intercept answers for a control id — `{ observer_id, once,
/// note? }` / `{ removed }`. It keeps the raw `functions::info` details
/// (console, hooks, contract ledger) honest; the model-visible copy strips
/// response schemas.
pub fn control_response_schema(function_id: &str) -> Option<Value> {
    match function_id {
        OBSERVE_ID => Some(crate::surface::schema_value::<ObserveResponse>()),
        UNOBSERVE_ID => Some(crate::surface::schema_value::<UnobserveResponse>()),
        _ => None,
    }
}

pub fn native_control_tools(policy: &CompiledPolicy) -> Vec<AgentFunction> {
    [
        (
            OBSERVE_ID,
            OBSERVE_TOOL_DESC,
            crate::surface::schema_value::<ObserveRequest>(),
        ),
        (
            UNOBSERVE_ID,
            UNOBSERVE_TOOL_DESC,
            crate::surface::schema_value::<UnobserveRequest>(),
        ),
    ]
    .into_iter()
    .filter(|(function_id, _, _)| policy.allows(function_id))
    .map(|(function_id, description, parameters)| AgentFunction {
        name: function_id.to_string(),
        description: description.to_string(),
        parameters,
        label: None,
        execution_mode: Some("sequential".to_string()),
    })
    .collect()
}

/// The registered `harness::observe` / `harness::unobserve` answer only calls
/// that bypass the turn loop's intercept: there is no calling session to
/// observe for.
pub async fn direct_handle<T>(function_id: &str) -> Result<T, HarnessError> {
    Err(HarnessError::InvalidRequest(format!(
        "{function_id} observes for the calling harness session and runs only from a turn. \
         Outside a turn, observe with iii.observe, and remove a session's observers with \
         harness::observers::dispose."
    )))
}

/// The observing turn's dispatch policy, threaded from the dispatch
/// chokepoints so a call binding can freeze the observer's capability.
#[derive(Clone, Copy)]
pub struct CallerModel<'a> {
    pub model: &'a str,
    pub provider: Option<&'a str>,
    pub functions: Option<&'a crate::types::turn::FunctionPolicy>,
}

impl<'a> CallerModel<'a> {
    pub fn from_options(options: &'a crate::types::turn::TurnOptions) -> Self {
        Self {
            model: &options.model,
            provider: options.provider.as_deref(),
            functions: options.functions.as_ref(),
        }
    }
}

/// The single per-call invocation chokepoint. Observe controls and a locked
/// caller's exact self-targeted `harness::send` are handled inline with
/// trusted caller context; the model cannot forge session-lock ownership.
/// Everything else invokes the target normally. Every call site (the turn loop,
/// `harness::function::call`, and the hook-held release path) routes through
/// here so trusted context cannot be bypassed — and so every result, error
/// results included, leaves through one capture-time size cap (MOT-4498).
#[allow(clippy::too_many_arguments)]
pub async fn invoke(
    deps: &Deps,
    engine: &EngineClient,
    policy: &CompiledPolicy,
    function_id: &str,
    arguments: &Value,
    session_id: &str,
    caller_holds_session_lock: bool,
    caller: Option<CallerModel<'_>>,
) -> ResultData {
    if agent_call_forbidden(function_id) {
        return call::denied_result(function_id);
    }
    if function_id == OBSERVE_ID {
        return intercept_observe(deps, arguments, session_id, caller, policy).await;
    }
    let files_only;
    let mut start_note = None;
    let arguments =
        match scaffold_start_approval(deps, policy, function_id, arguments, session_id).await {
            call::StartGate::Pass => arguments,
            call::StartGate::Deny(denied) => return denied,
            call::StartGate::FilesOnly(args, note) => {
                files_only = args;
                start_note = Some(note);
                &files_only
            }
        };
    let witness =
        match super::delete_session_tree::begin_dispatch(deps, session_id, function_id).await {
            Ok(id) => id,
            Err(error) => {
                return call::invocation_error_result(Some(error.code().into()), error.to_string())
            }
        };
    let send = send_invocation_context(
        function_id,
        arguments,
        session_id,
        caller_holds_session_lock,
    );
    let (mut result, outcome_unknown) = if let Some(request) = send {
        (intercept_send(deps, request, session_id).await, false)
    } else {
        match function_id {
            OBSERVE_ID => (
                intercept_observe(deps, arguments, session_id, caller, policy).await,
                false,
            ),
            UNOBSERVE_ID => (
                intercept_unobserve(deps, arguments, session_id).await,
                false,
            ),
            crate::functions::observers_list::OBSERVERS_LIST_ID
            | crate::functions::observers_list::OBSERVERS_DISPOSE_ID => {
                // In-turn controls always target their caller. External console
                // calls bypass this chokepoint and continue supplying session_id.
                let args = with_caller_session_id(arguments, session_id);
                call::invoke_target_classified(engine, policy, function_id, &args).await
            }
            // Tag the job with its owner so `harness::stop` can reap it
            // (`shell::kill-session-jobs`); the stamp overwrites a forged one.
            "shell::exec_bg" => {
                let args = with_caller_session_id(arguments, session_id);
                call::invoke_target_classified(engine, policy, function_id, &args).await
            }
            internal if agent_call_forbidden(internal) => (call::denied_result(internal), false),
            _ => call::invoke_target_classified(engine, policy, function_id, arguments).await,
        }
    };
    // A timeout or broken transport does not cancel the remote invocation.
    // Keep its witness until an operator can establish completion; deletion
    // fails closed instead of treating the local error result as tool exit.
    // The flag is classified by the harness from the structured SDK error,
    // never from the target-controlled result text.
    if !outcome_unknown {
        if let Err(error) = super::delete_session_tree::end_dispatch(deps, &witness).await {
            return call::invocation_error_result(Some(error.code().into()), error.to_string());
        }
    }
    if let (Some(note), false) = (start_note, result.is_error) {
        result.content.push(ContentBlock::text(note));
    }
    call::cap_result(result, deps.cfg().await.max_result_bytes)
}

/// Whether [`invoke`] would settle this call inside the harness — a
/// control-plane interception with its own multi-step writes (`harness::send`
/// into the running turn, an observe = durable record + observer) — rather
/// than dispatch it to the engine. Interceptions are short and NOT safe to
/// drop midway, so the stop race in the turn loop skips them: only an engine
/// dispatch, the long-running case, is interruptible. Mirrors the match in
/// [`invoke`].
pub fn is_locally_intercepted(
    function_id: &str,
    arguments: &Value,
    session_id: &str,
    caller_holds_session_lock: bool,
) -> bool {
    send_invocation_context(
        function_id,
        arguments,
        session_id,
        caller_holds_session_lock,
    )
    .is_some()
        || matches!(function_id, OBSERVE_ID | UNOBSERVE_ID)
}

/// Agent calls carry the harness worker's authority. Private state accessors
/// do not check their caller, so neither those nor credential-management RPCs
/// may be reached by agents, even with `allow = ["*"]`. Namespace claims are
/// likewise reserved for the worker itself, and so is disposing observers by
/// id: the harness created every binding's observer, so its authority would
/// dispose any session's. Console/operator calls bypass this agent boundary;
/// ordinary provider and public state calls remain valid.
fn agent_call_forbidden(function_id: &str) -> bool {
    function_id.starts_with("harness::delete-session-tree")
        || function_id == crate::state::CLAIM_NAMESPACE_ID
        || function_id == crate::bindings::OBSERVERS_DISPOSE_ID
        // `harness::function::call` takes the session lock a running step
        // already holds: an agent calling it for its own session deadlocks,
        // and recursive pipeline invocation is never an agent need.
        || function_id == crate::functions::FUNCTION_CALL_ID
        || [
            "harness::state::",
            "provider::openai-codex::login::",
            "provider::openai-codex::auth::",
            "provider-openai-codex::state::",
        ]
        .iter()
        .any(|prefix| function_id.starts_with(prefix))
}

fn send_invocation_context(
    function_id: &str,
    arguments: &Value,
    session_id: &str,
    caller_holds_session_lock: bool,
) -> Option<crate::functions::send::SendRequest> {
    if function_id != crate::functions::SEND_ID || !caller_holds_session_lock {
        return None;
    }
    let request: crate::functions::send::SendRequest =
        serde_json::from_value(arguments.clone()).ok()?;
    (request.session_id.as_deref() == Some(session_id)).then_some(request)
}

async fn intercept_send(
    deps: &Deps,
    request: crate::functions::send::SendRequest,
    caller_session_id: &str,
) -> ResultData {
    match crate::functions::send::handle_from_invoke(deps, request, caller_session_id, true).await {
        Ok(response) => match serde_json::to_value(response) {
            Ok(value) => call::normalized_result(value),
            Err(error) => call::invocation_error_result(
                None,
                format!("{}: {error}", crate::functions::SEND_ID),
            ),
        },
        Err(error) => call::invocation_error_result(
            Some(error.code().to_string()),
            format!("{}: {error}", crate::functions::SEND_ID),
        ),
    }
}

/// Stamp `session_id` with the calling session. Non-object args pass through
/// untouched — the target's own deserialization owns that rejection.
fn with_caller_session_id(arguments: &Value, session_id: &str) -> Value {
    let mut args = arguments.clone();
    if let Some(map) = args.as_object_mut() {
        map.insert("session_id".into(), Value::String(session_id.to_string()));
    }
    args
}

/// The `once` default, by SHAPE — the single semantic that killed three of
/// five discovery runs when it was one rule for everything:
///
/// * explicit always wins;
/// * `harness::on-timer` is once by definition (a deadline that repeats is a
///   cron);
/// * `cron::on-tick` is recurring by definition;
/// * a WAKE (no target function) is once: it parks its session, and a park
///   that silently re-arms forever is the worse surprise;
/// * a CALL binding is STANDING: "handle every arrival" is what agents mean
///   when they bind work to an event stream — runs 1 and 5 both created
///   intended-standing handlers, got the old once default, and silently lost
///   every event after the first.
///
/// The standing default is not silent either: `standing_binding_advisory`
/// tells the observer it is standing and to bound or remove it.
fn effective_once(req: &ObserveRequest) -> bool {
    if let Some(once) = req
        .once
        .or_else(|| req.lifecycle.as_ref().and_then(|l| l.once))
    {
        return once;
    }
    match req.function_id.as_str() {
        crate::timer::ON_TIMER_FN => true,
        CRON_ON_TICK_FN => false,
        _ => is_wake_shape(req),
    }
}

/// Whether the observe is a wake — no explicit target function, so each
/// delivery is a notification into the observing session.
fn is_wake_shape(req: &ObserveRequest) -> bool {
    req.target.is_none()
}

/// Validate a `harness::on-timer` observe and resolve `{ in_ms }` to an
/// absolute `{ at }` BEFORE the observer is created — observer args replay
/// unchanged to the timer's added hook on every reconnect, and a relative
/// countdown replayed after a restart would silently restart from zero.
/// Strict here, lenient in the provider: agents get told about a nonsense
/// deadline when they observe; a replayed past-due `at` still fires.
fn resolve_timer_request(req: &mut ObserveRequest) -> Result<(), HarnessError> {
    if req.function_id != crate::timer::ON_TIMER_FN {
        return Ok(());
    }
    if !effective_once(req) {
        return Err(HarnessError::InvalidRequest(
            "a timer fires exactly once — for recurrence observe `cron::on-tick` (with a \
             lifecycle bound)"
                .into(),
        ));
    }
    let args: crate::timer::TimerRequestArgs = serde_json::from_value(req.args.clone())
        .map_err(|e| HarnessError::InvalidRequest(format!("harness::on-timer args: {e}")))?;
    let now = crate::types::message::AgentMessage::now_ms();
    let at = crate::timer::resolve_fire_at(&args, now).map_err(HarnessError::InvalidRequest)?;
    if at <= now {
        return Err(HarnessError::InvalidRequest(format!(
            "timer `at` ({at}) is in the past — pass epoch MILLISECONDS, or use `in_ms` for a \
             relative deadline"
        )));
    }
    if at - now > crate::timer::MAX_TIMER_MS {
        return Err(HarnessError::InvalidRequest(format!(
            "timer deadline is more than 7 days out ({at}) — a deadline that far away is \
             usually an epoch mix-up; use `in_ms` for a relative deadline"
        )));
    }
    req.args = json!({ "at": at });
    Ok(())
}

/// The `$eq` value of a top-level filter on `path` (`input.scope`).
fn filter_eq<'a>(filters: &'a Value, path: &str) -> Option<&'a Value> {
    filters.get(path)?.get("$eq")
}

/// Advisory for a standing call binding (no `once`): agents keep creating
/// one-run kickoffs without `once: true`, leaving observers that re-run their
/// call on every future matching event. Purely informative — deliberate
/// standing watchers ignore it.
///
/// Cron gets its own version: recurring is its point, but a recurring cron
/// with NO lifecycle bound fires forever, and every live run that created
/// one meant it as a deadline or a bounded sweep (rctest7's 69 fires; both
/// discovery runs left one firing past their own teardown claim).
///
/// A wake is one-shot by default, so a standing one was asked for with
/// `once: false`: it has no call to re-run, and the cost is a new paid turn
/// in the observing session per event (MOT-5449).
fn standing_binding_advisory(req: &ObserveRequest, once: bool) -> Option<String> {
    if once {
        return None;
    }
    let bounded = req
        .lifecycle
        .as_ref()
        .is_some_and(|l| l.max_fires.is_some() || l.expires_in_ms.is_some());
    if is_wake_shape(req) && req.function_id != CRON_ON_TICK_FN {
        if bounded {
            return None;
        }
        return Some(
            "note: this WAKE is STANDING (`once: false`) — every future matching event wakes \
             this session with a new paid turn until unobserved. Bound it with lifecycle \
             { max_fires | expires_in_ms } or unobserve it in your teardown so it cannot wake \
             you forever."
                .to_string(),
        );
    }
    if req.function_id == CRON_ON_TICK_FN {
        if bounded {
            return None;
        }
        let expr = req
            .args
            .get("expression")
            .and_then(Value::as_str)
            .unwrap_or("?");
        return Some(format!(
            "warning: this cron is UNBOUNDED — `{expr}` fires at every boundary FOREVER until \
             unobserved, and each tick delivers its own paid reaction (a woken turn or a \
             dispatched call). This observer was still CREATED. A deadline wants \
             `harness::on-timer` with args {{ \"in_ms\": <ms> }} (fires once, exactly on time) — \
             or keep this cron and pass `once: true`; a bounded run sets lifecycle \
             {{ max_fires | expires_in_ms }}; a deliberate forever-cron must be unobserved by \
             your teardown."
        ));
    }
    Some(
        "note: this call binding is STANDING — it re-runs its call on EVERY future matching \
         event until unobserved. That is the default for a call target (per-event work is \
         what a call binding means); pass `once: true` for a one-shot, and give a standing \
         binding a lifecycle bound ({ max_fires | expires_in_ms }) or unobserve it in your \
         teardown so it cannot fire forever."
            .to_string(),
    )
}

/// The `(scope, key)` a keyed `state::on-changed` observer watches through
/// its `$eq` filters — the only shape whose pre-existing value can be checked
/// cheaply when the observer is created.
fn watched_state_key(req: &ObserveRequest) -> Option<(&str, &str)> {
    if req.function_id != STATE_ON_CHANGED_FN {
        return None;
    }
    Some((
        filter_eq(&req.filters, "input.scope").and_then(Value::as_str)?,
        filter_eq(&req.filters, "input.key").and_then(Value::as_str)?,
    ))
}

fn prewritten_key_note(scope: &str, key: &str) -> String {
    format!(
        "warning: state {scope}/{key} ALREADY holds a value — state changes do not replay, so \
         writes from before this observer never reach it, and a condition counting arrivals \
         (state::barrier) starts without them. If earlier writers matter, observe BEFORE \
         starting them, or reconcile the gate against what is already written."
    )
}

/// Advisory for a state observer on a key somebody already wrote. Discovery
/// run 2: the finish gate — a barrier over three suppliers — was armed
/// mid-setup, AFTER the first supplier's done event had happened; it starved
/// at 2/3 forever and only the deadline path saved the run. The stale value
/// is detectable when the observer is created, so say so. Fail-open: a lookup
/// error produces no note.
async fn prewritten_key_advisory(deps: &Deps, req: &ObserveRequest) -> Option<String> {
    let (scope, key) = watched_state_key(req)?;
    let timeout_ms = deps.cfg().await.session_timeout_ms;
    let existing = crate::state::state_get(&deps.iii, scope, key, timeout_ms)
        .await
        .ok()?;
    if existing.is_null() {
        return None;
    }
    Some(prewritten_key_note(scope, key))
}

/// Advisory for a `state::on-changed` observer with no `input.key` filter:
/// it fires for EVERY key written in the scope (or every write anywhere, with
/// no scope either) — order signals, done markers, completion keys, all of
/// it. Created beside a keyed observer it double-fires every event, and every
/// delivery is its own unthrottled paid reaction (rctest-k7m3 postmortem).
/// Purely informative — a deliberate catch-all watcher ignores it.
fn state_catchall_advisory(req: &ObserveRequest, once: bool) -> Option<String> {
    if req.function_id != STATE_ON_CHANGED_FN || filter_eq(&req.filters, "input.key").is_some() {
        return None;
    }
    let scope = filter_eq(&req.filters, "input.scope").and_then(Value::as_str);
    let gated = if req.conditions.is_empty() {
        ""
    } else {
        " that passes its conditions"
    };
    match (scope, once) {
        // The fan-in wave (MOT-5408): a one-shot gated by a barrier that
        // names its arrivals wakes once, on the writes it names, so no stray
        // key wins and nothing fans out.
        (Some(_), true) if names_its_arrivals(req) => None,
        // Any other one-shot is removed on the FIRST write that reaches it,
        // so the risk is the wrong write winning (a count barrier counts any
        // key).
        (Some(scope), true) => Some(format!(
            "note: this one-shot state wake has no `input.key` filter, so the FIRST write of ANY \
             key in scope \"{scope}\"{gated} (a progress or done marker included) wakes it and \
             removes it. If only specific writes should wake it, add an `input.key` filter, or \
             list their keys in a state::barrier `expect` array."
        )),
        // No scope: every state write anywhere (the harness's own records
        // included) reaches it, conditions or not.
        (None, true) => Some(format!(
            "warning: this one-shot state wake filters on neither `input.scope` nor `input.key`, \
             so the first state write in ANY scope{gated}, other runs' traffic included, wakes \
             it. Filter it to this run's scope at minimum."
        )),
        // Scope-only is CORRECT for the per-event-unique-key shape (producers
        // writing `item-1`, `item-2`, … so nothing overwrites) — discovery
        // run 7 armed exactly that, read the old "add a `key`" push as an
        // instruction, pinned `key: "shipment"`, and its producers' unique
        // keys then matched nothing at all. State the trade-off; do not pick
        // a side.
        (Some(scope), false) => Some(format!(
            "note: this state observer has no `input.key` filter, so it fires for EVERY key \
             written in scope \"{scope}\" — done markers and progress keys included — and every \
             matching write{gated} dispatches its own unthrottled paid reaction. That is the \
             RIGHT shape when producers write one unique key per event (item-1, item-2, …); keep \
             this scope private to those events. Add an `input.key` filter ONLY if every producer \
             writes that exact key."
        )),
        (None, false) => Some(format!(
            "warning: this state observer filters on neither `input.scope` nor `input.key` — it \
             fires for EVERY state write in EVERY scope, including other runs' traffic, and each \
             one{gated} dispatches its own unthrottled paid reaction. Filter it to this run's \
             scope at minimum."
        )),
    }
}

/// Whether a condition picks the writes that reach the binding: a
/// `state::barrier` whose `expect` is a list accepts only those arrivals,
/// while a count accepts any key.
fn names_its_arrivals(req: &ObserveRequest) -> bool {
    req.conditions.iter().any(|condition| {
        condition.function_id == "state::barrier"
            && condition
                .config
                .as_ref()
                .and_then(|config| config.get("expect"))
                .is_some_and(Value::is_array)
    })
}

async fn intercept_observe(
    deps: &Deps,
    args: &Value,
    session_id: &str,
    caller: Option<CallerModel<'_>>,
    policy: &CompiledPolicy,
) -> ResultData {
    let req: ObserveRequest = match serde_json::from_value(args.clone()) {
        Ok(r) => r,
        Err(e) => return error_result(format!("invalid {OBSERVE_ID} arguments: {e}")),
    };

    match handle(deps, req, session_id, caller, policy).await {
        Ok(resp) => ok_result(&resp),
        Err(e) => error_result(e.to_string()),
    }
}

async fn intercept_unobserve(deps: &Deps, args: &Value, session_id: &str) -> ResultData {
    let req: UnobserveRequest = match serde_json::from_value(args.clone()) {
        Ok(req) => req,
        Err(error) => {
            return error_result(format!("{UNOBSERVE_ID} requires a string `id`: {error}"))
        }
    };
    let id = req.id.as_str();

    // An agent-created post-turn validator? Owner-checked in the registry.
    if let Some(removed) = deps.hooks.dispose_owned(id, session_id).await {
        return ok_result(&UnobserveResponse { removed });
    }

    let store = deps.bindings().await;
    let binding = match store.get(id).await {
        Ok(b) => b,
        Err(e) => return error_result(format!("binding lookup failed: {e}")),
    };

    if let Some(binding) = binding {
        // Owner-only: there is no reaction lineage — a session removes its
        // own observers, the console uses `harness::observers::dispose`, and
        // nothing else may.
        if binding.owner.session_id != session_id {
            return error_result("observer belongs to a different session".to_string());
        }
        if binding.observer_id.is_some() {
            crate::bindings::dispose_observer(deps, &binding.id).await;
        }
        if let Err(e) = store.delete(id).await {
            return error_result(format!("binding delete failed: {e}"));
        }
        return ok_result(&UnobserveResponse { removed: true });
    }

    // No record: already removed (a `once` that fired, or someone else's
    // teardown). `removed: false` is the honest answer, not an error.
    ok_result(&UnobserveResponse { removed: false })
}

/// Build the durable binding an observe describes, create ONE observer aimed
/// at the delivery hop, and hand the id back. Both shapes — wake, mechanical
/// call — produce the same record; only the target differs.
async fn handle(
    deps: &Deps,
    mut req: ObserveRequest,
    session_id: &str,
    caller: Option<CallerModel<'_>>,
    policy: &CompiledPolicy,
) -> Result<ObserveResponse, HarnessError> {
    validate_object("args", &req.args)?;
    validate_object("filters", &req.filters)?;
    // The one harness hook point an agent MAY observe: a post-turn validator
    // for ITS OWN session. The session scope is force-stamped, so an agent
    // can gate its own completions but never anyone else's.
    if req.function_id == crate::hooks::ON_POST_TURN_FN {
        return observe_post_turn(deps, req, session_id, policy).await;
    }
    reject_forbidden_source(&req.function_id)?;

    validate_lifecycle(req.lifecycle.as_ref())?;
    validate_event_action(req.action.as_deref())?;
    let once = effective_once(&req);
    // Idempotency is based on the caller's RELATIVE request (timer `in_ms`,
    // lifecycle `expires_in_ms`). Resolving first would mint a different
    // absolute deadline on every retry and defeat the dedup.
    let dedup = registration_dedup_key(&req, once);
    resolve_timer_request(&mut req)?;
    let now_ms = crate::types::message::AgentMessage::now_ms();
    let expires_at = resolved_expires_at(req.lifecycle.as_ref(), now_ms);

    reject_settled_compose_operation(deps, &req).await?;
    let cfg = deps.cfg().await;
    // Leafness is lineage, not a policy shape: a root under a narrow
    // allow-list cannot spawn either, and must keep both binding shapes.
    let has_parent = crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms)
        .await?
        .is_some_and(|record| {
            record.parent.is_some() || record.display_parent_session_id.is_some()
        });
    reject_leaf_control_plane(
        policy,
        has_parent,
        req.target.as_ref().map(|t| t.function_id.as_str()),
    )?;
    let expires_at = leaf_wake_deadline(
        policy,
        has_parent,
        expires_at,
        now_ms,
        cfg.default_pending_timeout_ms,
    );
    let target = resolve_target(deps, &req, session_id, policy).await?;
    authorize_conditions(deps, &req.conditions, session_id, policy).await?;
    let host = resolve_source_namespace(deps, &req).await;

    let topology = deps.topology.lock().await;
    super::delete_session_tree::ensure_live(deps, session_id).await?;
    // Idempotency: an identical re-observe (a model retry, a re-run prompt in
    // the same session) returns the standing binding instead of wiring a twin
    // that double-delivers forever.
    let store = deps.bindings().await;
    if let Some(existing) = store.find_duplicate(session_id, &dedup).await? {
        return Ok(ObserveResponse {
            observer_id: existing,
            once,
            note: None,
        });
    }
    let mut binding = Binding {
        id: format!("sub_{}", uuid::Uuid::new_v4().simple()),
        observer_id: None,
        source: Some(SourceSpec {
            function_id: req.function_id.clone(),
            namespace: host.namespace.clone(),
            args: (!req.args.is_null()).then(|| req.args.clone()),
            filters: (!req.filters.is_null()).then(|| req.filters.clone()),
        }),
        owner: OwnerScope {
            session_id: session_id.to_string(),
            root_session_id: None,
        },
        target,
        conditions: req.conditions.clone(),
        lifecycle: Lifecycle {
            once,
            max_fires: req.lifecycle.as_ref().and_then(|l| l.max_fires),
            expires_at,
        },
        // The observer's policy, frozen: a delivered call is checked against
        // what the session could call WHEN IT OBSERVED, never against a
        // policy that widened afterwards.
        capability: caller.and_then(|c| c.functions.cloned()),
        dedup_key: Some(dedup),
        fires: 0,
        created_at: crate::types::message::AgentMessage::now_ms(),
    };
    // Durable BEFORE the observer exists: a delivery that arrives before the
    // attach below must still resolve its record. The reservation and
    // per-owner capacity check are one CAS-backed operation; a deadline that
    // lapsed since resolution surfaces as `ReserveOutcome::Exhausted`.
    require_reserved(store.reserve(&binding).await?)?;
    // The deadline is a timer from the moment the record is durable; every
    // later delete of the record (rollback below included) cancels it.
    crate::bindings::expiry::arm(deps, &binding);

    let Some(handle) = crate::bindings::observe_binding(deps, &binding) else {
        let _ = store.delete(&binding.id).await;
        return Err(HarnessError::Internal(format!(
            "binding {} has no source to observe",
            binding.id
        )));
    };
    match store.attach_observer_id(&binding, &binding.id).await {
        Ok(crate::bindings::AttachOutcome::Attached(current)) => {
            binding = *current;
        }
        Ok(crate::bindings::AttachOutcome::Gone) => {
            // A fast one-shot was delivered and removed before the attach
            // landed. Its observer still needs explicit disposal.
            crate::bindings::dispose_observer(deps, &binding.id).await;
        }
        Err(error) => {
            // The caller must never see a failed observe while its observer
            // stays live.
            crate::bindings::dispose_observer(deps, &binding.id).await;
            let _ = store.delete(&binding.id).await;
            return Err(error);
        }
    }

    // A preflight snapshot cannot close the Compose operation's asynchronous
    // activation window. Recover a missed terminal event now; the harness's
    // own terminal Compose watch and every engine worker change retry it from
    // the record.
    crate::bindings::compose::schedule(deps, &binding);
    drop(topology);

    let pending = match readiness(&handle).await {
        Readiness::Active => false,
        Readiness::Pending => true,
        Readiness::Failed(reason) => {
            // Refused by the engine or by the function's worker: nothing can
            // ever deliver, so the observe fails and leaves nothing behind.
            crate::bindings::dispose_observer(deps, &binding.id).await;
            let _ = store.delete(&binding.id).await;
            return Err(HarnessError::InvalidRequest(format!(
                "observing `{}` failed: {reason}",
                req.function_id
            )));
        }
    };

    let notes: Vec<String> = [
        pending
            .then(|| host.parked_note(&req.function_id))
            .flatten(),
        prewritten_key_advisory(deps, &req).await,
        standing_binding_advisory(&req, once),
        state_catchall_advisory(&req, once),
        armed_wake_advisory(&req, once, expires_at.is_some()),
    ]
    .into_iter()
    .flatten()
    .collect();

    Ok(ObserveResponse {
        observer_id: binding.id,
        once,
        note: compose_note(notes),
    })
}

/// How an observer came out of the bounded wait.
enum Readiness {
    Active,
    /// Still waiting for its function's worker (or its target).
    Pending,
    /// Ended in `error` or `disposed`: the reason's code and message.
    Failed(String),
}

async fn readiness(handle: &ObserverHandle) -> Readiness {
    match tokio::time::timeout(OBSERVE_READY_WAIT, handle.ready()).await {
        Ok(Ok(())) => Readiness::Active,
        Ok(Err(error)) => Readiness::Failed(
            handle
                .reason()
                .map(|reason| format!("{}: {}", reason.code, reason.message))
                .unwrap_or_else(|| error.to_string()),
        ),
        Err(_) => Readiness::Pending,
    }
}

/// A non-object `args` or `filters` (above all a stringified
/// `"{\"scope\":…}"` that reconciliation was off for) is refused by the
/// engine only after this call answered: the observer would never deliver.
fn validate_object(field: &str, value: &Value) -> Result<(), HarnessError> {
    if value.is_object() || value.is_null() {
        return Ok(());
    }
    Err(HarnessError::InvalidRequest(format!(
        "`{field}` must be a JSON object such as {{\"input.scope\": {{\"$eq\": \"run\"}}}}, not a \
         JSON-encoded string or any other type"
    )))
}

fn validate_lifecycle(lifecycle: Option<&LifecycleRequest>) -> Result<(), HarnessError> {
    let Some(lifecycle) = lifecycle else {
        return Ok(());
    };
    if lifecycle.max_fires == Some(0) {
        return Err(HarnessError::InvalidRequest(
            "`lifecycle.max_fires` must be at least 1".into(),
        ));
    }
    if let Some(in_ms) = lifecycle.expires_in_ms.filter(|&ms| ms <= 0) {
        return Err(HarnessError::InvalidRequest(format!(
            "`lifecycle.expires_in_ms` must be positive, got {in_ms}"
        )));
    }
    Ok(())
}

fn require_reserved(outcome: ReserveOutcome) -> Result<(), HarnessError> {
    match outcome {
        ReserveOutcome::Reserved => Ok(()),
        ReserveOutcome::Capacity => Err(HarnessError::InvalidRequest(format!(
            "observer cap reached ({} active for this session); unobserve first",
            crate::bindings::MAX_BINDINGS_PER_SESSION
        ))),
        ReserveOutcome::Exhausted => Err(HarnessError::InvalidRequest(
            "observer lifecycle expired before it could be reserved".into(),
        )),
    }
}

/// Join the advisories under one header that says the outcome FIRST.
/// Discovery run 3: a model read a warning as a redo order, removed the
/// binding, re-rolled the same request twice, then tore down its whole watch
/// set — the notes never said the observer had been created, so starting
/// over looked mandatory.
fn compose_note(notes: Vec<String>) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    Some(format!(
        "observer CREATED; everything below is advisory, not an error. Apply a named fix to THIS \
         observer (or unobserve it) rather than starting over. {}",
        notes.join(" ")
    ))
}

/// Turn events (and every other harness function but the timer) are not
/// agent-observable in ANY shape: child outcomes reach a parent through the
/// medium the children write, never through an observer on their turns. Runs
/// before target resolution, so wake, call, and would-be spawn shapes all
/// hit it.
fn reject_forbidden_source(function_id: &str) -> Result<(), HarnessError> {
    if subscriptions::is_forbidden_source(function_id) {
        return Err(HarnessError::InvalidRequest(format!(
            "cannot observe harness function `{function_id}`: turn events are not \
             agent-observable. Watch what the work WRITES instead — observe the state keys or \
             database rows the tasks update (omit `target` for a wake). `{}` is the one harness \
             function an agent observes.",
            crate::timer::ON_TIMER_FN
        )));
    }
    Ok(())
}

/// The operation a Compose progress observer filters on, `input.operation_id`.
fn compose_operation_id(req: &ObserveRequest) -> Option<&str> {
    if req.function_id != COMPOSE_PROGRESS_FN {
        return None;
    }
    filter_eq(&req.filters, "input.operation_id").and_then(Value::as_str)
}

/// Refuse a wake when the operation is already settled at preflight: its result
/// is available now, so direct the caller to read it instead of parking.
/// Completion after this read is recovered from Compose's persisted terminal
/// event after the observe and by the binding sweep. An unknown status or an
/// unavailable diagnostic still permits the observe.
async fn reject_settled_compose_operation(
    deps: &Deps,
    req: &ObserveRequest,
) -> Result<(), HarnessError> {
    let Some(operation_id) = compose_operation_id(req) else {
        return Ok(());
    };
    let Ok(snapshot) = deps
        .engine()
        .await
        .dispatch(
            "compose::operation",
            json!({ "operation_id": operation_id }),
        )
        .await
    else {
        return Ok(());
    };
    let status = snapshot
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if !is_settled_operation(status) {
        return Ok(());
    }
    // Ids are daemon-global, so a settled op may be another session's that
    // reused the same fixed name (MOT-5334): say both readings.
    Err(HarnessError::InvalidRequest(format!(
        "operation `{operation_id}` already reached `{status}`. If you started it, read the \
         result with `compose::operation` now instead of arming a wake, and observe it BEFORE \
         starting the operation next time, in the same message. If you have NOT started it yet, \
         the id belongs to an earlier operation: pick a fresh id that starts with your session \
         id and observe again."
    )))
}

/// A compose operation reaches one of these exactly once, and never leaves it —
/// so a wake armed afterwards has nothing left to wait for. Anything else
/// (`running`, or a status this harness does not know) is still in flight and
/// observes normally: an unknown status must not silently swallow a wake.
fn is_settled_operation(status: &str) -> bool {
    matches!(status, "succeeded" | "failed" | "cancelled")
}

/// A leaf may arm its own wake, but may not install a mechanical call it cannot
/// remove. The deadline half lives in [`leaf_wake_deadline`]: a leaf's wake is
/// stamped rather than refused, because a leaf with no way to wait is the
/// problem this set out to fix.
fn reject_leaf_control_plane(
    policy: &CompiledPolicy,
    has_parent: bool,
    explicit_target: Option<&str>,
) -> Result<(), HarnessError> {
    if !crate::policy::is_leaf(policy, has_parent) {
        return Ok(());
    }
    if let Some(target) = explicit_target {
        return Err(HarnessError::InvalidRequest(format!(
            "a sub-agent cannot bind a mechanical call to `{target}`: that installs durable \
             machinery it does not own and cannot remove. Omit `target` to observe as a wake for \
             your own session instead — the event comes back to you as a message and you make the \
             call yourself."
        )));
    }
    Ok(())
}

/// Give a leaf's wake a deadline when it asked for none.
///
/// `once: true` is NOT one. It caps how many times a binding may fire, not how
/// long it waits, so a `once` wake that never fires is exactly as eternal as an
/// open-ended one — and a leaf cannot remove it. A child that armed two `once`
/// Compose wakes and ended its turn hung forever, and because an incomplete
/// child keeps `tree_complete` false it hung the whole run with it: 15 idle
/// minutes and a RESOURCE LIMIT on a run whose work was done (MOT-4766).
///
/// A root keeps whatever it asked for — it can unobserve, and whoever drives
/// it has its own timeout. The leaf's cap is the harness's existing patience
/// for pending work, so its wake can never outlive that.
fn leaf_wake_deadline(
    policy: &CompiledPolicy,
    has_parent: bool,
    requested: Option<i64>,
    now_ms: i64,
    pending_timeout_ms: u64,
) -> Option<i64> {
    if !crate::policy::is_leaf(policy, has_parent) {
        return requested;
    }
    let cap = now_ms.saturating_add(pending_timeout_ms as i64);
    Some(requested.map_or(cap, |asked| asked.min(cap)))
}

/// Resolve and VALIDATE the target an observe asks for. Two shapes (omitted
/// `target` = wake, anything else = mechanical call); past this point they
/// are one record. A binding never starts an agent — `harness::spawn` falls
/// into the call arm and is refused there with the direct-spawn migration in
/// the error.
async fn resolve_target(
    deps: &Deps,
    req: &ObserveRequest,
    session_id: &str,
    policy: &CompiledPolicy,
) -> Result<BindingTarget, HarnessError> {
    match req.target.as_ref() {
        // A wake: each event is injected into the owner session as a message.
        None => {
            let mut target = BindingTarget::new(crate::functions::SEND_ID);
            target.payload = Some(json!({
                "session_id": session_id,
                "label": req.label,
            }));
            Ok(target)
        }
        // A mechanical call. Gated twice: the observer's own policy, and the
        // deployment's approval gate — a delivered call runs outside any turn
        // and can never raise a prompt.
        Some(requested) => {
            validate_call_target(&requested.function_id, policy)
                .map_err(HarnessError::InvalidRequest)?;
            validate_event_into(requested.event_into.as_deref())?;
            let target = requested.clone();
            let arguments = target.payload.clone().unwrap_or_else(|| json!({}));
            approval_allows_unattended(deps, &target.function_id, session_id, &arguments)
                .await
                .map_err(HarnessError::InvalidRequest)?;
            // A delivered scaffold can start (its payload is projected only
            // then), running a compose::add no approval hook sees.
            if target.function_id == crate::clients::engine::SCAFFOLD_WORKER {
                approval_allows_unattended(deps, COMPOSE_ADD_ID, session_id, &json!({}))
                    .await
                    .map_err(HarnessError::InvalidRequest)?;
            }
            Ok(target)
        }
    }
}

/// Conditions execute with worker authority too, so they cross the same
/// observe boundary as the delivery target.
async fn authorize_conditions(
    deps: &Deps,
    conditions: &[ConditionSpec],
    session_id: &str,
    policy: &CompiledPolicy,
) -> Result<(), HarnessError> {
    for condition in conditions {
        validate_call_target(&condition.function_id, policy)
            .map_err(HarnessError::InvalidRequest)?;
        let arguments = json!({
            "event": null,
            "condition_config": condition.config.clone().unwrap_or(Value::Null),
            "binding": { "id": null, "fires": 0 },
            "context": { "owner_session_id": session_id },
        });
        approval_allows_unattended(deps, &condition.function_id, session_id, &arguments)
            .await
            .map_err(HarnessError::InvalidRequest)?;
    }
    Ok(())
}

/// Validate the one host-interpreted presentation field. The action is plain
/// display text; it never participates in routing or authorization.
fn validate_event_action(action: Option<&str>) -> Result<(), HarnessError> {
    match action {
        Some(action) if action.trim().is_empty() => Err(HarnessError::InvalidRequest(
            "`action` must be a non-empty string describing the event".into(),
        )),
        _ => Ok(()),
    }
}

fn validate_event_into(pointer: Option<&str>) -> Result<(), HarnessError> {
    if pointer.is_some_and(|p| !p.is_empty() && !p.starts_with('/')) {
        return Err(HarnessError::InvalidRequest(format!(
            "`event_into` must be a JSON pointer starting with `/`, got `{}`",
            pointer.unwrap_or_default()
        )));
    }
    Ok(())
}

/// Advisory for a one-shot state-key WAKE: the observing session parks until
/// someone writes that exact scope/key, and the harness cannot verify any
/// task is wired to do so. rctest postmortem: an orchestrator armed
/// `report_ready` while its finalizer's task never mentioned the key — every
/// row landed correctly and the orchestrator still slept forever, leaving the
/// run's teardown permanently pending. Purely informative.
fn armed_wake_advisory(req: &ObserveRequest, once: bool, has_deadline: bool) -> Option<String> {
    // A deadline (the caller's `expires_in_ms`, or a leaf's capped wake)
    // turns a write that never comes into an expiry notice: the park-forever
    // risk this note exists for is already handled.
    if !once || has_deadline || req.function_id != STATE_ON_CHANGED_FN {
        return None;
    }
    let scope = filter_eq(&req.filters, "input.scope")
        .and_then(Value::as_str)
        .unwrap_or("*");
    let key = filter_eq(&req.filters, "input.key")
        .and_then(Value::as_str)
        .unwrap_or("*");
    // With a condition (a `state::barrier`), only a write that passes it
    // wakes the session.
    let gated = if req.conditions.is_empty() {
        ""
    } else {
        " that passes its conditions"
    };
    // The two observed park-forever shapes, named in order: the medium
    // mismatch (a finisher wrote a DATABASE table named like the state scope
    // — same word, disjoint worlds, the wake never fired), and the missing
    // deadline (no lifecycle, so nothing could ever convert the silence into
    // a notice).
    Some(format!(
        "note: one-shot WAKE — this session stays parked until a state write on \
         {scope}/{key}{gated} wakes it, and nothing wakes it automatically. Only a state \
         write (state::set, state::update, a swapping state::compare-and-set or state::delete) on that \
         exact scope/key wakes it — a database table or row with the same name does NOT. \
         Double-check that a task you spawned (worker/finalizer) EXPLICITLY sets that exact scope/key when its condition is met, or this session \
         sleeps forever and its cleanup never runs. Set lifecycle \
         {{ expires_in_ms: <relative ms> }} to be woken with an expiry notice instead if the \
         write never comes."
    ))
}

/// Resolve the RELATIVE lifecycle deadline (`expires_in_ms`) into the
/// absolute instant the binding stores. Computed AFTER the idempotency key is
/// built — like the timer's `in_ms`, resolving first would mint a different
/// absolute deadline on every model retry. `validate_lifecycle` already
/// rejected non-positive values, so the result is a future instant by
/// construction; a deadline that lapses between here and the reservation is
/// the store's `ReserveOutcome::Exhausted`.
fn resolved_expires_at(lifecycle: Option<&LifecycleRequest>, now_ms: i64) -> Option<i64> {
    lifecycle
        .and_then(|l| l.expires_in_ms)
        .map(|in_ms| now_ms.saturating_add(in_ms))
}

/// The canonicalized observe request used for same-session idempotency.
/// Built from the agent's RAW arguments (before the owner stamp) with `once`
/// normalized to its effective value; `serde_json::Value` equality is
/// key-order-insensitive, so semantically identical requests always match.
fn registration_dedup_key(req: &ObserveRequest, once: bool) -> Value {
    json!({
        "function_id": req.function_id,
        "namespace": req.namespace,
        "args": req.args,
        "filters": req.filters,
        "label": req.label,
        "action": req.action,
        "lifecycle": {
            "once": once,
            "max_fires": req.lifecycle.as_ref().and_then(|l| l.max_fires),
            "expires_in_ms": req.lifecycle.as_ref().and_then(|l| l.expires_in_ms),
        },
        "target": req.target,
        "conditions": req.conditions,
    })
}

/// Observe `harness::hook::on-post-turn` with a validator on behalf of the
/// calling agent. The validator target rides the caller's authority (same
/// rule as call bindings: only functions you could call yourself), and the
/// args' `sessions` scope is FORCE-STAMPED to the calling session —
/// self-gating only. The harness hosts the hook function; the handle is kept
/// for owner-checked removal.
async fn observe_post_turn(
    deps: &Deps,
    req: ObserveRequest,
    session_id: &str,
    policy: &CompiledPolicy,
) -> Result<ObserveResponse, HarnessError> {
    let Some(validator) = req.target.as_ref().map(|t| t.function_id.clone()) else {
        return Err(HarnessError::InvalidRequest(format!(
            "{} needs `target.function_id` — the validator function (e.g. fp::pipe); a wake \
             shape is meaningless for a hook",
            crate::hooks::ON_POST_TURN_FN
        )));
    };
    validate_call_target(&validator, policy).map_err(HarnessError::InvalidRequest)?;
    let raw = if req.args.is_null() {
        json!({})
    } else {
        req.args.clone()
    };
    let mut args: crate::hooks::HookArgs = serde_json::from_value(raw)
        .map_err(|e| HarnessError::InvalidRequest(format!("invalid post-turn hook args: {e}")))?;
    args.sessions = Some(
        validate_post_turn_scope(session_id, args.sessions.take())
            .map_err(HarnessError::InvalidRequest)?,
    );
    let args = serde_json::to_value(&args)
        .map_err(|e| HarnessError::InvalidRequest(format!("hook args serialize: {e}")))?;
    let topology = deps.topology.lock().await;
    super::delete_session_tree::ensure_live(deps, session_id).await?;
    let handle = deps.iii.observe(
        Source::function(crate::hooks::ON_POST_TURN_FN).args(args),
        validator.as_str(),
    );
    let observer_id = deps.hooks.record_owned(session_id, handle.clone());
    drop(topology);
    if let Readiness::Failed(reason) = readiness(&handle).await {
        deps.hooks.dispose_owned(&observer_id, session_id).await;
        return Err(HarnessError::InvalidRequest(format!(
            "post-turn validator observer failed: {reason}"
        )));
    }
    Ok(ObserveResponse {
        observer_id,
        once: false,
        note: Some(format!(
            "post-turn validator bound to THIS session only ({session_id}); unobserve it with \
             this id when the goal is met."
        )),
    })
}

/// The sessions an agent-created post-turn validator may gate: itself, or
/// children it names under the `<own-session>-` prefix (the `harness::spawn`
/// `session_id` convention). No patterns → just itself. Anything else is out
/// of scope — an agent must never gate sessions it does not own.
fn validate_post_turn_scope(
    session_id: &str,
    requested: Option<Vec<String>>,
) -> Result<Vec<String>, String> {
    let Some(patterns) = requested.filter(|p| !p.is_empty()) else {
        return Ok(vec![session_id.to_string()]);
    };
    let child_prefix = format!("{session_id}-");
    for pattern in &patterns {
        if pattern != session_id && !pattern.starts_with(&child_prefix) {
            // A foreign id carrying the spawn `-child-` convention is a prompt
            // written for ANOTHER session (a copied example, an e2e-authored
            // prompt run in a console chat): still refused, but name the exact
            // substitution so the retry is mechanical, not inferential.
            let suggestion = pattern
                .split_once("-child-")
                .map(|(_, tail)| {
                    format!(
                        ". This pattern names another session's child — for a child of THIS \
                         session use `{child_prefix}child-{tail}` (and spawn it under that id)"
                    )
                })
                .unwrap_or_default();
            return Err(format!(
                "post-turn hook sessions pattern `{pattern}` is out of scope: an agent may \
                 gate itself (`{session_id}`) or children it spawns under its own prefix \
                 (`{child_prefix}*`) only{suggestion}"
            ));
        }
    }
    Ok(patterns)
}

/// Gate a call binding's target. `harness::*` is refused outright — a binding
/// can wake its owner or call a plain function, never start an agent or
/// re-enter the harness control plane (the observe controls included) — and
/// everything else must be callable by the OBSERVER: the delivered call runs
/// with worker authority outside the turn loop, so without this a narrowed
/// session could wire a reaction to any function on the bus.
fn validate_call_target(target: &str, policy: &CompiledPolicy) -> Result<(), String> {
    if target.is_empty() {
        return Err("`function_id` must name the function to call".into());
    }
    if target.starts_with("harness::") {
        return Err(format!(
            "`{target}` is not a binding target: a binding can wake you (omit `target`) or \
             call a plain non-harness function — it never starts an agent. Spawn children \
             directly from a turn and observe what they write."
        ));
    }
    if agent_call_forbidden(target) {
        return Err(format!(
            "`{target}` is not available to agents, including through bindings"
        ));
    }
    if !policy.allows(target) {
        return Err(format!(
            "`{target}` is not permitted by this session's dispatch policy — a reaction can \
             only call functions you can call yourself"
        ));
    }
    // A reaction's payload is fixed or projected only at delivery time, where
    // `start: true` would run compose::add with the ide's authority; so the
    // target needs compose::add whatever the payload says now.
    if target == crate::clients::engine::SCAFFOLD_WORKER && !policy.allows("compose::add") {
        return Err(format!(
            "`{target}` as a reaction needs compose::add as well: its `start: true` runs \
             compose::add, and a delivered payload can set it. Call it from a turn instead"
        ));
    }
    Ok(())
}

/// The dispatch policy is only HALF the gate: it says what this turn may call,
/// not whether the deployment wants a human to approve each call. A call an
/// observer delivers runs with worker authority outside any turn, so it never
/// reaches `approval::gate` and can never prompt — binding a `needs_approval`
/// target would silently convert "a human approves each call" into "it runs
/// unattended, forever".
///
/// So ask the approval-gate what it WOULD decide (`approval::evaluate` is
/// side-effect-free — probing must not write a pending record nobody asked
/// for) and bind only what it allows outright.
///
/// Fail-open ONLY when the gate is absent: with no approval worker registered,
/// a direct call needs no approval either, so the reaction grants nothing the
/// session did not already have. Any other failure fails CLOSED — an
/// undetermined verdict must not become a standing unattended call.
pub(crate) async fn approval_allows_unattended(
    deps: &Deps,
    target: &str,
    session_id: &str,
    arguments: &Value,
) -> Result<(), String> {
    match approval_verdict(deps, target, session_id, arguments).await {
        Ok(Verdict::Allow) => Ok(()),
        Ok(Verdict::Withheld { verdict, reason }) => Err(format!(
            "`{target}` is `{verdict}` for this session: {reason}. A call an observer delivers \
             runs outside any turn and cannot ask for approval, so it cannot be bound. Call it \
             yourself on a woken turn, or have an operator allow it."
        )),
        Err(problem) => {
            tracing::warn!(target, error = %problem, "approval probe failed; refusing the binding");
            Err(format!(
                "could not check whether `{target}` needs approval ({problem}); refusing to \
                 bind a reaction that would run unattended"
            ))
        }
    }
}

/// What the approval gate would decide, without a pending record.
enum Verdict {
    /// `allow`, or no approval gate on the bus.
    Allow,
    Withheld {
        verdict: String,
        reason: String,
    },
}

/// Ask `approval::evaluate` (side-effect-free). Open ONLY when the gate is
/// absent; an undetermined verdict is an `Err`, which callers fail closed.
async fn approval_verdict(
    deps: &Deps,
    target: &str,
    session_id: &str,
    arguments: &Value,
) -> Result<Verdict, String> {
    let resp = deps
        .iii
        .call(CallRequest {
            function_id: APPROVAL_EVALUATE_ID.to_string(),
            payload: approval_probe_payload(session_id, target, arguments),
            action: None,
            timeout_ms: Some(deps.cfg().await.dispatch_timeout_ms),
        })
        .await;
    let resp = match resp {
        Ok(v) => v,
        Err(e) => {
            let msg = e.to_string();
            if is_function_absent(&msg) {
                tracing::debug!(target, "approval-gate absent; no approval probe");
                return Ok(Verdict::Allow);
            }
            return Err(msg);
        }
    };
    match resp.get("verdict").and_then(Value::as_str) {
        Some("allow") => Ok(Verdict::Allow),
        Some(other) => Ok(Verdict::Withheld {
            verdict: other.to_string(),
            reason: resp
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("no reason given")
                .to_string(),
        }),
        None => Err("the approval gate returned no verdict".to_string()),
    }
}

/// A scaffold that starts runs compose::add inside the ide, where no approval
/// hook sees it. So the turn asks the approval gate about compose::add for
/// this session first; anything but `allow` is a start this turn cannot have
/// (files only, or refused), and the agent's own compose::add then goes
/// through the gate, held for a human like any other. Policy is the
/// dispatch gate's (`call::scaffold_start_gate`).
pub(crate) async fn scaffold_start_approval(
    deps: &Deps,
    policy: &CompiledPolicy,
    function_id: &str,
    arguments: &Value,
    session_id: &str,
) -> call::StartGate {
    if !call::scaffold_starts(function_id, arguments) || !policy.allows(COMPOSE_ADD_ID) {
        return call::StartGate::Pass;
    }
    let verdict = approval_verdict(deps, COMPOSE_ADD_ID, session_id, &json!({})).await;
    start_after_approval(arguments, verdict)
}

/// The turn's decision on a starting scaffold, given what the approval gate
/// said about its compose::add.
fn start_after_approval(arguments: &Value, verdict: Result<Verdict, String>) -> call::StartGate {
    let why = match verdict {
        Ok(Verdict::Allow) => return call::StartGate::Pass,
        Ok(Verdict::Withheld { verdict, reason }) => {
            format!("its compose::add is `{verdict}` for this session ({reason})")
        }
        Err(problem) => {
            format!("the approval gate could not be asked about its compose::add ({problem})")
        }
    };
    call::start_refused(
        arguments,
        &why,
        "Then send its compose_add to compose::add yourself: that call goes through approval, \
         where a human can allow it.",
    )
}

/// The function a starting `coder::scaffold-worker` runs inside the ide.
pub(crate) const COMPOSE_ADD_ID: &str = "compose::add";

fn approval_probe_payload(session_id: &str, target: &str, arguments: &Value) -> Value {
    json!({
        "session_id": session_id,
        "function_id": target,
        "arguments": arguments,
    })
}

/// Distinguish "no approval worker registered" from a real failure. The engine
/// reports an unregistered id rather than a transport problem, and only that
/// case is safe to treat as open.
fn is_function_absent(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    e.contains("function_not_found")
        || e.contains("not found")
        || e.contains("no worker")
        || e.contains("unregistered")
}

/// The Compose supervisor's namespace (`III_COMPOSE_NAMESPACE`), if set.
pub(crate) fn compose_namespace() -> Option<String> {
    let namespace = std::env::var(COMPOSE_NAMESPACE_ENV).ok()?;
    let namespace = namespace.trim();
    (!namespace.is_empty()).then(|| namespace.to_string())
}

/// Where an observed function is hosted, as far as the engine could say.
struct SourceHost {
    /// The namespace the observer names; `None` resolves like a call (this
    /// worker's namespace).
    namespace: Option<String>,
    /// The namespaces checked when no worker hosts the function anywhere
    /// yet; empty when it was found or the probe could not answer.
    unhosted_in: Vec<String>,
}

impl SourceHost {
    /// Loud advisory when NOTHING currently hosts the observed function: the
    /// observer is accepted engine-side but stays `pending` and cannot
    /// deliver until a worker registers the function. Without this the
    /// response reads "CREATED" and the owner sleeps forever on an observer
    /// that was never live (console-04e02cb7 postmortem).
    fn parked_note(&self, function_id: &str) -> Option<String> {
        if self.unhosted_in.is_empty() {
            return None;
        }
        Some(format!(
            "warning: OBSERVING but currently PENDING — no connected worker hosts `{function_id}` \
             (checked namespace(s): {}). Nothing can deliver to this observer until a worker \
             registers the function; it then activates automatically. If its worker should be \
             running (e.g. the state worker for `state::on-changed`), start it.",
            self.unhosted_in.join(", ")
        ))
    }
}

/// Resolve the source namespace: the agent's explicit one, else the first of
/// this harness's namespace, `default` and Compose's that hosts the function.
/// When none does yet, the observer waits in this harness's namespace
/// (Compose's for a `compose::*` function). Fail-open: a probe error that is
/// not a definitive NOT_FOUND stops probing and leaves the namespace unset.
async fn resolve_source_namespace(deps: &Deps, req: &ObserveRequest) -> SourceHost {
    if let Some(namespace) = req
        .namespace
        .as_deref()
        .map(str::trim)
        .filter(|namespace| !namespace.is_empty())
    {
        return SourceHost {
            namespace: Some(namespace.to_string()),
            unhosted_in: Vec::new(),
        };
    }
    let own = deps.iii.namespace();
    let compose = compose_namespace();
    let candidates = source_namespace_candidates(own.as_deref(), compose.as_deref());
    let timeout_ms = deps.cfg().await.dispatch_timeout_ms;
    for namespace in &candidates {
        let probe = deps
            .iii
            .call(CallRequest {
                function_id: FUNCTIONS_INFO_ID.to_string(),
                payload: json!({ "function_id": req.function_id, "namespace": namespace }),
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await;
        match probe {
            Ok(info) if !info.is_null() => {
                return SourceHost {
                    namespace: Some(namespace.clone()),
                    unhosted_in: Vec::new(),
                }
            }
            Ok(_) => {}
            Err(e) if e.to_string().contains("NOT_FOUND") => {}
            Err(_) => {
                return SourceHost {
                    namespace: None,
                    unhosted_in: Vec::new(),
                }
            }
        }
    }
    let fallback = if req.function_id.starts_with("compose::") {
        compose.or(own)
    } else {
        own
    };
    SourceHost {
        namespace: fallback,
        unhosted_in: candidates,
    }
}

/// The namespaces an unqualified observe looks in, in order: this harness's,
/// `default`, then Compose's.
fn source_namespace_candidates(own: Option<&str>, compose: Option<&str>) -> Vec<String> {
    let mut namespaces: Vec<String> = Vec::new();
    for namespace in [own, Some("default"), compose].into_iter().flatten() {
        if !namespaces.iter().any(|seen| seen == namespace) {
            namespaces.push(namespace.to_string());
        }
    }
    namespaces
}

fn ok_result<T: Serialize>(value: &T) -> ResultData {
    let details = serde_json::to_value(value).unwrap_or(Value::Null);
    ResultData {
        content: vec![ContentBlock::text(
            serde_json::to_string(&details).unwrap_or_default(),
        )],
        is_error: false,
        details,
    }
}

fn error_result(msg: String) -> ResultData {
    ResultData {
        content: vec![ContentBlock::text(msg.clone())],
        is_error: true,
        details: json!({ "error": msg }),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn req(body: Value) -> ObserveRequest {
        serde_json::from_value(body).unwrap()
    }

    fn state_filters(scope: Option<&str>, key: Option<&str>) -> Value {
        let mut filters = json!({});
        if let Some(scope) = scope {
            filters["input.scope"] = json!({ "$eq": scope });
        }
        if let Some(key) = key {
            filters["input.key"] = json!({ "$eq": key });
        }
        filters
    }

    fn state(scope: Option<&str>, key: Option<&str>) -> Value {
        json!({ "function_id": STATE_ON_CHANGED_FN, "filters": state_filters(scope, key) })
    }

    /// An unqualified observe looks home-first: this harness's namespace,
    /// then `default`, then Compose's — each once.
    #[test]
    fn unqualified_sources_resolve_home_then_default_then_compose() {
        assert_eq!(
            source_namespace_candidates(Some("project"), Some("compose-host")),
            vec!["project", "default", "compose-host"]
        );
        assert_eq!(source_namespace_candidates(None, None), vec!["default"]);
        assert_eq!(
            source_namespace_candidates(Some("default"), Some("default")),
            vec!["default"]
        );
    }

    #[test]
    fn a_parked_note_names_every_namespace_checked() {
        let host = SourceHost {
            namespace: Some("project".into()),
            unhosted_in: vec!["project".into(), "default".into()],
        };
        let note = host.parked_note("state::on-changed").unwrap();
        assert!(note.contains("PENDING"), "{note}");
        assert!(note.contains("project, default"), "{note}");
        let found = SourceHost {
            namespace: Some("default".into()),
            unhosted_in: Vec::new(),
        };
        assert!(found.parked_note("state::on-changed").is_none());
    }

    /// An agent may gate itself and its own `<session>-` children; anything
    /// else is out of scope. No patterns → just itself.
    #[test]
    fn post_turn_scope_is_self_or_own_children() {
        assert_eq!(
            validate_post_turn_scope("run-1", None).unwrap(),
            vec!["run-1"]
        );
        assert_eq!(
            validate_post_turn_scope("run-1", Some(vec![])).unwrap(),
            vec!["run-1"]
        );
        assert_eq!(
            validate_post_turn_scope("run-1", Some(vec!["run-1".into(), "run-1-child-*".into()]))
                .unwrap(),
            vec!["run-1", "run-1-child-*"]
        );
        // Foreign session, sibling prefix without the dash, and wildcards
        // outside the namespace are refused.
        assert!(validate_post_turn_scope("run-1", Some(vec!["other".into()])).is_err());
        assert!(validate_post_turn_scope("run-1", Some(vec!["run-10".into()])).is_err());
        assert!(validate_post_turn_scope("run-1", Some(vec!["*".into()])).is_err());
    }

    /// A refused FOREIGN pattern that carries the spawn `-child-` convention
    /// is a prompt written for another session (a copied example, an
    /// e2e-authored prompt pasted into a console chat). Still refused — but
    /// the error names the exact in-scope substitution, exact tail included,
    /// so the model's retry is mechanical. A foreign id without the
    /// convention gets no guess.
    #[test]
    fn out_of_scope_child_pattern_names_the_substitution() {
        let err = validate_post_turn_scope("console-2c21", Some(vec!["e2e_12ac-child-1".into()]))
            .unwrap_err();
        assert!(err.contains("out of scope"), "refusal must stand: {err}");
        assert!(
            err.contains("use `console-2c21-child-1`"),
            "the substitution must be spelled out: {err}"
        );

        let glob =
            validate_post_turn_scope("run-1", Some(vec!["other-child-*".into()])).unwrap_err();
        assert!(
            glob.contains("use `run-1-child-*`"),
            "the glob tail must survive the substitution: {glob}"
        );

        let plain = validate_post_turn_scope("run-1", Some(vec!["other".into()])).unwrap_err();
        assert!(
            !plain.contains("another session's child"),
            "no substitution guess without the child convention: {plain}"
        );
    }

    /// Turn events are not agent-observable in ANY shape — child outcomes
    /// flow through the medium children write, never through an observer on
    /// their turns. The guard runs before target resolution, so wake, call,
    /// and would-be spawn shapes all hit it. The timer is the one exception.
    #[test]
    fn turn_events_are_not_observable_in_any_shape() {
        for id in [
            crate::events::ON_TURN_STARTED_FN,
            crate::events::ON_TURN_COMPLETED_FN,
            crate::hooks::ON_PRE_GENERATE_FN,
            "harness::notify_agent",
        ] {
            let err = reject_forbidden_source(id).unwrap_err().to_string();
            assert!(err.contains("not agent-observable"), "{id}: {err}");
            assert!(
                err.contains("observe the state keys"),
                "{id} must point at the medium instead: {err}"
            );
        }
        assert!(reject_forbidden_source(STATE_ON_CHANGED_FN).is_ok());
        assert!(reject_forbidden_source("database::on-row-changed").is_ok());
        assert!(reject_forbidden_source(CRON_ON_TICK_FN).is_ok());
        assert!(reject_forbidden_source(crate::timer::ON_TIMER_FN).is_ok());
    }

    #[test]
    fn state_catchall_advisory_flags_keyless_state_observers() {
        // Scope without key: the trade-off, naming the scope — NOT a push to
        // add a key. Run 7 read the old push as an instruction, pinned an
        // exact key, and its per-event unique keys then matched nothing.
        let note = state_catchall_advisory(&req(state(Some("run-1"), None)), false);
        let note = note.as_deref().unwrap_or("");
        assert!(note.contains("run-1"), "got: {note}");
        assert!(note.contains("no `input.key` filter"), "got: {note}");
        assert!(
            note.contains("RIGHT shape when producers write one unique key per event"),
            "the legitimate scope-only design must be endorsed: {note}"
        );
        assert!(
            note.contains("ONLY if every producer writes that exact key"),
            "adding a key must be conditioned on the producers: {note}"
        );
        // No scope AND no key: the global catch-all, still warned hard.
        let note = state_catchall_advisory(&req(state(None, None)), false);
        assert!(note.as_deref().unwrap_or("").contains("EVERY scope"));
        // A key filter silences it, with or without scope.
        for once in [false, true] {
            assert!(
                state_catchall_advisory(&req(state(Some("run-1"), Some("change"))), once).is_none()
            );
            assert!(state_catchall_advisory(&req(state(None, Some("change"))), once).is_none());
            // Other functions are not advised on.
            assert!(
                state_catchall_advisory(&req(json!({ "function_id": CRON_ON_TICK_FN })), once)
                    .is_none()
            );
        }
    }

    /// The fan-in wave shape (MOT-5408): a scope-only one-shot wake gated by
    /// `state::barrier` with a lifecycle deadline. It delivers one reaction,
    /// on the write the barrier allows, and the deadline already turns
    /// silence into a notice, so neither advisory has anything true to say.
    #[test]
    fn a_barrier_gated_one_shot_wake_with_a_deadline_gets_no_advisory() {
        let wave = req(json!({
            "function_id": STATE_ON_CHANGED_FN,
            "filters": state_filters(Some("run-r1-w1"), None),
            "conditions": [{
                "function_id": "state::barrier",
                "config": { "id": "run-r1-w1", "expect": ["KAN-12", "KAN-13"], "carry": "/event_type" }
            }],
            "lifecycle": { "expires_in_ms": 1_260_000 }
        }));
        let once = effective_once(&wave);
        assert!(once, "a wake defaults to one-shot");
        let has_deadline = resolved_expires_at(wave.lifecycle.as_ref(), 0).is_some();
        assert!(has_deadline, "expires_in_ms resolves to a deadline");
        assert!(state_catchall_advisory(&wave, once).is_none());
        assert!(armed_wake_advisory(&wave, once, has_deadline).is_none());
    }

    #[test]
    fn a_keyless_one_shot_state_wake_is_told_which_write_wins() {
        let barrier = |expect: serde_json::Value| json!([{ "function_id": "state::barrier", "config": { "id": "b", "expect": expect } }]);
        let with_conditions = |scope: Option<&str>, conditions: Value| {
            let mut body = state(scope, None);
            body["conditions"] = conditions;
            req(body)
        };
        // No condition: the first write of any key removes it.
        let note = state_catchall_advisory(&req(state(Some("run-1"), None)), true);
        let note = note.as_deref().unwrap_or("");
        assert!(note.contains("FIRST write of ANY key"), "got: {note}");
        assert!(note.contains("run-1"), "got: {note}");
        assert!(
            !note.contains("every matching write"),
            "a one-shot is removed on its first delivery: {note}"
        );
        assert!(!note.contains("passes its conditions"), "got: {note}");
        // A count barrier counts ANY key, so a stray write can still complete
        // it: the note stays, naming the gate.
        let note =
            state_catchall_advisory(&with_conditions(Some("run-1"), barrier(json!(3))), true);
        let note = note.as_deref().unwrap_or("");
        assert!(note.contains("that passes its conditions"), "got: {note}");
        assert!(note.contains("`expect` array"), "got: {note}");
        // No scope: conditions or not, every write anywhere reaches it.
        for conditions in [json!([]), barrier(json!(["a"]))] {
            let note = state_catchall_advisory(&with_conditions(None, conditions), true);
            assert!(note.as_deref().unwrap_or("").contains("ANY scope"));
        }
        // Standing: a named barrier does not silence it, and the reaction is
        // per write the conditions allow.
        let note = state_catchall_advisory(
            &with_conditions(Some("run-1"), barrier(json!(["a"]))),
            false,
        );
        let note = note.as_deref().unwrap_or("");
        assert!(
            note.contains("every matching write that passes its conditions"),
            "got: {note}"
        );
    }

    #[test]
    fn armed_wake_advisory_names_the_exact_key_for_one_shot_state_wakes() {
        // A one-shot state wake: warned, naming scope/key, with the
        // sleeps-forever consequence spelled out.
        let note = armed_wake_advisory(
            &req(state(Some("run-1"), Some("report_ready"))),
            true,
            false,
        );
        let note = note.as_deref().unwrap_or("");
        assert!(note.contains("run-1/report_ready"), "got: {note}");
        assert!(note.contains("sleeps forever"), "got: {note}");
        // The discovery-run failure modes, named: a DATABASE table with the
        // watched name does not wake a state observer, and a lifecycle
        // deadline is what converts eternal silence into a notice.
        assert!(
            note.contains("database table or row with the same name does NOT"),
            "the medium sentence is the advisory's whole point: {note}"
        );
        assert!(note.contains("expires_in_ms"), "got: {note}");
        assert!(note.contains("expiry notice"), "got: {note}");
        // Every state write wakes it, compare-and-set and update included
        // (the state worker calls state::on-changed for each of them).
        for write in [
            "state::set",
            "state::update",
            "state::compare-and-set",
            "state::delete",
        ] {
            assert!(note.contains(write), "{write} missing: {note}");
        }
        assert!(
            !note.contains("passes its conditions"),
            "no condition, no gate: {note}"
        );
        // A condition gates the wake: say so.
        let mut gated = state(Some("run-1"), Some("report_ready"));
        gated["conditions"] =
            json!([{ "function_id": "state::barrier", "config": { "id": "b", "expect": ["a"] } }]);
        let note = armed_wake_advisory(&req(gated), true, false);
        assert!(note
            .as_deref()
            .unwrap_or("")
            .contains("that passes its conditions"));
        // A deadline (expires_in_ms, or a leaf's capped wake) already
        // converts silence into a notice.
        let keyed = req(state(Some("run-1"), Some("report_ready")));
        assert!(armed_wake_advisory(&keyed, true, true).is_none());
        // A lifecycle without a deadline does not: still warned.
        for lifecycle in [json!({ "once": true }), json!({ "max_fires": 1 })] {
            let mut body = state(Some("run-1"), Some("report_ready"));
            body["lifecycle"] = lifecycle;
            let r = req(body);
            let has_deadline = resolved_expires_at(r.lifecycle.as_ref(), 0).is_some();
            assert!(armed_wake_advisory(&r, true, has_deadline).is_some());
        }
        // Standing notifies and other functions are not wakes.
        assert!(armed_wake_advisory(&req(state(None, Some("k"))), false, false).is_none());
        assert!(
            armed_wake_advisory(&req(json!({ "function_id": CRON_ON_TICK_FN })), true, false)
                .is_none()
        );
    }

    #[test]
    fn standing_binding_advisory_notes_non_once_only() {
        let mk = |function_id: &str| {
            req(json!({
                "function_id": function_id,
                "target": { "function_id": "state::set" },
            }))
        };
        // The leak shape: a state kickoff observed without once.
        let note = standing_binding_advisory(&mk(STATE_ON_CHANGED_FN), false);
        assert!(note.as_deref().unwrap_or("").contains("STANDING"));
        // once removes itself.
        assert!(standing_binding_advisory(&mk(STATE_ON_CHANGED_FN), true).is_none());
        assert!(standing_binding_advisory(&mk(CRON_ON_TICK_FN), true).is_none());
    }

    /// MOT-5449: a wake asked to stand (`once: false`) has no call to re-run;
    /// its cost is a paid turn per event, and one-shot is its default, not
    /// something to tell it to pass.
    #[test]
    fn a_standing_wake_is_not_told_it_is_a_call_binding() {
        let mk = |lifecycle: Value| {
            let mut v = state(Some("run-1"), None);
            if !lifecycle.is_null() {
                v["lifecycle"] = lifecycle;
            }
            req(v)
        };
        let note = standing_binding_advisory(&mk(Value::Null), false);
        let note = note.as_deref().unwrap_or("");
        assert!(note.contains("WAKE is STANDING"), "got: {note}");
        assert!(note.contains("paid turn"), "got: {note}");
        assert!(!note.contains("call"), "a wake has no call: {note}");
        // A lifecycle bound already stops it waking forever.
        assert!(standing_binding_advisory(&mk(json!({ "max_fires": 3 })), false).is_none());
        assert!(
            standing_binding_advisory(&mk(json!({ "expires_in_ms": 60_000 })), false).is_none()
        );
    }

    /// Both discovery runs created a recurring cron with no lifecycle and
    /// left it firing past their own teardown claims — an unbounded cron is
    /// never what an agent meant, so it gets a warning of its own.
    #[test]
    fn an_unbounded_recurring_cron_is_warned() {
        let mk = |lifecycle: Value| {
            let mut v = json!({
                "function_id": CRON_ON_TICK_FN,
                "args": { "expression": "0 */10 * * * *" },
                "target": { "function_id": "state::set" },
            });
            if !lifecycle.is_null() {
                v["lifecycle"] = lifecycle;
            }
            req(v)
        };
        let note = standing_binding_advisory(&mk(Value::Null), false);
        let note = note.as_deref().unwrap_or("");
        assert!(note.contains("UNBOUNDED"), "got: {note}");
        assert!(
            note.contains("0 */10 * * * *"),
            "the expression is named: {note}"
        );
        assert!(note.contains("CREATED"), "redo-thrash guard: {note}");
        assert!(
            note.contains("harness::on-timer") && note.contains("once: true"),
            "both deadline fixes are named: {note}"
        );
        // Any lifecycle bound silences it — bounded recurrence is deliberate.
        assert!(standing_binding_advisory(&mk(json!({ "max_fires": 6 })), false).is_none());
        assert!(
            standing_binding_advisory(&mk(json!({ "expires_in_ms": 3_600_000 })), false).is_none()
        );
    }

    /// `{ in_ms }` resolves to an absolute `at` BEFORE the observer is
    /// created — a relative countdown replayed after a restart would silently
    /// restart from zero. And the nonsense shapes fail loudly when observing.
    #[test]
    fn timer_observers_resolve_relative_and_refuse_nonsense() {
        let mk = |args: Value, once: Option<bool>| {
            let mut v = json!({ "function_id": crate::timer::ON_TIMER_FN, "args": args });
            if let Some(o) = once {
                v["once"] = json!(o);
            }
            req(v)
        };
        let now = crate::types::message::AgentMessage::now_ms();

        let mut rel = mk(json!({ "in_ms": 600_000 }), None);
        resolve_timer_request(&mut rel).unwrap();
        let at = rel.args["at"].as_i64().unwrap();
        assert!(
            (at - now - 600_000).abs() < 5_000,
            "at ≈ now + in_ms, got {at}"
        );
        assert!(
            rel.args.get("in_ms").is_none(),
            "args are normalized to `at` only"
        );
        assert!(effective_once(&rel), "a timer defaults to once");

        let err = resolve_timer_request(&mut mk(json!({ "in_ms": 1000 }), Some(false)))
            .unwrap_err()
            .to_string();
        assert!(err.contains("exactly once"), "got: {err}");

        // Epoch-seconds (1000× off) reads as deep past — named, not armed.
        let err = resolve_timer_request(&mut mk(json!({ "at": now / 1000 }), None))
            .unwrap_err()
            .to_string();
        assert!(err.contains("MILLISECONDS"), "got: {err}");

        let err = resolve_timer_request(&mut mk(
            json!({ "at": now + crate::timer::MAX_TIMER_MS + 60_000 }),
            None,
        ))
        .unwrap_err()
        .to_string();
        assert!(err.contains("7 days"), "got: {err}");

        let err = resolve_timer_request(&mut mk(json!({ "at": now + 5000, "in_ms": 5000 }), None))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not both"), "got: {err}");

        // Other functions pass through untouched.
        let mut cron = req(json!({
            "function_id": CRON_ON_TICK_FN,
            "args": { "expression": "0 * * * * *" }
        }));
        resolve_timer_request(&mut cron).unwrap();
        assert_eq!(cron.args["expression"], json!("0 * * * * *"));
    }

    /// The run-3 lesson: notes must SAY the observer was created, or a
    /// warning reads as a redo order and the model starts over.
    #[test]
    fn composed_notes_lead_with_the_outcome() {
        assert!(compose_note(vec![]).is_none());
        let note = compose_note(vec!["warning: x.".into(), "note: y.".into()]).unwrap();
        assert!(note.starts_with("observer CREATED"), "got: {note}");
        assert!(note.contains("rather than starting over"), "got: {note}");
        assert!(note.contains("warning: x.") && note.contains("note: y."));
    }

    /// The barrier-armed-late shape: a keyed state observer is checkable for
    /// a pre-existing value when it is created; the note explains no-replay.
    #[test]
    fn prewritten_key_advisory_targets_keyed_state_observers() {
        assert_eq!(
            watched_state_key(&req(state(Some("run"), Some("done")))),
            Some(("run", "done"))
        );
        // Keyless and non-state shapes are never probed.
        assert!(watched_state_key(&req(state(Some("run"), None))).is_none());
        assert!(watched_state_key(&req(json!({
            "function_id": CRON_ON_TICK_FN,
            "filters": state_filters(Some("run"), Some("done")),
        })))
        .is_none());

        let note = prewritten_key_note("receiving_events", "done");
        assert!(note.contains("receiving_events/done"), "got: {note}");
        assert!(note.contains("do not replay"), "got: {note}");
        assert!(
            note.contains("BEFORE starting them"),
            "the ordering rule is the fix: {note}"
        );
    }

    /// The default-by-shape matrix — the semantic that killed three of five
    /// discovery runs when one rule covered everything. A wake parks and must
    /// not silently re-arm; a call binding is per-event and must not silently
    /// retire at delivery one.
    #[test]
    fn once_defaults_by_shape() {
        let call = json!({ "function_id": "state::set" });

        // Wakes (no target): once.
        assert!(effective_once(&req(state(None, None))));
        assert!(effective_once(&req(json!({
            "function_id": "database::on-row-changed",
            "args": { "db": "primary" }
        }))));

        // Call bindings: STANDING.
        assert!(!effective_once(&req(json!({
            "function_id": "database::on-row-changed",
            "target": call,
        }))));

        // Cron recurring, timer once — by definition, regardless of target.
        assert!(!effective_once(&req(
            json!({ "function_id": CRON_ON_TICK_FN })
        )));
        assert!(!effective_once(&req(json!({
            "function_id": CRON_ON_TICK_FN,
            "target": call,
        }))));
        assert!(effective_once(&req(json!({
            "function_id": crate::timer::ON_TIMER_FN,
            "target": call,
        }))));

        // Explicit always wins, in both directions.
        assert!(effective_once(&req(
            json!({ "function_id": CRON_ON_TICK_FN, "once": true })
        )));
        assert!(!effective_once(&req(
            json!({ "function_id": STATE_ON_CHANGED_FN, "once": false })
        )));
        assert!(effective_once(&req(json!({
            "function_id": STATE_ON_CHANGED_FN,
            "target": call,
            "once": true,
        }))));
        assert!(effective_once(&req(json!({
            "function_id": STATE_ON_CHANGED_FN,
            "target": call,
            "lifecycle": { "once": true },
        }))));
        assert!(!effective_once(&req(json!({
            "function_id": STATE_ON_CHANGED_FN,
            "lifecycle": { "once": false },
        }))));
    }

    #[test]
    fn lifecycle_deadlines_are_relative_only() {
        assert!(validate_lifecycle(None).is_ok());
        assert!(validate_lifecycle(Some(&LifecycleRequest {
            max_fires: Some(0),
            ..Default::default()
        }))
        .is_err());
        for bad in [0_i64, -5] {
            let err = validate_lifecycle(Some(&LifecycleRequest {
                expires_in_ms: Some(bad),
                ..Default::default()
            }))
            .unwrap_err()
            .to_string();
            assert!(err.contains("must be positive"), "got: {err}");
        }
        assert!(validate_lifecycle(Some(&LifecycleRequest {
            max_fires: Some(1),
            expires_in_ms: Some(60_000),
            ..Default::default()
        }))
        .is_ok());

        // Relative resolves against the observe clock.
        let lifecycle = LifecycleRequest {
            expires_in_ms: Some(60_000),
            ..Default::default()
        };
        assert_eq!(resolved_expires_at(Some(&lifecycle), 1_000), Some(61_000));
        assert_eq!(resolved_expires_at(None, 1_000), None);
        assert_eq!(
            resolved_expires_at(Some(&LifecycleRequest::default()), 1_000),
            None
        );
    }

    /// The retired absolute form must be a NAMED error, not a silently
    /// dropped deadline: models carried `expires_at` over from old contracts
    /// and every observed value was a stale epoch guess.
    #[test]
    fn absolute_expires_at_is_rejected_by_name() {
        let err = serde_json::from_value::<LifecycleRequest>(json!({
            "once": true,
            "expires_at": 1_770_000_000_000_i64
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("expires_at"), "got: {err}");
        assert!(err.contains("expires_in_ms"), "got: {err}");
    }

    #[test]
    fn an_expired_reservation_is_not_reported_as_capacity() {
        assert!(require_reserved(ReserveOutcome::Reserved).is_ok());
        let full = require_reserved(ReserveOutcome::Capacity)
            .unwrap_err()
            .to_string();
        assert!(full.contains("observer cap reached"));

        let expired = require_reserved(ReserveOutcome::Exhausted)
            .unwrap_err()
            .to_string();
        assert!(expired.contains("lifecycle expired"));
        assert!(!expired.contains("cap reached"));
    }

    #[test]
    fn in_turn_observer_calls_are_forced_to_the_calling_session() {
        // Absent, null, and forged explicit ids all take the caller.
        let filled = with_caller_session_id(&json!({}), "s_me");
        assert_eq!(filled["session_id"], "s_me");
        let filled = with_caller_session_id(&json!({ "session_id": null }), "s_me");
        assert_eq!(filled["session_id"], "s_me");
        let replaced = with_caller_session_id(&json!({ "session_id": "s_other" }), "s_me");
        assert_eq!(replaced["session_id"], "s_me");
        // Non-object args pass through for the target to reject.
        assert_eq!(
            with_caller_session_id(&json!("nope"), "s_me"),
            json!("nope")
        );
    }

    #[test]
    fn trusted_self_send_is_intercepted_locally() {
        let arguments = json!({ "session_id": "s", "message": "hello" });
        let request = send_invocation_context(crate::functions::SEND_ID, &arguments, "s", true)
            .expect("trusted self-send should be intercepted");
        assert_eq!(request.session_id.as_deref(), Some("s"));
    }

    #[test]
    fn sends_without_trusted_exact_self_target_use_engine_dispatch() {
        let cases = [
            json!({ "session_id": "other", "message": "hello" }),
            json!({ "message": "hello" }),
            json!({ "session_id": 42, "message": "hello" }),
            json!({ "session_id": "s", "message": 42 }),
        ];
        for arguments in cases {
            assert!(
                send_invocation_context(crate::functions::SEND_ID, &arguments, "s", true,)
                    .is_none()
            );
        }

        let self_send = json!({ "session_id": "s", "message": "hello" });
        assert!(
            send_invocation_context(crate::functions::SEND_ID, &self_send, "s", false,).is_none()
        );
        assert!(send_invocation_context("state::get", &self_send, "s", true).is_none());
    }

    /// The intercepted controls are published by the harness itself for
    /// native exposure — but only when the turn's policy allows them.
    #[test]
    fn native_observe_controls_follow_dispatch_policy() {
        let both = policy_allowing(&[OBSERVE_ID, UNOBSERVE_ID]);
        let tools = native_control_tools(&both);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, [OBSERVE_ID, UNOBSERVE_ID]);
        assert!(tools
            .iter()
            .all(|t| t.execution_mode.as_deref() == Some("sequential")));

        let observe_only = policy_allowing(&[OBSERVE_ID]);
        let names: Vec<String> = native_control_tools(&observe_only)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, [OBSERVE_ID]);

        let mut walled = policy_allowing(&["*"]);
        assert!(native_control_tools(&walled).len() == 2);
        // Under the leaf wall the toolset keeps observe and loses unobserve:
        // a leaf arms its own wake but cannot retract anyone's observer, which
        // is why its wake must be self-limiting (MOT-4766).
        walled = CompiledPolicy::from(Some(&crate::types::turn::FunctionPolicy {
            allow: vec!["*".into()],
            deny: crate::policy::CONTROL_PLANE_DENY
                .iter()
                .map(|s| s.to_string())
                .collect(),
            expose: Default::default(),
        }));
        let names: Vec<String> = native_control_tools(&walled)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, [OBSERVE_ID]);
    }

    fn leaf_policy() -> CompiledPolicy {
        CompiledPolicy::from(Some(&crate::types::turn::FunctionPolicy {
            allow: vec!["*".into()],
            deny: crate::policy::CONTROL_PLANE_DENY
                .iter()
                .map(|s| s.to_string())
                .collect(),
            expose: Default::default(),
        }))
    }

    // The half a leaf keeps: park on your own work instead of polling for it.
    #[test]
    fn a_leaf_may_arm_a_wake_for_itself() {
        assert!(reject_leaf_control_plane(&leaf_policy(), true, None).is_ok());
    }

    // A root under a narrow allow-list cannot spawn either, and reading that
    // as leafness refused its mechanical-call bindings — which is how this
    // change first broke INT-016's standing wake.
    #[test]
    fn a_parentless_session_keeps_both_shapes_however_narrow_its_policy() {
        let narrow = leaf_policy();
        assert!(reject_leaf_control_plane(&narrow, false, Some("database::execute")).is_ok());
        assert!(reject_leaf_control_plane(&narrow, false, None).is_ok());
        // ...and its standing wake keeps the open end it asked for.
        assert_eq!(
            leaf_wake_deadline(&narrow, false, None, 1_000, 1_800_000),
            None
        );
    }

    // The half it does not: durable machinery pointed at someone else's
    // function, which it could not remove afterwards.
    #[test]
    fn a_leaf_may_not_bind_a_mechanical_call() {
        let err = reject_leaf_control_plane(&leaf_policy(), true, Some("database::execute"))
            .expect_err("a mechanical call is control plane");
        let text = format!("{err:?}");
        assert!(text.contains("database::execute"), "{text}");
        assert!(text.contains("Omit `target`"), "{text}");
        // A root (or an explicit orchestrator child) keeps both shapes.
        assert!(reject_leaf_control_plane(
            &policy_allowing(&["*"]),
            true,
            Some("database::execute")
        )
        .is_ok());
    }

    // Which statuses mean "nothing left to wait for". Getting this wrong in
    // either direction is silent: too narrow and the wake still strands, too
    // wide and a live operation's wake is refused. An unknown status must read
    // as in-flight (MOT-4766).
    #[test]
    fn only_the_three_terminal_operation_statuses_are_settled() {
        for status in ["succeeded", "failed", "cancelled"] {
            assert!(is_settled_operation(status), "{status} is terminal");
        }
        for status in ["running", "", "queued", "Succeeded", "partial"] {
            assert!(!is_settled_operation(status), "{status} is still in flight");
        }
    }

    /// The settled-operation preflight reads the operation from the Compose
    /// progress observer's `input.operation_id` filter, and only there.
    #[test]
    fn the_compose_preflight_reads_the_operation_filter() {
        let watch = req(json!({
            "function_id": COMPOSE_PROGRESS_FN,
            "filters": {
                "input.operation_id": { "$eq": "compose:op-1" },
                "input.terminal": { "$eq": true },
            }
        }));
        assert_eq!(compose_operation_id(&watch), Some("compose:op-1"));
        assert_eq!(
            compose_operation_id(&req(json!({ "function_id": COMPOSE_PROGRESS_FN }))),
            None
        );
        assert_eq!(
            compose_operation_id(&req(state(Some("compose:op-1"), None))),
            None
        );
    }

    // `once: true` caps deliveries, not time: the run-30 hang was two `once`
    // Compose wakes that never fired, on a child that cannot unobserve. Every
    // leaf wake gets a deadline whether it asked or not.
    #[test]
    fn a_leaf_wake_always_carries_a_deadline() {
        let leaf = leaf_policy();
        let now = 1_000_000;
        let cap = now + 1_800_000;

        assert_eq!(
            leaf_wake_deadline(&leaf, true, None, now, 1_800_000),
            Some(cap)
        );
        // A shorter request is honoured; a longer one is capped.
        assert_eq!(
            leaf_wake_deadline(&leaf, true, Some(now + 5_000), now, 1_800_000),
            Some(now + 5_000)
        );
        assert_eq!(
            leaf_wake_deadline(&leaf, true, Some(now + 9_000_000), now, 1_800_000),
            Some(cap)
        );
        // A root is left exactly as it asked, open-ended included.
        let root = policy_allowing(&["*"]);
        assert_eq!(leaf_wake_deadline(&root, true, None, now, 1_800_000), None);
        assert_eq!(
            leaf_wake_deadline(&root, true, Some(now + 9_000_000), now, 1_800_000),
            Some(now + 9_000_000)
        );
    }

    #[test]
    fn observe_control_tool_contract_has_compact_return_guidance() {
        let (description, _) = control_contract(OBSERVE_ID)
            .expect("harness::observe should expose a public control contract");
        assert_eq!(description, OBSERVE_TOOL_DESC);
        assert!(description.contains("Returns `{ observer_id, once }`"));
        assert!(description.contains("Call-target results are discarded"));

        let tool = native_control_tools(&policy_allowing(&[OBSERVE_ID]))
            .into_iter()
            .next()
            .expect("harness::observe should be present on the native control surface");
        assert_eq!(tool.name, OBSERVE_ID);
        assert_eq!(tool.description, OBSERVE_TOOL_DESC);
    }

    fn policy_allowing(allow: &[&str]) -> CompiledPolicy {
        CompiledPolicy::from(Some(&crate::types::turn::FunctionPolicy {
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: vec![],
            expose: Default::default(),
        }))
    }

    const AGENT_FORBIDDEN_CALLS: &[&str] = &[
        "harness::state::get",
        "harness::state::list",
        "harness::state::compare-and-set",
        "state::claim-namespace",
        "engine::observers::dispose",
        "provider::openai-codex::login::start",
        "provider::openai-codex::login::poll",
        "provider::openai-codex::login::cancel",
        "provider::openai-codex::auth::status",
        "provider::openai-codex::auth::logout",
        "provider-openai-codex::state::get",
        "provider-openai-codex::state::list",
        "provider-openai-codex::state::compare-and-set",
        "provider::openai-codex::login::future-operation",
        "provider::openai-codex::auth::future-operation",
        // Re-entering the pipeline for the caller's own session would wait on
        // the session lock the running step holds across tool execution.
        crate::functions::FUNCTION_CALL_ID,
        "provider-openai-codex::state::future-operation",
    ];

    pub(crate) fn disconnected_deps() -> Deps {
        let iii = std::sync::Arc::new(iii_sdk::IIIClient::new("ws://127.0.0.1:0"));
        Deps::new(
            iii.clone(),
            std::sync::Arc::new(tokio::sync::RwLock::new(std::sync::Arc::new(
                crate::config::WorkerConfig::default(),
            ))),
            crate::discovery::new_cell(),
            crate::skills::new_cell(),
            crate::events::TurnEvents::register(&iii),
            crate::hooks::HookRegistry::register(&iii),
        )
    }

    #[tokio::test]
    async fn operator_only_calls_are_denied_before_dispatch_even_with_allow_all() {
        let deps = disconnected_deps();
        let engine = deps.engine().await;
        let policy = policy_allowing(&["*"]);
        for &id in AGENT_FORBIDDEN_CALLS {
            assert!(
                policy.allows(id),
                "the policy must allow {id} for this regression"
            );
            // No connected engine: a denial must complete locally, without
            // waiting for an RPC timeout or relying on the target's own auth.
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                invoke(
                    &deps,
                    &engine,
                    &policy,
                    id,
                    &json!({}),
                    "owner",
                    false,
                    None,
                ),
            )
            .await
            .unwrap_or_else(|_| panic!("{id} escaped the agent guard"));
            assert!(result.is_error, "{id} must be denied");
            assert_eq!(result.details["error"], "policy_denied", "{id}");
            assert_eq!(result.details["function_id"], id);
        }
    }

    #[test]
    fn operator_only_calls_cannot_be_binding_targets_even_with_allow_all() {
        let policy = policy_allowing(&["*"]);
        for &id in AGENT_FORBIDDEN_CALLS {
            assert!(policy.allows(id));
            assert!(
                validate_call_target(id, &policy).is_err(),
                "{id} must be denied"
            );
        }
    }

    #[test]
    fn intercepted_observe_controls_cannot_be_binding_targets_even_with_allow_all() {
        let policy = policy_allowing(&["*"]);
        for id in [OBSERVE_ID, UNOBSERVE_ID] {
            assert!(policy.allows(id));
            assert!(
                validate_call_target(id, &policy).is_err(),
                "{id} must be intercepted"
            );
        }
    }

    #[tokio::test]
    async fn operator_only_observer_targets_and_conditions_are_denied() {
        let deps = disconnected_deps();
        let policy = policy_allowing(&["*"]);
        for id in AGENT_FORBIDDEN_CALLS
            .iter()
            .copied()
            .chain([OBSERVE_ID, UNOBSERVE_ID])
        {
            let r = req(json!({
                "function_id": STATE_ON_CHANGED_FN,
                "target": { "function_id": id }
            }));
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                resolve_target(&deps, &r, "owner", &policy),
            )
            .await
            .unwrap_or_else(|_| panic!("{id} reached the approval probe"));
            assert!(
                matches!(result, Err(HarnessError::InvalidRequest(_))),
                "{id}"
            );
            let conditions = vec![ConditionSpec {
                function_id: id.into(),
                config: None,
            }];
            let result = tokio::time::timeout(
                std::time::Duration::from_millis(100),
                authorize_conditions(&deps, &conditions, "owner", &policy),
            )
            .await
            .unwrap_or_else(|_| panic!("{id} reached the condition approval probe"));
            assert!(
                matches!(result, Err(HarnessError::InvalidRequest(_))),
                "{id}"
            );
        }
    }

    #[tokio::test]
    async fn operator_only_post_turn_validators_are_denied_by_the_observe_intercept() {
        let deps = disconnected_deps();
        let engine = deps.engine().await;
        let policy = policy_allowing(&["*"]);
        for id in AGENT_FORBIDDEN_CALLS
            .iter()
            .copied()
            .chain([OBSERVE_ID, UNOBSERVE_ID])
        {
            let result = invoke(
                &deps,
                &engine,
                &policy,
                OBSERVE_ID,
                &json!({
                    "function_id": crate::hooks::ON_POST_TURN_FN,
                    "target": { "function_id": id }
                }),
                "owner",
                false,
                None,
            )
            .await;
            assert!(result.is_error, "{id} must not become a validator");
            assert!(result.details["error"].as_str().unwrap().contains(id));
        }
    }

    /// A post-turn validator names its validator as the target; without one
    /// there is nothing to gate with.
    #[tokio::test]
    async fn a_post_turn_validator_needs_a_target() {
        let deps = disconnected_deps();
        let engine = deps.engine().await;
        let result = invoke(
            &deps,
            &engine,
            &policy_allowing(&["*"]),
            OBSERVE_ID,
            &json!({ "function_id": crate::hooks::ON_POST_TURN_FN }),
            "owner",
            false,
            None,
        )
        .await;
        assert!(result.is_error);
        assert!(result.details["error"]
            .as_str()
            .unwrap()
            .contains("target.function_id"));
    }

    /// A running step holds its session lock across tool execution and
    /// `harness::function::call` takes that lock: an agent reaching it for
    /// its own session would deadlock, even with an allow-all policy.
    #[tokio::test]
    async fn agents_cannot_reenter_the_function_call_pipeline() {
        let id = crate::functions::FUNCTION_CALL_ID;
        assert_eq!(id, "harness::function::call");
        assert!(agent_call_forbidden(id));
        let deps = disconnected_deps();
        let engine = deps.engine().await;
        let policy = policy_allowing(&["*"]);
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            invoke(&deps, &engine, &policy, id, &json!({}), "owner", true, None),
        )
        .await
        .expect("the denial completes locally without taking the session lock");
        assert!(result.is_error);
        assert_eq!(result.details["error"], "policy_denied");
    }

    #[test]
    fn ordinary_provider_and_state_binding_targets_remain_allowed() {
        let policy = policy_allowing(&["*"]);
        for id in [
            "provider::openai-codex::stream",
            "provider::openai-codex::models",
            "provider::openai::stream",
            "state::get",
            "state::list",
            "state::set",
            "state::compare-and-set",
            "other-worker::state::get",
            "provider-openai::state::get",
            "provider::openai-codex::login-info",
            "provider::openai-codex::authenticate",
            "provider-openai-codex::state-info",
            "engine::observers::list",
        ] {
            assert!(
                !agent_call_forbidden(id),
                "{id} must stay directly callable"
            );
            assert!(
                validate_call_target(id, &policy).is_ok(),
                "{id} must stay callable"
            );
        }
    }

    /// A delivered call runs with WORKER authority outside the turn loop, so
    /// the observer's own policy is the only thing standing between a
    /// narrowed session and every function on the bus.
    #[test]
    fn call_target_must_be_callable_by_the_observer() {
        let policy = policy_allowing(&["fp::*", "state::get"]);
        assert!(validate_call_target("fp::pipe", &policy).is_ok());
        assert!(validate_call_target("state::get", &policy).is_ok());

        let err = validate_call_target("shell::run", &policy).unwrap_err();
        assert!(
            err.contains("not permitted by this session's dispatch policy"),
            "unhelpful error: {err}"
        );
        assert!(validate_call_target("", &policy)
            .unwrap_err()
            .contains("must name the function"));
    }

    /// Prevents: a binding or post-turn validator on coder::scaffold-worker
    /// whose delivered payload sets `start: true`, running compose::add with
    /// the ide's authority for a session that may not call it (both delivery
    /// paths dispatch raw, outside the turn's scaffold_start_denial).
    #[test]
    fn a_scaffold_reaction_needs_compose_add() {
        let coder_only = policy_allowing(&["coder::*"]);
        let err = validate_call_target("coder::scaffold-worker", &coder_only).unwrap_err();
        assert!(err.contains("needs compose::add"), "unhelpful error: {err}");
        assert!(validate_call_target("coder::read-file", &coder_only).is_ok());
        let with_add = policy_allowing(&["coder::*", "compose::add"]);
        assert!(validate_call_target("coder::scaffold-worker", &with_add).is_ok());
    }

    /// Prevents: a starting scaffold whose compose::add the approval gate
    /// would hold running unseen inside the ide. Only `allow` (or no gate)
    /// lets the start through; otherwise the agent's own compose::add goes
    /// through the gate.
    #[test]
    fn a_scaffold_starts_only_when_its_compose_add_is_approved() {
        let default = json!({ "template": "worker-node-ade", "name": "orders" });
        let explicit = json!({ "template": "worker-node-ade", "name": "orders", "start": true });
        let held = || {
            Ok(Verdict::Withheld {
                verdict: "hold".into(),
                reason: "no rule matched".into(),
            })
        };

        assert!(matches!(
            start_after_approval(&default, Ok(Verdict::Allow)),
            call::StartGate::Pass
        ));
        let call::StartGate::FilesOnly(sent, note) = start_after_approval(&default, held()) else {
            panic!("a held compose::add must leave a defaulted start files-only");
        };
        assert_eq!(sent["start"], json!(false));
        assert!(
            note.contains("`hold`") && note.contains("Not started"),
            "{note}"
        );
        let call::StartGate::Deny(denied) = start_after_approval(&explicit, held()) else {
            panic!("an explicit start with a held compose::add must be refused");
        };
        assert_eq!(denied.details["error"]["code"], "scaffold_start_denied");
        // An undetermined verdict fails closed.
        assert!(matches!(
            start_after_approval(&default, Err("timeout".into())),
            call::StartGate::FilesOnly(..)
        ));
    }

    #[test]
    fn only_a_scaffold_without_start_false_starts() {
        let id = "coder::scaffold-worker";
        assert!(call::scaffold_starts(id, &json!({ "name": "a" })));
        assert!(call::scaffold_starts(
            id,
            &json!({ "name": "a", "start": true })
        ));
        assert!(!call::scaffold_starts(
            id,
            &json!({ "name": "a", "start": false })
        ));
        assert!(!call::scaffold_starts("coder::create-file", &json!({})));
    }

    /// A call re-entering the harness's control plane is refused even when the
    /// policy would allow it — a binding can wake its owner or call a plain
    /// function, never start an agent. `harness::spawn` lands here too, with
    /// the direct-spawn migration named in the error.
    #[test]
    fn harness_internal_targets_are_never_call_targets() {
        let wide_open = policy_allowing(&["*"]);
        for internal in [
            "harness::spawn",
            "harness::send",
            "harness::function::call",
            "harness::turn",
        ] {
            let err = validate_call_target(internal, &wide_open).unwrap_err();
            assert!(
                err.contains("not a binding target"),
                "{internal} must be refused: {err}"
            );
        }
        let spawn_err = validate_call_target("harness::spawn", &wide_open).unwrap_err();
        assert!(
            spawn_err.contains("Spawn children directly"),
            "the migration must be named: {spawn_err}"
        );
    }

    /// Only a genuinely absent approval worker may fail open: with no gate,
    /// a direct call needs no approval either. A transport or state failure
    /// must fail closed, or an outage silently becomes a permission grant.
    #[test]
    fn only_an_absent_approval_worker_reads_as_open() {
        for absent in [
            "function_not_found: approval::evaluate",
            "engine: no worker registered for approval::evaluate",
            "call failed: unregistered function",
            "approval::evaluate not found",
        ] {
            assert!(is_function_absent(absent), "must read as absent: {absent}");
        }
        for real_failure in [
            "approval/state_unavailable: state::get timed out",
            "timeout waiting for approval::evaluate",
            "connection reset",
            "approval/invalid_payload: session_id must be non-empty",
        ] {
            assert!(
                !is_function_absent(real_failure),
                "must fail closed: {real_failure}"
            );
        }
    }

    #[test]
    fn a_call_target_rejects_sub_agent_keys_and_bad_pointers() {
        // A `task` here means the caller wanted a spawn binding; refusing it
        // beats silently wiring a call that fires with an empty template.
        let err = serde_json::from_value::<ObserveRequest>(json!({
            "function_id": STATE_ON_CHANGED_FN,
            "target": { "function_id": "state::set", "task": "do it" }
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("task"), "{err}");
        // A pointer that is not a pointer is a per-event no-op at delivery.
        assert!(validate_event_into(Some("value")).is_err());
        // The valid shapes pass, including the empty-pointer whole-payload form.
        assert!(validate_event_into(Some("/args/event")).is_ok());
        assert!(validate_event_into(Some("")).is_ok());
        assert!(validate_event_into(None).is_ok());

        let explicit: BindingTarget = serde_json::from_value(json!({
            "function_id": "state::set",
            "event_into": "not/a/pointer",
        }))
        .unwrap();
        assert!(validate_event_into(explicit.event_into.as_deref()).is_err());
    }

    #[test]
    fn event_action_must_be_non_empty_text() {
        assert!(validate_event_action(None).is_ok());
        assert!(validate_event_action(Some("new Explorer message received")).is_ok());
        assert!(validate_event_action(Some("  ")).is_err());
        assert!(serde_json::from_value::<ObserveRequest>(json!({
            "function_id": STATE_ON_CHANGED_FN,
            "action": 7
        }))
        .is_err());
    }

    #[test]
    fn the_dedup_key_covers_the_target_too() {
        // Two observes that differ ONLY in their target must not dedup onto
        // each other — the second would silently return the first binding's
        // id.
        let mut a = req(state(Some("run"), None));
        let mut b = req(state(Some("run"), None));
        a.target = Some(crate::bindings::BindingTarget::new("state::set"));
        b.target = Some(crate::bindings::BindingTarget::new("state::delete"));
        assert_ne!(
            registration_dedup_key(&a, true),
            registration_dedup_key(&b, true)
        );
        // Same request twice still matches itself.
        assert_eq!(
            registration_dedup_key(&a, true),
            registration_dedup_key(&a, true)
        );
        // Args and filters are part of what is observed.
        let other_scope = req(state(Some("other"), None));
        assert_ne!(
            registration_dedup_key(&a, true),
            registration_dedup_key(&other_scope, true)
        );
    }

    #[test]
    fn the_dedup_key_uses_raw_timers_and_normalized_lifecycle() {
        let r = req(json!({
            "function_id": crate::timer::ON_TIMER_FN,
            "args": { "in_ms": 250 },
            "lifecycle": { "once": true, "max_fires": 1, "expires_in_ms": 1234 },
        }));
        let key = registration_dedup_key(&r, effective_once(&r));
        assert_eq!(key["args"], json!({ "in_ms": 250 }));
        // Relative on purpose: a model retry re-sends the same relative
        // request, so the key matches even though each attempt would resolve
        // a different absolute instant.
        assert_eq!(
            key["lifecycle"],
            json!({ "once": true, "max_fires": 1, "expires_in_ms": 1234 })
        );

        let mut different = r.clone();
        different.lifecycle.as_mut().unwrap().expires_in_ms = Some(5678);
        assert_ne!(
            registration_dedup_key(&r, true),
            registration_dedup_key(&different, true)
        );
    }

    #[test]
    fn approval_probe_carries_the_arguments_it_authorizes() {
        let arguments = json!({ "command": "rm", "path": "/tmp/x" });
        let payload = approval_probe_payload("s_1", "shell::exec", &arguments);
        assert_eq!(payload["session_id"], "s_1");
        assert_eq!(payload["function_id"], "shell::exec");
        assert_eq!(payload["arguments"], arguments);
    }

    #[test]
    fn stringified_args_and_filters_are_parsed_back_into_objects() {
        // deepwork leads sent objects as JSON strings; the worker refused
        // them and the wake never fired. Reconciliation repairs both.
        let (_, schema) = control_contract(OBSERVE_ID).unwrap();
        let reconciled = crate::reconcile::coerce(
            &schema,
            &json!({
                "function_id": STATE_ON_CHANGED_FN,
                "filters": "{\"input.scope\":{\"$eq\":\"run-r1\"}}",
                "args": "{\"expression\":\"0 * * * * *\"}",
            }),
        )
        .expect("stringified objects are repairable");
        assert_eq!(
            reconciled.arguments["filters"],
            json!({ "input.scope": { "$eq": "run-r1" } })
        );
        assert_eq!(
            reconciled.arguments["args"],
            json!({ "expression": "0 * * * * *" })
        );
    }

    #[test]
    fn only_an_object_or_absent_args_and_filters_are_accepted() {
        for field in ["args", "filters"] {
            assert!(validate_object(field, &json!({ "scope": "run" })).is_ok());
            assert!(validate_object(field, &Value::Null).is_ok());
            for bad in [json!("{\"scope\":\"run\"}"), json!(["run"]), json!(7)] {
                let Err(HarnessError::InvalidRequest(msg)) = validate_object(field, &bad) else {
                    panic!("{bad} must be refused");
                };
                assert!(msg.contains("must be a JSON object"), "{msg}");
                assert!(msg.contains(field), "{msg}");
            }
        }
    }

    /// The registered functions only answer calls that bypassed the turn
    /// loop, which have no session to observe for.
    #[tokio::test]
    async fn direct_calls_outside_a_turn_are_refused_with_the_way_out() {
        let err = direct_handle::<ObserveResponse>(OBSERVE_ID)
            .await
            .unwrap_err()
            .to_string();
        assert!(err.contains("runs only from a turn"), "{err}");
        assert!(err.contains("iii.observe"), "{err}");
        assert!(err.contains("harness::observers::dispose"), "{err}");
    }
}

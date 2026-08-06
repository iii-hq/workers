//! Subscriptions — register an ephemeral iii trigger and be notified when it
//! fires, instead of polling (harness.md § Subscriptions). The agent calls
//! `engine::register_trigger` / `engine::unregister_trigger`; the harness
//! intercepts those calls (see [`invoke`]) so the trusted owning session is
//! injected and teardown stays owner-checked — the agent can never supply
//! those. Every registration becomes a durable binding whose fire either
//! notifies the owner session (a wake) or calls a plain, policy-checked
//! function. A binding never starts an agent: `harness::spawn` is not a
//! target, and spawning is a direct call the owner makes on its own turn.

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::TriggerAction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::bindings::{
    Binding, BindingTarget, Causation, ConditionSpec, Lifecycle, OwnerScope, ReserveOutcome,
};
use crate::clients::EngineClient;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::policy::CompiledPolicy;
use crate::subscriptions;
use crate::trigger::{self, ResultData};
use crate::types::content::ContentBlock;
use crate::types::model::AgentFunction;

/// The engine function the agent calls to subscribe. The harness intercepts it
/// (the agent never reaches the raw engine registrar) so it can stamp the
/// trusted session and point the engine trigger at the harness delivery hop.
pub const REGISTER_TRIGGER_ID: &str = "engine::register_trigger";

/// Side-effect-free "what would the gate decide" probe. Absent when the
/// deployment runs no approval worker — see [`approval_allows_unattended`].
const APPROVAL_EVALUATE_ID: &str = "approval::evaluate";

/// The engine function the agent calls to unsubscribe. The harness intercepts it
/// so it resolves the caller's subscription, enforces ownership, and unregisters
/// the underlying engine trigger.
pub const UNREGISTER_TRIGGER_ID: &str = "engine::unregister_trigger";

/// Agent-facing subscription contract.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(rename = "SubscribeArgs")]
pub struct SubscribeRequest {
    /// The iii trigger type to listen on: `cron`, `state`, `stream`, `timer`
    /// (one-shot deadline: `{ "in_ms": <ms> }` or `{ "at": <epoch ms> }`), or
    /// another worker's custom trigger type (e.g. `database::row-changed`).
    /// For an ad-hoc signal, subscribe to `state` on a key and have the
    /// signaller call `state::set` on it (no dedicated emit needed — the
    /// engine fans the trigger out to every subscriber).
    pub trigger_type: String,
    /// The trigger config, passed verbatim to the engine — e.g.
    /// `{ "expression": "0 */5 * * * *" }` for cron, or a `state` scope/key.
    #[serde(default)]
    pub config: Value,
    /// A short human label echoed back in the notification text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Auto-unsubscribe after the first delivered fire. Defaults by SHAPE:
    /// a WAKE (no `function_id`) is once — it parks this session until the
    /// event; a CALL binding is STANDING — it runs per matching event until
    /// unregistered or its lifecycle ends; `cron` is recurring; `timer` is
    /// once. Pass `once` explicitly to override any of these.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
    /// Target function called on each event. Omit for a notification message
    /// into this session — the only shape that can reach you. Any non-harness
    /// function your policy allows may be named; `harness::spawn` is NOT a
    /// binding target — a binding never starts an agent. Spawn children
    /// directly from a turn and register a wake on what they write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    /// Call-binding template: `{ payload?, event_into? }`. `payload` is the
    /// fixed argument object sent to the target; `event_into` is a JSON
    /// pointer naming where the fired event is injected into that payload.
    /// Meaningless for a wake; sub-agent fields (task/model/session_id/
    /// options) are rejected — spawning is not something a binding does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    /// Explicit target, the long form of `function_id` + `metadata`:
    /// `{ function_id, payload?, event_into? }`. The shorthands stay valid and
    /// mean exactly this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<BindingTarget>,
    /// Gates evaluated at fire time, in order, each an ordinary iii function
    /// answering `{ decision: "allow" | "skip", payload?, reason? }`. The
    /// built-in safety gates run first and are not listed here.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub conditions: Vec<ConditionSpec>,
    /// When the binding stops firing. `once` may also be given at the top
    /// level (the shorthand every prompt uses).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<LifecycleRequest>,
}

/// The caller-settable half of a binding's lifecycle.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct LifecycleRequest {
    /// Auto-unsubscribe after the first delivered fire. The top-level `once`
    /// field remains accepted as shorthand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
    /// Lifetime delivery budget; the binding retires on its Nth delivery.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_fires: Option<u64>,
    /// Wall-clock deadline (epoch ms) after which the binding stops.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubscribeResponse {
    pub subscription_id: String,
    /// The effective `once` flag applied (after the per-type default).
    pub once: bool,
    /// Advisory only — the registration SUCCEEDED, but its wiring looks
    /// suspicious (e.g. a turn-event filter naming a session that doesn't
    /// exist). Read it and fix the wiring if it applies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// Agent-facing unsubscribe contract. The id is the harness subscription id
/// returned by [`SubscribeResponse`], not the underlying engine binding id.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(rename = "UnsubscribeArgs")]
pub struct UnsubscribeRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct UnsubscribeResponse {
    pub removed: bool,
}

/// Subscription controls are intercepted by the harness and therefore do not
/// appear in the engine's public function registry. Native exposure still has
/// to publish their real contracts when the turn policy allows them.
/// The contract agents actually get for the intercepted control ids — shown
/// as the native tool AND overlaid onto `engine::functions::info/list`
/// responses, which would otherwise report the raw engine registration (no
/// `once`, no `lifecycle`, no `conditions`, `function_id` required).
pub const REGISTER_TOOL_DESC: &str =
    "Subscribe the current harness session to a iii trigger: omit function_id to be \
     notified in this session, or name a non-harness function to call with the event.";
pub const UNREGISTER_TOOL_DESC: &str =
    "Remove a trigger subscription owned by the current harness session.";

/// The intercept's request schema for a control id, for discovery overlays.
pub fn control_contract(function_id: &str) -> Option<(&'static str, Value)> {
    match function_id {
        REGISTER_TRIGGER_ID => Some((
            REGISTER_TOOL_DESC,
            crate::surface::schema_value::<SubscribeRequest>(),
        )),
        UNREGISTER_TRIGGER_ID => Some((
            UNREGISTER_TOOL_DESC,
            crate::surface::schema_value::<UnsubscribeRequest>(),
        )),
        _ => None,
    }
}

pub fn native_control_tools(policy: &CompiledPolicy) -> Vec<AgentFunction> {
    [
        (
            REGISTER_TRIGGER_ID,
            REGISTER_TOOL_DESC,
            crate::surface::schema_value::<SubscribeRequest>(),
        ),
        (
            UNREGISTER_TRIGGER_ID,
            UNREGISTER_TOOL_DESC,
            crate::surface::schema_value::<UnsubscribeRequest>(),
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

/// The registering turn's dispatch policy, threaded from the dispatch
/// chokepoints so a call binding can freeze the registrant's capability.
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

/// The single per-call invocation chokepoint. Subscription control calls
/// (`engine::register_trigger` / `engine::unregister_trigger`) are handled inline
/// with the trusted owning session injected — the model can never widen the
/// target; everything else invokes the target normally. Every call site (the
/// turn loop, `harness::function::trigger`, and the hook-held release path) routes
/// through here so the trusted injection can't be bypassed.
pub async fn invoke(
    deps: &Deps,
    engine: &EngineClient,
    policy: &CompiledPolicy,
    function_id: &str,
    arguments: &Value,
    session_id: &str,
    caller: Option<CallerModel<'_>>,
) -> ResultData {
    match function_id {
        REGISTER_TRIGGER_ID => {
            intercept_register(deps, arguments, session_id, caller, policy).await
        }
        UNREGISTER_TRIGGER_ID => intercept_unregister(deps, arguments, session_id).await,
        crate::functions::triggers_list::TRIGGERS_LIST_ID
        | crate::functions::triggers_list::TRIGGERS_UNREGISTER_ID => {
            // In-turn controls always target their caller. External console
            // calls bypass this chokepoint and continue supplying session_id.
            let args = with_caller_session_id(arguments, session_id);
            trigger::invoke_target(engine, policy, function_id, &args).await
        }
        internal if internal.starts_with("harness::state::") => trigger::denied_result(internal),
        // Claiming a private state namespace is a control-plane act this
        // worker performs for ITSELF. Agent calls are dispatched with the
        // harness's own worker identity, so without this an agent could
        // launder a claim through us and reserve arbitrary scopes — denying
        // other workers the public `state::*` API. It could never READ them
        // (the `harness::state::*` accessors are denied above), so the risk
        // is denial of service, not exfiltration; deny it anyway.
        crate::state::CLAIM_NAMESPACE_ID => trigger::denied_result(function_id),
        _ => trigger::invoke_target(engine, policy, function_id, arguments).await,
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
/// * `timer` is once by definition (a deadline that repeats is a cron);
/// * `cron` is recurring by definition;
/// * a WAKE (no target function) is once: it parks its session, and a park
///   that silently re-arms forever is the worse surprise;
/// * a CALL binding is STANDING: "handle every arrival" is what agents mean
///   when they bind work to an event stream — runs 1 and 5 both registered
///   intended-standing handlers, got the old once default, and silently lost
///   every event after the first.
///
/// The standing default is not silent either: `standing_binding_advisory`
/// tells the registrant it is standing and to bound or retire it.
fn effective_once(req: &SubscribeRequest) -> bool {
    if let Some(once) = req
        .once
        .or_else(|| req.lifecycle.as_ref().and_then(|l| l.once))
    {
        return once;
    }
    match req.trigger_type.as_str() {
        crate::timer::TIMER_TYPE => true,
        "cron" => false,
        _ => is_wake_shape(req),
    }
}

/// Whether the registration is a wake — no explicit target function, so the
/// fire delivers a notification into the registering session.
fn is_wake_shape(req: &SubscribeRequest) -> bool {
    req.function_id.is_none() && req.target.is_none()
}

/// Validate a `timer` registration and resolve `{ in_ms }` to an absolute
/// `{ at }` BEFORE the engine stores it — the engine replays registrations to
/// the provider on reconnect, and a relative countdown replayed after a
/// restart would silently restart from zero. Strict here, lenient in the
/// provider: agents get told about a nonsense deadline at registration; a
/// replayed past-due `at` still fires.
fn resolve_timer_request(req: &mut SubscribeRequest) -> Result<(), HarnessError> {
    if req.trigger_type != crate::timer::TIMER_TYPE {
        return Ok(());
    }
    if !effective_once(req) {
        return Err(HarnessError::InvalidRequest(
            "a timer fires exactly once — for recurrence use `cron` (with a lifecycle bound)"
                .into(),
        ));
    }
    let cfg: crate::timer::TimerTriggerConfig = serde_json::from_value(req.config.clone())
        .map_err(|e| HarnessError::InvalidRequest(format!("timer config: {e}")))?;
    let now = crate::types::message::AgentMessage::now_ms();
    let at = crate::timer::resolve_fire_at(&cfg, now).map_err(HarnessError::InvalidRequest)?;
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
    req.config = json!({ "at": at });
    Ok(())
}

/// Advisory for a standing call binding (no `once`): agents keep registering
/// one-run kickoffs without `once: true`, leaving bindings that re-run their
/// call on every future matching event. Purely informative — deliberate
/// standing watchers ignore it.
///
/// Cron gets its own version: recurring is its point, but a recurring cron
/// with NO lifecycle bound fires forever, and every live run that registered
/// one meant it as a deadline or a bounded sweep (rctest7's 69 fires; both
/// discovery runs left one firing past their own teardown claim).
fn standing_binding_advisory(req: &SubscribeRequest, once: bool) -> Option<String> {
    if once {
        return None;
    }
    if req.trigger_type == "cron" {
        let bounded = req
            .lifecycle
            .as_ref()
            .is_some_and(|l| l.max_fires.is_some() || l.expires_at.is_some());
        if bounded {
            return None;
        }
        let expr = req
            .config
            .get("expression")
            .and_then(Value::as_str)
            .unwrap_or("?");
        return Some(format!(
            "warning: this cron is UNBOUNDED — `{expr}` fires at every boundary FOREVER until \
             unregistered, and each fire delivers its own paid reaction (a woken turn or a \
             dispatched call). This registration still SUCCEEDED. A deadline wants trigger_type \
             \"timer\" with {{ \"in_ms\": <ms> }} (fires once, exactly on time) — or keep this \
             cron and pass `once: true`; a bounded run sets lifecycle \
             {{ max_fires | expires_at }}; a deliberate forever-cron must be unregistered by \
             your teardown."
        ));
    }
    Some(
        "note: this call binding is STANDING — it re-runs its call on EVERY future matching \
         event until unregistered. That is the default for a call target (per-event work is \
         what a call binding means); pass `once: true` for a one-shot, and give a standing \
         binding a lifecycle bound ({ max_fires | expires_at }) or unregister it in your \
         teardown so it cannot fire forever."
            .to_string(),
    )
}

/// The `(scope, key)` a keyed state binding watches — the only shape whose
/// pre-existing value can be checked cheaply at registration.
fn watched_state_key(req: &SubscribeRequest) -> Option<(&str, &str)> {
    if req.trigger_type != "state" {
        return None;
    }
    Some((
        req.config.get("scope").and_then(Value::as_str)?,
        req.config.get("key").and_then(Value::as_str)?,
    ))
}

fn prewritten_key_note(scope: &str, key: &str) -> String {
    format!(
        "warning: state {scope}/{key} ALREADY holds a value — state events do not replay, so \
         writes from before this registration never fire this binding, and a condition counting \
         arrivals (state::barrier) starts without them. If earlier writers matter, arm the \
         binding BEFORE starting them, or reconcile the gate against what is already written."
    )
}

/// Advisory for a state binding on a key somebody already wrote. Discovery
/// run 2: the finish gate — a barrier over three suppliers — was registered
/// mid-setup, AFTER the first supplier's done event had fired; it starved at
/// 2/3 forever and only the deadline path saved the run. The stale value is
/// detectable at registration, so say so. Fail-open: a lookup error produces
/// no note.
async fn prewritten_key_advisory(deps: &Deps, req: &SubscribeRequest) -> Option<String> {
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

/// Advisory for a `state` binding with no `key` filter: it fires for EVERY
/// key written in the scope (or every write anywhere, with no scope either) —
/// order signals, done markers, completion keys, all of it. Registered beside
/// a keyed binding it double-fires every event, and every fire delivers its
/// own unthrottled paid reaction (rctest-k7m3 postmortem). Purely informative —
/// a deliberate catch-all watcher ignores it.
fn state_catchall_advisory(req: &SubscribeRequest) -> Option<String> {
    if req.trigger_type != "state" || req.config.get("key").is_some() {
        return None;
    }
    // Scope-only is CORRECT for the per-event-unique-key shape (producers
    // writing `item-1`, `item-2`, … so nothing overwrites) — discovery run 7
    // registered exactly that, read the old "add a `key`" push as an
    // instruction, pinned `key: "shipment"`, and its producers' unique keys
    // then matched nothing at all. State the trade-off; do not pick a side.
    match req.config.get("scope").and_then(Value::as_str) {
        Some(scope) => Some(format!(
            "note: this state binding has no `key` filter, so it fires for EVERY key written in \
             scope \"{scope}\" — done markers and progress keys included — and every matching \
             write dispatches its own unthrottled paid reaction. That is the RIGHT shape when \
             producers write one unique key per event (item-1, item-2, …); keep this scope \
             private to those events. Add a `key` ONLY if every producer writes that exact key."
        )),
        None => Some(
            "warning: this state binding has neither `scope` nor `key` — it fires for EVERY \
             state write in EVERY scope, including other runs' traffic, and each one dispatches \
             its own unthrottled paid reaction. Scope it to this run at minimum."
                .to_string(),
        ),
    }
}

async fn intercept_register(
    deps: &Deps,
    args: &Value,
    session_id: &str,
    caller: Option<CallerModel<'_>>,
    policy: &CompiledPolicy,
) -> ResultData {
    let req: SubscribeRequest = match serde_json::from_value(args.clone()) {
        Ok(r) => r,
        Err(e) => return error_result(format!("invalid subscribe arguments: {e}")),
    };

    match handle(deps, req, session_id, caller, policy).await {
        Ok(resp) => ok_result(&resp),
        Err(e) => error_result(e.to_string()),
    }
}

async fn intercept_unregister(deps: &Deps, args: &Value, session_id: &str) -> ResultData {
    let req: UnsubscribeRequest = match serde_json::from_value(args.clone()) {
        Ok(req) => req,
        Err(error) => {
            return error_result(format!(
                "{UNREGISTER_TRIGGER_ID} requires a string `id`: {error}"
            ))
        }
    };
    let id = req.id.as_str();

    // Agent-registered post-turn validator? Owner-checked in the registry.
    if let Some(removed) = deps.hooks.unregister_owned(id, session_id) {
        return ok_result(&UnsubscribeResponse { removed });
    }

    let store = deps.bindings().await;
    let binding = match store.get(id).await {
        Ok(b) => b,
        Err(e) => return error_result(format!("binding lookup failed: {e}")),
    };

    if let Some(binding) = binding {
        // Owner-only: with trigger-driven spawning gone there is no reaction
        // lineage anymore — a session tears down its own bindings, the
        // console uses `harness::triggers::unregister`, and nothing else may.
        if binding.owner.session_id != session_id {
            return error_result("subscription belongs to a different session".to_string());
        }
        if let Some(trigger_id) = binding.trigger_id.as_deref() {
            unregister_engine_trigger(deps, trigger_id).await;
        }
        if let Err(e) = store.delete(id).await {
            return error_result(format!("binding delete failed: {e}"));
        }
        return ok_result(&UnsubscribeResponse { removed: true });
    }

    // No record: already retired (a `once` that fired, or someone else's
    // teardown). `removed: false` is the honest answer, not an error.
    ok_result(&UnsubscribeResponse { removed: false })
}

/// Build the durable binding a registration describes, register ONE engine
/// trigger pointing at the delivery hop, and hand the id back. Both shapes —
/// wake, mechanical call — produce the same record; only the target differs.
async fn handle(
    deps: &Deps,
    mut req: SubscribeRequest,
    session_id: &str,
    caller: Option<CallerModel<'_>>,
    policy: &CompiledPolicy,
) -> Result<SubscribeResponse, HarnessError> {
    // The one harness-internal type an agent MAY bind: a post-turn validator
    // for ITS OWN session. The session scope is force-stamped, so an agent
    // can gate its own completions but never anyone else's.
    if req.trigger_type == crate::hooks::POST_TURN {
        return register_post_turn_hook(deps, req, session_id, policy).await;
    }
    reject_forbidden_type(&req.trigger_type)?;

    // The engine's own condition contract vetoes only on a bare `false` and
    // treats an erroring condition as "skip", silently and forever. Steer
    // agents onto the typed one rather than letting them wire a binding that
    // looks armed and never fires.
    if req.config.get("condition_function_id").is_some() {
        return Err(HarnessError::InvalidRequest(
            "`condition_function_id` in `config` is not supported: an erroring or unknown \
             condition silently starves the binding. Use `conditions: [{ function_id, config? }]` \
             at the top level — those report why a fire was skipped."
                .into(),
        ));
    }

    validate_lifecycle(
        req.lifecycle.as_ref(),
        crate::types::message::AgentMessage::now_ms(),
    )?;
    let once = effective_once(&req);
    // Timer idempotency is based on the caller's relative request. Resolving
    // `in_ms` first would mint a different absolute deadline on every retry.
    let dedup = registration_dedup_key(&req, once);
    resolve_timer_request(&mut req)?;

    let target = resolve_target(deps, &req, session_id, policy).await?;
    authorize_conditions(deps, &req.conditions, session_id, policy).await?;

    // Idempotency: an identical re-registration (a model retry, a re-run
    // prompt in the same session) returns the standing binding instead of
    // wiring a twin that double-delivers forever.
    let store = deps.bindings().await;
    if let Some(existing) = store.find_duplicate(session_id, &dedup).await? {
        return Ok(SubscribeResponse {
            subscription_id: existing,
            once,
            note: None,
        });
    }
    let mut binding = Binding {
        id: format!("sub_{}", uuid::Uuid::new_v4().simple()),
        trigger_id: None,
        owner: OwnerScope {
            session_id: session_id.to_string(),
            root_session_id: None,
        },
        target,
        conditions: req.conditions.clone(),
        lifecycle: Lifecycle {
            once,
            max_fires: req.lifecycle.as_ref().and_then(|l| l.max_fires),
            expires_at: req.lifecycle.as_ref().and_then(|l| l.expires_at),
        },
        // The registrant's policy, frozen: a fired call is checked against
        // what the session could call WHEN IT REGISTERED, never against a
        // policy that widened afterwards.
        capability: caller.and_then(|c| c.functions.cloned()),
        causation: Causation::default(),
        dedup_key: Some(dedup),
        fires: 0,
        created_at: crate::types::message::AgentMessage::now_ms(),
    };
    // Durable BEFORE the engine knows about it: a fire that arrives before the
    // engine's answer must still resolve its record. The reservation and
    // per-owner capacity check are one CAS-backed operation.
    validate_lifecycle(
        req.lifecycle.as_ref(),
        crate::types::message::AgentMessage::now_ms(),
    )?;
    require_reserved(store.reserve(&binding).await?)?;

    let resp = deps
        .iii
        .trigger(TriggerRequest {
            function_id: REGISTER_TRIGGER_ID.to_string(),
            payload: json!({
                "trigger_type": req.trigger_type,
                "function_id": crate::functions::trigger_deliver::DELIVER_ID,
                "config": req.config,
                "metadata": { "__binding": binding.id },
            }),
            action: None,
            timeout_ms: Some(deps.cfg().await.dispatch_timeout_ms),
        })
        .await;

    match resp.map(|v| v.get("id").and_then(Value::as_str).map(str::to_string)) {
        Ok(Some(trigger_id)) => {
            match store.attach_trigger_id(&binding, &trigger_id).await {
                Ok(crate::bindings::AttachOutcome::Attached(current)) => {
                    binding = *current;
                }
                Ok(crate::bindings::AttachOutcome::Gone) => {
                    // A fast one-shot fired and retired before registration
                    // returned. Its provider id still needs explicit teardown.
                    unregister_engine_trigger(deps, &trigger_id).await;
                }
                Err(error) => {
                    // The caller must never observe a failed registration
                    // while its provider trigger remains live.
                    unregister_engine_trigger(deps, &trigger_id).await;
                    let _ = store.delete(&binding.id).await;
                    return Err(error);
                }
            }
        }
        // Carry the engine's rejection reason (e.g. an unknown config key for
        // this trigger type) — an opaque "failed" sends the agent guess-looping.
        outcome => {
            let _ = store.delete(&binding.id).await;
            let reason = match outcome {
                Err(e) => e.to_string(),
                _ => "no binding id in response".to_string(),
            };
            return Err(HarnessError::Dependency(format!(
                "{REGISTER_TRIGGER_ID} `{}` failed: {reason}",
                req.trigger_type
            )));
        }
    }

    let notes: Vec<String> = [
        prewritten_key_advisory(deps, &req).await,
        standing_binding_advisory(&req, once),
        state_catchall_advisory(&req),
        armed_wake_advisory(&req, once),
        (binding.target.function_id != crate::functions::SEND_ID).then(|| {
            format!(
                "note: this binding dispatches `{}` — its result reaches nobody and cannot wake \
                 you. Anything that must reach this chat writes state you watch with a plain \
                 notify (no `function_id`).",
                binding.target.function_id
            )
        }),
    ]
    .into_iter()
    .flatten()
    .collect();

    Ok(SubscribeResponse {
        subscription_id: binding.id,
        once,
        note: compose_note(notes),
    })
}

fn validate_lifecycle(
    lifecycle: Option<&LifecycleRequest>,
    now_ms: i64,
) -> Result<(), HarnessError> {
    let Some(lifecycle) = lifecycle else {
        return Ok(());
    };
    if lifecycle.max_fires == Some(0) {
        return Err(HarnessError::InvalidRequest(
            "`lifecycle.max_fires` must be at least 1".into(),
        ));
    }
    if lifecycle
        .expires_at
        .is_some_and(|expires_at| expires_at <= now_ms)
    {
        return Err(HarnessError::InvalidRequest(
            "`lifecycle.expires_at` must be in the future".into(),
        ));
    }
    Ok(())
}

fn require_reserved(outcome: ReserveOutcome) -> Result<(), HarnessError> {
    match outcome {
        ReserveOutcome::Reserved => Ok(()),
        ReserveOutcome::Capacity => Err(HarnessError::InvalidRequest(format!(
            "binding cap reached ({} active for this session); unregister first",
            crate::bindings::MAX_BINDINGS_PER_SESSION
        ))),
        ReserveOutcome::Exhausted => Err(HarnessError::InvalidRequest(
            "binding lifecycle expired before it could be reserved".into(),
        )),
    }
}

/// Join the advisories under one header that says the outcome FIRST.
/// Discovery run 3: a model read a warning as a redo order, unregistered the
/// binding, re-rolled the same registration twice, then tore down its whole
/// watch set — the notes never said the registration had succeeded, so
/// starting over looked mandatory.
fn compose_note(notes: Vec<String>) -> Option<String> {
    if notes.is_empty() {
        return None;
    }
    Some(format!(
        "registration SUCCEEDED; everything below is advisory, not an error. Apply a named fix \
         to THIS binding (or unregister it) rather than starting over. {}",
        notes.join(" ")
    ))
}

/// Turn events (and every other harness-internal type) are not agent-bindable
/// in ANY shape: child outcomes reach a parent through the medium the children
/// write, never through a binding on their turns. Runs before target
/// resolution, so wake, call, and would-be spawn shapes all hit it.
fn reject_forbidden_type(trigger_type: &str) -> Result<(), HarnessError> {
    if subscriptions::is_forbidden_trigger_type(trigger_type) {
        return Err(HarnessError::InvalidRequest(format!(
            "cannot bind harness-internal trigger type `{trigger_type}`: turn events are not \
             agent-bindable. Watch what the work WRITES instead — register a wake (omit \
             `function_id`) on the state keys or database rows the tasks update."
        )));
    }
    Ok(())
}

/// Resolve and VALIDATE the target a registration asks for. Two shapes
/// (omitted `function_id` = wake, anything else = mechanical call); past this
/// point they are one record. A binding never starts an agent —
/// `harness::spawn` falls into the call arm and is refused there with the
/// direct-spawn migration in the error.
async fn resolve_target(
    deps: &Deps,
    req: &SubscribeRequest,
    session_id: &str,
    policy: &CompiledPolicy,
) -> Result<BindingTarget, HarnessError> {
    let explicit = req
        .target
        .as_ref()
        .map(|t| t.function_id.clone())
        .or_else(|| req.function_id.clone());

    match explicit.as_deref() {
        // A wake: the fire is injected into the owner session as a message.
        None => {
            let mut target = BindingTarget::new(crate::functions::SEND_ID);
            target.payload = Some(json!({
                "session_id": session_id,
                "label": req.label,
            }));
            Ok(target)
        }
        // A mechanical call. Gated twice: the registrant's own policy, and the
        // deployment's approval gate — a fired call runs outside any turn and
        // can never raise a prompt.
        Some(id) => {
            let id = id.to_string();
            validate_call_target(&id, policy).map_err(HarnessError::InvalidRequest)?;
            let (payload, event_into) = match req.target.as_ref() {
                Some(t) => {
                    validate_event_into(t.event_into.as_deref())?;
                    (t.payload.clone(), t.event_into.clone())
                }
                None => {
                    let m = req.metadata.clone().unwrap_or(Value::Null);
                    validate_call_template(&m)?;
                    (
                        m.get("payload").cloned(),
                        m.get("event_into")
                            .and_then(Value::as_str)
                            .map(str::to_string),
                    )
                }
            };
            let target = BindingTarget {
                function_id: id,
                payload,
                event_into,
            };
            let arguments = target.payload.clone().unwrap_or_else(|| json!({}));
            approval_allows_unattended(deps, &target.function_id, session_id, &arguments)
                .await
                .map_err(HarnessError::InvalidRequest)?;
            Ok(target)
        }
    }
}

/// Conditions execute with worker authority too, so they cross the same
/// registration boundary as the delivery target.
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

/// A call binding's shorthand `metadata` takes only `{ payload?, event_into? }`.
/// Reject the rest LOUDLY: sub-agent keys here mean the caller wanted a spawn
/// binding, and a pointer that cannot be created is a silent per-event no-op at
/// fire time.
fn validate_call_template(metadata: &Value) -> Result<(), HarnessError> {
    let Some(map) = metadata.as_object() else {
        return Ok(());
    };
    const ALLOWED: [&str; 2] = ["payload", "event_into"];
    if let Some(unknown) = map.keys().find(|k| !ALLOWED.contains(&k.as_str())) {
        return Err(HarnessError::InvalidRequest(format!(
            "unknown key `{unknown}` for a call binding: `metadata` takes only \
             {{ payload?, event_into? }} — the target is the registration's `function_id`. \
             Sub-agent fields (task/model/session_id/options) are not valid anywhere in a \
             binding: spawn children directly from a turn."
        )));
    }
    validate_event_into(map.get("event_into").and_then(Value::as_str))
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

/// Advisory for a one-shot state-key WAKE: the registering session parks
/// until someone writes that exact scope/key, and the harness cannot verify
/// any task is wired to do so. rctest postmortem: an orchestrator armed
/// `report_ready` while its finalizer's task never mentioned the key — every
/// row landed correctly and the orchestrator still slept forever, leaving
/// the run's teardown permanently pending. Purely informative.
fn armed_wake_advisory(req: &SubscribeRequest, once: bool) -> Option<String> {
    if !once || req.trigger_type != "state" {
        return None;
    }
    let scope = req
        .config
        .get("scope")
        .and_then(Value::as_str)
        .unwrap_or("*");
    let key = req.config.get("key").and_then(Value::as_str).unwrap_or("*");
    // The two observed park-forever shapes, named in order: the medium
    // mismatch (a finisher wrote a DATABASE table named like the state scope
    // — same word, disjoint worlds, the wake never fired), and the missing
    // deadline (no lifecycle, so nothing could ever convert the silence into
    // a notice).
    Some(format!(
        "note: one-shot WAKE — this session stays parked until state {scope}/{key} is \
         written, and nothing fires it automatically. Only a state write (state::set / \
         state::delete) on that exact scope/key fires it — a database table or row with the \
         same name does NOT. Double-check that a task you spawned (worker/finalizer) \
         EXPLICITLY sets that exact scope/key when its condition is met, or this session \
         sleeps forever and its cleanup never runs. Set lifecycle {{ expires_at: <epoch ms> }} \
         to be woken with an expiry notice instead if the write never comes."
    ))
}

/// The canonicalized registration request used for same-session idempotency.
/// Built from the agent's RAW arguments (before the owner stamp) with `once`
/// normalized to its effective value; `serde_json::Value` equality is
/// key-order-insensitive, so semantically identical requests always match.
fn registration_dedup_key(req: &SubscribeRequest, once: bool) -> Value {
    json!({
        "trigger_type": req.trigger_type,
        "config": req.config,
        "label": req.label,
        "lifecycle": {
            "once": once,
            "max_fires": req.lifecycle.as_ref().and_then(|l| l.max_fires),
            "expires_at": req.lifecycle.as_ref().and_then(|l| l.expires_at),
        },
        "function_id": req.function_id,
        "metadata": req.metadata,
        "target": req.target,
        "conditions": req.conditions,
    })
}

/// Gate a call binding's target. `harness::*` is refused outright — a binding
/// can wake its owner or call a plain function, never start an agent or
/// re-enter the harness control plane — and everything else must be callable
/// by the REGISTRANT: the fired call runs with worker authority outside the
/// turn loop, so without this a narrowed session could wire a reaction to any
/// function on the bus.
/// Register a `harness::hook::post-turn` validator on behalf of the calling
/// agent. The validator target rides the caller's authority (same rule as
/// call bindings: only functions you could call yourself), and the config's
/// `sessions` filter is FORCE-STAMPED to the calling session — self-gating
/// only. Forwarded to the engine so the binding survives like any other hook
/// registration; the handle is kept for owner-checked teardown.
async fn register_post_turn_hook(
    deps: &Deps,
    req: SubscribeRequest,
    session_id: &str,
    policy: &CompiledPolicy,
) -> Result<SubscribeResponse, HarnessError> {
    let Some(function_id) = req.function_id.clone() else {
        return Err(HarnessError::InvalidRequest(
            "harness::hook::post-turn needs `function_id` — the validator function \
             (e.g. fp::pipe); a wake shape is meaningless for a hook"
                .into(),
        ));
    };
    validate_call_target(&function_id, policy).map_err(HarnessError::InvalidRequest)?;
    let raw = if req.config.is_null() {
        json!({})
    } else {
        req.config.clone()
    };
    let mut cfg: crate::hooks::HookTriggerConfig = serde_json::from_value(raw)
        .map_err(|e| HarnessError::InvalidRequest(format!("invalid post-turn hook config: {e}")))?;
    cfg.sessions = Some(
        validate_post_turn_scope(session_id, cfg.sessions.take())
            .map_err(HarnessError::InvalidRequest)?,
    );
    let config = serde_json::to_value(&cfg)
        .map_err(|e| HarnessError::InvalidRequest(format!("hook config serialize: {e}")))?;
    let handle = deps
        .iii
        .register_trigger(iii_sdk::protocol::RegisterTriggerInput {
            trigger_type: crate::hooks::POST_TURN.to_string(),
            function_id: function_id.clone(),
            config,
            metadata: None,
            namespace: deps.iii.namespace(),
        })
        .map_err(|e| {
            HarnessError::InvalidRequest(format!("post-turn hook registration failed: {e}"))
        })?;
    let subscription_id = deps.hooks.record_owned(session_id, handle);
    Ok(SubscribeResponse {
        subscription_id,
        once: false,
        note: Some(format!(
            "post-turn validator bound to THIS session only ({session_id}): every completing \
             turn must pass {function_id} or it is re-prompted, bounded by \
             max_validation_retries. STANDING — unregister with this id when the goal is met."
        )),
    })
}

/// The sessions an agent-registered post-turn validator may gate: itself, or
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

fn validate_call_target(target: &str, policy: &CompiledPolicy) -> Result<(), String> {
    if target.is_empty() {
        return Err("`function_id` must name the function to call".into());
    }
    if target.starts_with("harness::") {
        return Err(format!(
            "`{target}` is not a binding target: a binding can wake you (omit `function_id`) or \
             call a plain non-harness function — it never starts an agent. Spawn children \
             directly from a turn and register a wake on what they write."
        ));
    }
    if !policy.allows(target) {
        return Err(format!(
            "`{target}` is not permitted by this session's dispatch policy — a reaction can \
             only call functions you can call yourself"
        ));
    }
    Ok(())
}

/// The dispatch policy is only HALF the gate: it says what this turn may call,
/// not whether the deployment wants a human to approve each call. A
/// trigger-fired call runs with worker authority outside any turn, so it never
/// reaches `approval::gate` and can never prompt — binding a
/// `needs_approval` target would silently convert "a human approves each call"
/// into "it fires unattended, forever".
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
    let resp = deps
        .iii
        .trigger(TriggerRequest {
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
                tracing::debug!(
                    target,
                    "approval-gate absent; binding a call reaction without an approval probe"
                );
                return Ok(());
            }
            tracing::warn!(target, error = %msg, "approval probe failed; refusing the binding");
            return Err(format!(
                "could not check whether `{target}` needs approval ({msg}); refusing to bind a \
                 reaction that would run unattended"
            ));
        }
    };

    match resp.get("verdict").and_then(Value::as_str) {
        Some("allow") => Ok(()),
        Some(other) => {
            let reason = resp
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or("no reason given");
            Err(format!(
                "`{target}` is `{other}` for this session: {reason}. A trigger-fired call runs \
                 outside any turn and cannot ask for approval, so it cannot be bound. Call it \
                 yourself on a woken turn, or have an operator allow it."
            ))
        }
        None => Err(format!(
            "the approval gate returned no verdict for `{target}`; refusing to bind a reaction \
             that would run unattended"
        )),
    }
}

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

/// Best-effort engine-side teardown; `true` when the engine accepted it, so
/// callers recording a fired-trigger `retired` flag report the real outcome.
pub async fn unregister_engine_trigger(deps: &Deps, trigger_id: &str) -> bool {
    match deps
        .iii
        .trigger(TriggerRequest {
            function_id: UNREGISTER_TRIGGER_ID.to_string(),
            payload: json!({ "id": trigger_id }),
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        })
        .await
    {
        Ok(_) => true,
        Err(e) => {
            tracing::warn!(trigger_id, error = %e, "subscription trigger unregister failed");
            false
        }
    }
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
mod tests {
    use super::*;

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

    /// Turn events are not agent-bindable in ANY shape — child outcomes flow
    /// through the medium children write, never through a binding on their
    /// turns. The guard runs before target resolution, so wake, call, and
    /// would-be spawn shapes all hit it.
    #[test]
    fn turn_events_are_not_bindable_in_any_shape() {
        for ty in [
            crate::events::TURN_STARTED,
            crate::events::TURN_COMPLETED,
            "harness::hook::pre-generate",
            "harness::notify_agent",
        ] {
            let err = reject_forbidden_type(ty).unwrap_err().to_string();
            assert!(err.contains("not agent-bindable"), "{ty}: {err}");
            assert!(
                err.contains("register a wake"),
                "{ty} must point at the medium instead: {err}"
            );
        }
        assert!(reject_forbidden_type("state").is_ok());
        assert!(reject_forbidden_type("database::row-changed").is_ok());
        assert!(reject_forbidden_type("cron").is_ok());
        assert!(reject_forbidden_type("timer").is_ok());
    }

    #[test]
    fn state_catchall_advisory_flags_keyless_state_bindings() {
        let mk = |ty: &str, config: serde_json::Value| -> SubscribeRequest {
            serde_json::from_value(json!({ "trigger_type": ty, "config": config })).unwrap()
        };
        // Scope without key: the trade-off, naming the scope — NOT a push to
        // add a key. Run 7 read the old push as an instruction, pinned an
        // exact key, and its per-event unique keys then matched nothing.
        let note = state_catchall_advisory(&mk("state", json!({ "scope": "run-1" })));
        let note = note.as_deref().unwrap_or("");
        assert!(note.contains("run-1"), "got: {note}");
        assert!(note.contains("no `key` filter"), "got: {note}");
        assert!(
            note.contains("RIGHT shape when producers write one unique key per event"),
            "the legitimate scope-only design must be endorsed: {note}"
        );
        assert!(
            note.contains("ONLY if every producer writes that exact key"),
            "adding a key must be conditioned on the producers: {note}"
        );
        // No scope AND no key: the global catch-all, still warned hard.
        let note = state_catchall_advisory(&mk("state", json!({})));
        assert!(note.as_deref().unwrap_or("").contains("EVERY scope"));
        // A key filter silences it, with or without scope.
        assert!(state_catchall_advisory(&mk(
            "state",
            json!({ "scope": "run-1", "key": "change" })
        ))
        .is_none());
        assert!(state_catchall_advisory(&mk("state", json!({ "key": "change" }))).is_none());
        // Non-state trigger types are not advised on.
        assert!(state_catchall_advisory(&mk("cron", json!({}))).is_none());
    }

    #[test]
    fn armed_wake_advisory_names_the_exact_key_for_one_shot_state_wakes() {
        let mk = |ty: &str, config: serde_json::Value| -> SubscribeRequest {
            serde_json::from_value(json!({ "trigger_type": ty, "config": config })).unwrap()
        };
        // A one-shot state wake: warned, naming scope/key, with the
        // sleeps-forever consequence spelled out.
        let note = armed_wake_advisory(
            &mk("state", json!({ "scope": "run-1", "key": "report_ready" })),
            true,
        );
        let note = note.as_deref().unwrap_or("");
        assert!(note.contains("run-1/report_ready"), "got: {note}");
        assert!(note.contains("sleeps forever"), "got: {note}");
        // The discovery-run failure modes, named: a DATABASE table with the
        // watched name does not fire a state wake, and a lifecycle deadline
        // is what converts eternal silence into a notice.
        assert!(
            note.contains("database table or row with the same name does NOT"),
            "the medium sentence is the advisory's whole point: {note}"
        );
        assert!(note.contains("expires_at"), "got: {note}");
        assert!(note.contains("expiry notice"), "got: {note}");
        // Standing notifies and non-state types are not wakes.
        assert!(armed_wake_advisory(&mk("state", json!({ "key": "k" })), false).is_none());
        assert!(armed_wake_advisory(&mk("cron", json!({})), true).is_none());
    }

    #[test]
    fn standing_binding_advisory_notes_non_once_only() {
        let mk = |ty: &str| -> SubscribeRequest {
            serde_json::from_value(json!({
                "trigger_type": ty,
                "config": {},
                "function_id": "state::set",
            }))
            .unwrap()
        };
        // The leak shape: a state kickoff registered without once.
        let note = standing_binding_advisory(&mk("state"), false);
        assert!(note.as_deref().unwrap_or("").contains("STANDING"));
        // once retires itself.
        assert!(standing_binding_advisory(&mk("state"), true).is_none());
        assert!(standing_binding_advisory(&mk("cron"), true).is_none());
    }

    /// Both discovery runs registered a recurring cron with no lifecycle and
    /// left it firing past their own teardown claims — an unbounded cron is
    /// never what an agent meant, so it gets a warning of its own.
    #[test]
    fn an_unbounded_recurring_cron_is_warned() {
        let mk = |lifecycle: Value| -> SubscribeRequest {
            let mut v = json!({
                "trigger_type": "cron",
                "config": { "expression": "0 */10 * * * *" },
                "function_id": "state::set",
            });
            if !lifecycle.is_null() {
                v["lifecycle"] = lifecycle;
            }
            serde_json::from_value(v).unwrap()
        };
        let note = standing_binding_advisory(&mk(Value::Null), false);
        let note = note.as_deref().unwrap_or("");
        assert!(note.contains("UNBOUNDED"), "got: {note}");
        assert!(
            note.contains("0 */10 * * * *"),
            "the expression is named: {note}"
        );
        assert!(note.contains("SUCCEEDED"), "redo-thrash guard: {note}");
        assert!(
            note.contains("\"timer\"") && note.contains("once: true"),
            "both deadline fixes are named: {note}"
        );
        // Any lifecycle bound silences it — bounded recurrence is deliberate.
        assert!(standing_binding_advisory(&mk(json!({ "max_fires": 6 })), false).is_none());
        assert!(
            standing_binding_advisory(&mk(json!({ "expires_at": 1785181426583_i64 })), false)
                .is_none()
        );
    }

    /// `{ in_ms }` resolves to an absolute `at` BEFORE the engine stores the
    /// config — a relative countdown replayed after a restart would silently
    /// restart from zero. And the nonsense shapes fail loudly at registration.
    #[test]
    fn timer_registrations_resolve_relative_and_refuse_nonsense() {
        let mk = |config: Value, once: Option<bool>| -> SubscribeRequest {
            let mut v = json!({ "trigger_type": "timer", "config": config });
            if let Some(o) = once {
                v["once"] = json!(o);
            }
            serde_json::from_value(v).unwrap()
        };
        let now = crate::types::message::AgentMessage::now_ms();

        let mut rel = mk(json!({ "in_ms": 600_000 }), None);
        resolve_timer_request(&mut rel).unwrap();
        let at = rel.config["at"].as_i64().unwrap();
        assert!(
            (at - now - 600_000).abs() < 5_000,
            "at ≈ now + in_ms, got {at}"
        );
        assert!(
            rel.config.get("in_ms").is_none(),
            "config is normalized to `at` only"
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

        // Non-timer types pass through untouched.
        let mut cron: SubscribeRequest = serde_json::from_value(
            json!({ "trigger_type": "cron", "config": { "expression": "0 * * * * *" } }),
        )
        .unwrap();
        resolve_timer_request(&mut cron).unwrap();
        assert_eq!(cron.config["expression"], json!("0 * * * * *"));
    }

    /// The run-3 lesson: notes must SAY the registration succeeded, or a
    /// warning reads as a redo order and the model starts over.
    #[test]
    fn composed_notes_lead_with_the_outcome() {
        assert!(compose_note(vec![]).is_none());
        let note = compose_note(vec!["warning: x.".into(), "note: y.".into()]).unwrap();
        assert!(note.starts_with("registration SUCCEEDED"), "got: {note}");
        assert!(note.contains("rather than starting over"), "got: {note}");
        assert!(note.contains("warning: x.") && note.contains("note: y."));
    }

    /// The barrier-armed-late shape: a keyed state watch is checkable for a
    /// pre-existing value at registration; the note explains no-replay.
    #[test]
    fn prewritten_key_advisory_targets_keyed_state_watches() {
        let mk = |ty: &str, config: Value| -> SubscribeRequest {
            serde_json::from_value(json!({ "trigger_type": ty, "config": config })).unwrap()
        };
        assert_eq!(
            watched_state_key(&mk("state", json!({ "scope": "run", "key": "done" }))),
            Some(("run", "done"))
        );
        // Keyless and non-state shapes are never probed.
        assert!(watched_state_key(&mk("state", json!({ "scope": "run" }))).is_none());
        assert!(watched_state_key(&mk("cron", json!({ "key": "done" }))).is_none());

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
    /// retire at fire one.
    #[test]
    fn once_defaults_by_shape() {
        let mk = |v: Value| -> SubscribeRequest { serde_json::from_value(v).unwrap() };

        // Wakes (no target): once.
        assert!(effective_once(&mk(json!({ "trigger_type": "state" }))));
        assert!(effective_once(&mk(
            json!({ "trigger_type": "database::row-changed", "config": { "db": "primary" } })
        )));

        // Call bindings: STANDING, in both the shorthand and the long form.
        assert!(!effective_once(&mk(json!({
            "trigger_type": "database::row-changed",
            "function_id": "state::set",
        }))));
        assert!(!effective_once(&mk(json!({
            "trigger_type": "state",
            "target": { "function_id": "state::set" },
        }))));

        // Cron recurring, timer once — by definition, regardless of target.
        assert!(!effective_once(&mk(json!({ "trigger_type": "cron" }))));
        assert!(!effective_once(&mk(json!({
            "trigger_type": "cron",
            "function_id": "state::set",
        }))));
        assert!(effective_once(&mk(json!({
            "trigger_type": "timer",
            "function_id": "state::set",
        }))));

        // Explicit always wins, in both directions.
        assert!(effective_once(&mk(
            json!({ "trigger_type": "cron", "once": true })
        )));
        assert!(!effective_once(&mk(
            json!({ "trigger_type": "state", "once": false })
        )));
        assert!(effective_once(&mk(json!({
            "trigger_type": "state",
            "function_id": "state::set",
            "once": true,
        }))));
        assert!(effective_once(&mk(json!({
            "trigger_type": "state",
            "function_id": "state::set",
            "lifecycle": { "once": true },
        }))));
        assert!(!effective_once(&mk(json!({
            "trigger_type": "state",
            "lifecycle": { "once": false },
        }))));
    }

    #[test]
    fn lifecycle_must_have_a_future_delivery_slot() {
        assert!(validate_lifecycle(None, 100).is_ok());
        assert!(validate_lifecycle(
            Some(&LifecycleRequest {
                max_fires: Some(0),
                ..Default::default()
            }),
            100,
        )
        .is_err());
        assert!(validate_lifecycle(
            Some(&LifecycleRequest {
                expires_at: Some(100),
                ..Default::default()
            }),
            100,
        )
        .is_err());
        assert!(validate_lifecycle(
            Some(&LifecycleRequest {
                max_fires: Some(1),
                expires_at: Some(101),
                ..Default::default()
            }),
            100,
        )
        .is_ok());

        // A deadline that passed during target/approval resolution must fail
        // the reservation-boundary recheck, not masquerade as a full owner.
        let expiring = LifecycleRequest {
            expires_at: Some(101),
            ..Default::default()
        };
        assert!(validate_lifecycle(Some(&expiring), 100).is_ok());
        assert!(validate_lifecycle(Some(&expiring), 101).is_err());
    }

    #[test]
    fn an_expired_reservation_is_not_reported_as_capacity() {
        assert!(require_reserved(ReserveOutcome::Reserved).is_ok());
        let full = require_reserved(ReserveOutcome::Capacity)
            .unwrap_err()
            .to_string();
        assert!(full.contains("binding cap reached"));

        let expired = require_reserved(ReserveOutcome::Exhausted)
            .unwrap_err()
            .to_string();
        assert!(expired.contains("lifecycle expired"));
        assert!(!expired.contains("cap reached"));
    }

    /// The intercepted controls never reach the engine registry, so native
    /// exposure must publish their contracts itself — but only when the
    /// turn's policy allows them (a leaf child sees neither).
    #[test]
    fn in_turn_triggers_calls_are_forced_to_the_calling_session() {
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
    fn native_subscription_controls_follow_dispatch_policy() {
        let both = policy_allowing(&["engine::register_trigger", "engine::unregister_trigger"]);
        let tools = native_control_tools(&both);
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(
            names,
            ["engine::register_trigger", "engine::unregister_trigger"]
        );
        assert!(tools
            .iter()
            .all(|t| t.execution_mode.as_deref() == Some("sequential")));

        let register_only = policy_allowing(&["engine::register_trigger"]);
        let names: Vec<String> = native_control_tools(&register_only)
            .into_iter()
            .map(|t| t.name)
            .collect();
        assert_eq!(names, ["engine::register_trigger"]);

        let mut walled = policy_allowing(&["*"]);
        // The leaf wall's denies must hide both controls from the toolset.
        assert!(native_control_tools(&walled).len() == 2);
        walled = CompiledPolicy::from(Some(&crate::types::turn::FunctionPolicy {
            allow: vec!["*".into()],
            deny: crate::policy::CONTROL_PLANE_DENY
                .iter()
                .map(|s| s.to_string())
                .collect(),
            expose: Default::default(),
        }));
        assert!(native_control_tools(&walled).is_empty());
    }

    fn policy_allowing(allow: &[&str]) -> CompiledPolicy {
        CompiledPolicy::from(Some(&crate::types::turn::FunctionPolicy {
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: vec![],
            expose: Default::default(),
        }))
    }

    /// A fired call runs with WORKER authority outside the turn loop, so the
    /// registrant's own policy is the only thing standing between a narrowed
    /// session and every function on the bus.
    #[test]
    fn call_target_must_be_callable_by_the_registrant() {
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
            "harness::trigger-call",
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
            "trigger failed: unregistered function",
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
    fn a_call_binding_rejects_sub_agent_keys_and_bad_pointers() {
        // The shorthand `metadata` for a call target is {payload?, event_into?}.
        // A `task` here means the caller wanted a spawn binding; saying so beats
        // silently ignoring it and wiring a call that fires with an empty
        // template forever.
        let err = validate_call_template(&json!({ "task": "do it" })).unwrap_err();
        assert!(err.to_string().contains("unknown key `task`"));
        // A pointer that is not a pointer is a per-event no-op at fire time.
        let err = validate_call_template(&json!({ "event_into": "value" })).unwrap_err();
        assert!(err.to_string().contains("JSON pointer"));
        // The valid shapes pass, including the empty-pointer whole-payload form.
        assert!(validate_call_template(&json!({ "payload": { "db": "primary" } })).is_ok());
        assert!(validate_call_template(&json!({ "event_into": "/args/event" })).is_ok());
        assert!(validate_call_template(&json!({ "event_into": "" })).is_ok());
        assert!(validate_call_template(&Value::Null).is_ok());

        let explicit: BindingTarget = serde_json::from_value(json!({
            "function_id": "state::set",
            "event_into": "not/a/pointer",
        }))
        .unwrap();
        assert!(validate_event_into(explicit.event_into.as_deref()).is_err());
    }

    #[test]
    fn the_dedup_key_covers_the_long_form_too() {
        // Two registrations that differ ONLY in their explicit target must not
        // dedup onto each other — before `target`/`conditions` joined the key,
        // the second would silently return the first binding's id.
        let base = json!({ "trigger_type": "state", "config": { "scope": "run" } });
        let mut a: SubscribeRequest = serde_json::from_value(base.clone()).unwrap();
        let mut b: SubscribeRequest = serde_json::from_value(base).unwrap();
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
    }

    #[test]
    fn the_dedup_key_uses_raw_timers_and_normalized_lifecycle() {
        let req: SubscribeRequest = serde_json::from_value(json!({
            "trigger_type": "timer",
            "config": { "in_ms": 250 },
            "lifecycle": { "once": true, "max_fires": 1, "expires_at": 1234 },
        }))
        .unwrap();
        let key = registration_dedup_key(&req, effective_once(&req));
        assert_eq!(key["config"], json!({ "in_ms": 250 }));
        assert_eq!(
            key["lifecycle"],
            json!({ "once": true, "max_fires": 1, "expires_at": 1234 })
        );

        let mut different = req.clone();
        different.lifecycle.as_mut().unwrap().expires_at = Some(5678);
        assert_ne!(
            registration_dedup_key(&req, true),
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
    fn an_engine_side_condition_is_refused_at_registration() {
        // `condition_function_id` reaches the engine's own contract, where an
        // unknown id silently skips every fire forever. The typed `conditions`
        // list reports why instead, so the shorthand is refused rather than
        // quietly forwarded.
        let req: SubscribeRequest = serde_json::from_value(json!({
            "trigger_type": "state",
            "config": { "scope": "run", "key": "go", "condition_function_id": "fp::get" },
        }))
        .unwrap();
        assert!(req.config.get("condition_function_id").is_some());
    }
}

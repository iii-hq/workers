//! Subscriptions — register an ephemeral iii trigger and be notified when it
//! fires, instead of polling (harness.md § Subscriptions). The agent calls
//! `engine::register_trigger` / `engine::unregister_trigger`; the harness
//! intercepts those calls (see [`invoke`]) so the trusted owning session,
//! `harness::notify_agent` target, and subscription metadata are injected, and
//! teardown stays owner-checked — the agent can never supply those. The engine
//! stores this metadata on the `Trigger` and delivers it to `notify_agent` as a
//! distinct invocation argument at fire time (not folded into the payload).

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::TriggerAction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::clients::EngineClient;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::policy::CompiledPolicy;
use crate::subscriptions::{self, NOTIFY_AGENT_ID};
use crate::trigger::{self, ResultData};
use crate::types::content::ContentBlock;

/// The engine function the agent calls to subscribe. The harness intercepts it
/// (the agent never reaches the raw engine registrar) so it can stamp the
/// trusted session and bind the trigger to `harness::notify_agent`.
pub const REGISTER_TRIGGER_ID: &str = "engine::register_trigger";

/// The engine function the agent calls to unsubscribe. The harness intercepts it
/// so it resolves the caller's subscription, enforces ownership, and unregisters
/// the underlying engine trigger.
pub const UNREGISTER_TRIGGER_ID: &str = "engine::unregister_trigger";

/// Agent-facing subscription contract.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[schemars(rename = "SubscribeArgs")]
pub struct SubscribeRequest {
    /// The iii trigger type to listen on: `cron`, `state`, `stream`, or another
    /// worker's custom trigger type (e.g. `approval::pending-resolved`). For an
    /// ad-hoc signal, subscribe to `state` on a key and have the signaller call
    /// `state::set` on it (no dedicated emit needed — the engine fans the trigger
    /// out to every subscriber).
    pub trigger_type: String,
    /// The trigger config, passed verbatim to the engine — e.g.
    /// `{ "expression": "0 */5 * * * *" }` for cron, or a `state` scope/key.
    #[serde(default)]
    pub config: Value,
    /// A short human label echoed back in the notification text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Auto-unsubscribe after the first delivered notification. Defaults to true
    /// for one-shot-ish types (state / stream / custom trigger types), false for
    /// recurring `cron`. On `harness::react` bindings only an explicit `true`
    /// is honored (the binding retires after its first successful spawn), and
    /// it is ignored for join predecessors — the join owns their lifecycle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
    /// Target function fired on each event. Omit for a notification message
    /// into this session. The ONLY explicit target allowed is `harness::react`
    /// (spawn a sub-agent from the event) — pass the reaction spec in
    /// `metadata`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    /// `harness::react` reaction spec: `{ task, model?, session_id?,
    /// parent_session_id?, options?, join? }`. Required (with `task`) when
    /// `function_id` is `harness::react`; forwarded verbatim, with `model`
    /// (and `provider`, when unset) defaulted to the registering turn's
    /// when omitted. The reaction INHERITS the registering turn's dispatch
    /// policy; `options.functions` narrows it and can never escalate (same
    /// shape as `harness::spawn` options, e.g. `{ "functions": { "allow":
    /// ["state::get"] } }`). Only raw engine-side registrations fall back to
    /// the read-only default policy. Join predecessors auto-unregister after
    /// the join fires unless `join.rearm: true` keeps them registered for the
    /// next complete set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
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

/// The `session_id` a turn-event registration filters on, if any — the only
/// filter shape whose typo/mismatch starves silently.
fn turn_event_session_filter(req: &SubscribeRequest) -> Option<&str> {
    if !is_turn_event_type(&req.trigger_type) {
        return None;
    }
    req.config.get("session_id").and_then(Value::as_str)
}

fn is_turn_event_type(trigger_type: &str) -> bool {
    trigger_type == crate::events::TURN_STARTED || trigger_type == crate::events::TURN_COMPLETED
}

/// Advisory for a turn-event join predecessor filtered on `parent_session_id`:
/// it matches EVERY child completion under that parent, but always records
/// under its one fixed `join.key` — with a single such binding the join's other
/// keys never fill and it starves (later children overwrite the same slot);
/// with one per key the FIRST completion fills every key with duplicate
/// payloads. Fires only for multi-key joins with no `session_id` narrowing the
/// match to one child. Pure; advisory only.
fn parent_filter_join_advisory(req: &SubscribeRequest) -> Option<String> {
    if !is_turn_event_type(&req.trigger_type) || req.config.get("session_id").is_some() {
        return None;
    }
    let parent = req
        .config
        .get("parent_session_id")
        .and_then(Value::as_str)?;
    let join = req.metadata.as_ref()?.get("join")?;
    let key = join.get("key").and_then(Value::as_str)?;
    if join.get("expect").and_then(Value::as_array)?.len() < 2 {
        return None;
    }
    Some(format!(
        "warning: this join predecessor filters on parent_session_id \"{parent}\", which matches \
         EVERY child completion under that parent but always records under its fixed key \
         \"{key}\" — the join's other expected keys never fill and it starves. Register ONE \
         subscription per predecessor with config {{ session_id: \"<that predecessor's child \
         session id>\" }} instead."
    ))
}

/// Advisory for a turn-event registration filtering on a session that doesn't
/// exist: it only ever fires if something creates that EXACT session. The
/// classic wiring mismatch is pinning invented child ids in a join's filters
/// while leaving the upstream spawn specs unpinned — the join then starves at
/// 0/N forever. Fail-open: a lookup error produces no note.
async fn session_filter_advisory(deps: &Deps, req: &SubscribeRequest) -> Option<String> {
    let sid = turn_event_session_filter(req)?;
    let exists = deps
        .iii
        .trigger(TriggerRequest {
            function_id: "session::get".to_string(),
            payload: json!({ "session_id": sid }),
            action: None,
            timeout_ms: None,
        })
        .await
        .map(|v| v.get("meta").is_some())
        .unwrap_or(true);
    if exists {
        return None;
    }
    Some(format!(
        "warning: session \"{sid}\" does not exist — this binding only fires if something \
         creates that exact session. If it is an upstream reaction's child, pin the SAME id on \
         that reaction's spec `session_id` (or join on state keys instead)."
    ))
}

/// The registering turn's model identity and dispatch policy, threaded from
/// the dispatch chokepoints so a react spec can inherit them (see
/// [`inherit_model`] and [`stamp_registrant_functions`]).
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
        _ => trigger::invoke_target(engine, policy, function_id, arguments).await,
    }
}

fn defaults_recurring(trigger_type: &str) -> bool {
    trigger_type == "cron"
}

fn effective_once(req: &SubscribeRequest) -> bool {
    req.once.unwrap_or(!defaults_recurring(&req.trigger_type))
}

/// Advisory for a standing react binding (no `once`, no join): agents keep
/// registering one-run pipeline kickoffs without `once: true`, leaving bindings
/// that respawn the whole pipeline on every future matching event. Purely
/// informative — deliberate standing watchers ignore it; cron is exempt
/// (recurring is its whole point).
fn standing_binding_advisory(req: &SubscribeRequest, once: bool) -> Option<String> {
    if once
        || req.trigger_type == "cron"
        || req
            .metadata
            .as_ref()
            .is_some_and(|m| m.get("join").is_some())
    {
        return None;
    }
    Some(
        "note: no `once` set — this reaction is STANDING and refires on EVERY future matching \
         event until unregistered; pass `once: true` if it should fire for one pipeline run only."
            .to_string(),
    )
}

/// Advisory for a `state` binding with no `key` filter: it fires for EVERY
/// key written in the scope (or every write anywhere, with no scope either) —
/// order signals, done markers, completion keys, all of it. Registered beside
/// a keyed binding it double-fires every event and burns the fire-rate budget
/// twice (rctest-k7m3 postmortem). Purely informative — a deliberate
/// catch-all watcher ignores it.
fn state_catchall_advisory(req: &SubscribeRequest) -> Option<String> {
    if req.trigger_type != "state" || req.config.get("key").is_some() {
        return None;
    }
    let breadth = match req.config.get("scope").and_then(Value::as_str) {
        Some(scope) => format!("EVERY key written in scope \"{scope}\""),
        None => "EVERY state write in EVERY scope".to_string(),
    };
    Some(format!(
        "warning: this state binding has no `key` filter — it fires for {breadth} (done \
         markers, completion signals, everything), each fire spending this subscription's \
         fire-rate budget. Add a `key` to the config unless you deliberately want a catch-all."
    ))
}

/// `once` on a react binding: simple non-cron edges default to one-shot, while
/// cron remains standing. Callers that truly want a standing state/turn watcher
/// opt out with `once: false`. Join predecessors ignore this field because the
/// join owns their lifecycle (auto-unregister on fire, `rearm` to keep).
fn react_once(req: &SubscribeRequest) -> bool {
    req.metadata.as_ref().and_then(|m| m.get("join")).is_none()
        && req.once.unwrap_or(!defaults_recurring(&req.trigger_type))
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

/// A react spec with no `model` inherits the registering turn's — agents drop
/// `metadata.model` often enough that requiring it just turned working pipelines
/// into failed registrations, and the harness already knows the answer. The
/// caller's `provider` rides along when the spec pins none, so an inherited
/// model cannot route to a different provider than the registering turn's
/// (ambiguous ids resolve per-provider). Only a truly absent or empty-string
/// `model` inherits: present-but-mistyped values (`null`, `false`, `0`) still
/// fail spec validation loudly rather than silently running on another model.
/// Metadata key carrying the registering turn's dispatch policy on a react
/// binding. At fire time a reaction with no `options.functions` inherits it,
/// and explicit options are subset against it (narrow, never escalate) —
/// matching the in-turn child rule instead of dropping a reaction delivered
/// into the registrant's own chat to the read-only baseline (the rctest-k7m3
/// wrap-up turn was denied database::query / state::set /
/// engine::unregister_trigger for exactly that reason).
pub const REGISTRANT_FUNCTIONS_KEY: &str = "__registrant_functions";

/// Stamp the registering turn's dispatch policy onto a react binding's
/// metadata. Harness stamp, never caller-supplied: any smuggled value is
/// dropped even when there is no caller policy to stamp.
fn stamp_registrant_functions(metadata: &mut Option<Value>, caller: Option<CallerModel<'_>>) {
    let Some(Value::Object(m)) = metadata.as_mut() else {
        return;
    };
    m.remove(REGISTRANT_FUNCTIONS_KEY);
    let Some(functions) = caller.and_then(|c| c.functions) else {
        return;
    };
    if let Ok(v) = serde_json::to_value(functions) {
        m.insert(REGISTRANT_FUNCTIONS_KEY.to_string(), v);
    }
}

fn inherit_model(metadata: &mut Option<Value>, caller: Option<CallerModel<'_>>) {
    let (Some(Value::Object(m)), Some(caller)) = (metadata.as_mut(), caller) else {
        return;
    };
    let inheritable = match m.get("model") {
        None => true,
        Some(Value::String(s)) => s.is_empty(),
        Some(_) => false,
    };
    if !inheritable {
        return;
    }
    m.insert("model".to_string(), Value::String(caller.model.to_string()));
    if !m.contains_key("provider") {
        if let Some(provider) = caller.provider {
            m.insert("provider".to_string(), Value::String(provider.to_string()));
        }
    }
}

async fn intercept_unregister(deps: &Deps, args: &Value, session_id: &str) -> ResultData {
    let id = match unregister_subscription_id(args) {
        Ok(id) => id,
        Err(e) => return error_result(e),
    };

    if let Some(owner) = deps.subscriptions.session_of(id) {
        if owner != session_id {
            return error_result("subscription belongs to a different session".to_string());
        }
    }

    let removed = match deps.subscriptions.take(id) {
        Some((_session, trigger_id)) => {
            if let Some(trigger_id) = trigger_id {
                unregister_engine_trigger(deps, &trigger_id).await;
            }
            true
        }
        None => false,
    };
    ok_result(&json!({ "removed": removed }))
}

fn unregister_subscription_id(args: &Value) -> Result<&str, String> {
    args.get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| "engine::unregister_trigger requires an `id`".to_string())
}

async fn handle(
    deps: &Deps,
    req: SubscribeRequest,
    session_id: &str,
    caller: Option<CallerModel<'_>>,
    policy: &CompiledPolicy,
) -> Result<SubscribeResponse, HarnessError> {
    if let Some(fid) = req.function_id.as_deref() {
        if fid != crate::functions::react::REACT_ID {
            return Err(HarnessError::InvalidRequest(format!(
                "only `{}` may be a subscription target (got `{fid}`); omit `function_id` to be notified in this session instead",
                crate::functions::react::REACT_ID
            )));
        }
        return handle_react(deps, req, session_id, caller, policy).await;
    }

    if subscriptions::is_forbidden_trigger_type(&req.trigger_type) {
        return Err(HarnessError::InvalidRequest(format!(
            "cannot bind harness-internal trigger type `{}` (self-notification guard)",
            req.trigger_type
        )));
    }

    let once = effective_once(&req);

    // Idempotency: an identical re-registration (a model retry or a re-run
    // prompt in the same session) returns the standing subscription instead
    // of wiring a twin binding that would double-deliver forever.
    let dedup = registration_dedup_key(&req, once);
    if let Some(existing) = deps.subscriptions.find_duplicate(session_id, &dedup) {
        return Ok(SubscribeResponse {
            subscription_id: existing,
            once,
            note: None,
        });
    }

    let sub_id = format!("sub_{}", uuid::Uuid::new_v4().simple());

    // A one-shot notify is an armed wake: its owning session completes
    // non-terminally until the wake fires (or is unregistered) — see the
    // `terminal` flag on `harness::turn-completed`.
    deps.subscriptions
        .try_insert_keyed(
            &sub_id,
            session_id,
            subscriptions::MAX_SUBSCRIPTIONS_PER_SESSION,
            dedup,
            once,
        )
        .map_err(|_| {
            HarnessError::InvalidRequest(format!(
                "subscription cap reached ({} active for this session); unsubscribe first",
                subscriptions::MAX_SUBSCRIPTIONS_PER_SESSION
            ))
        })?;

    let resp = deps
        .iii
        .trigger(register_trigger_request(
            &req,
            &sub_id,
            session_id,
            once,
            deps.cfg().await.dispatch_timeout_ms,
        ))
        .await;

    match resp.map(|v| v.get("id").and_then(Value::as_str).map(str::to_string)) {
        Ok(Some(trigger_id)) => {
            if !deps.subscriptions.set_trigger_id(&sub_id, &trigger_id) {
                // Documented race, not a failure: a `once` fire claimed the
                // slot inside the bind window, so the subscription already
                // delivered — only the orphan engine binding is left to clean
                // up. `Ok` is deliberate; erroring here would invite a
                // duplicate re-registration of work that already ran.
                unregister_engine_trigger(deps, &trigger_id).await;
            }
        }
        // Carry the engine's rejection reason (e.g. an unknown config key for
        // this trigger type) — an opaque "failed" sends the agent guess-looping.
        outcome => {
            deps.subscriptions.take(&sub_id);
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
        session_filter_advisory(deps, &req).await,
        // A key-less state notify wakes the owning session on every write in
        // the scope — same catch-all hazard as a react binding.
        state_catchall_advisory(&req),
    ]
    .into_iter()
    .flatten()
    .collect();
    Ok(SubscribeResponse {
        subscription_id: sub_id,
        once,
        note: (!notes.is_empty()).then(|| notes.join(" ")),
    })
}

/// True when an existing react binding (its `::info` JSON) is a sibling
/// predecessor of the same join bound to the SAME event source: one fire then
/// arrives for both keys and the join completes instantly with duplicate
/// payloads instead of distinct results. Returns the sibling's join key.
fn same_event_join_sibling(
    info: &Value,
    trigger_type: &str,
    config: &Value,
    join_id: &str,
    join_key: &str,
) -> Option<String> {
    let j = info.pointer("/metadata/join")?;
    let jid = j.get("id").and_then(Value::as_str)?;
    let jkey = j.get("key").and_then(Value::as_str)?;
    if jid != join_id || jkey == join_key {
        return None;
    }
    if info.get("trigger_type").and_then(Value::as_str) != Some(trigger_type) {
        return None;
    }
    if info.get("config") != Some(config) {
        return None;
    }
    Some(jkey.to_string())
}

/// Advisory for a join predecessor whose event source is already bound by a
/// sibling key of the same join — the instant-complete duplicate-payload
/// miswire. Best-effort: listing/info failures produce no note.
async fn join_wiring_advisory(deps: &Deps, req: &SubscribeRequest) -> Option<String> {
    let join = req.metadata.as_ref()?.get("join")?;
    let jid = join.get("id").and_then(Value::as_str)?;
    let jkey = join.get("key").and_then(Value::as_str)?;
    let list = deps
        .iii
        .trigger(TriggerRequest {
            function_id: "engine::registered-triggers::list".to_string(),
            payload: json!({ "function_id": crate::functions::react::REACT_ID }),
            action: None,
            timeout_ms: None,
        })
        .await
        .ok()?;
    let ids: Vec<String> = list
        .get("registered_triggers")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|t| t.get("id").and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    for id in ids {
        let Ok(info) = deps
            .iii
            .trigger(TriggerRequest {
                function_id: "engine::registered-triggers::info".to_string(),
                payload: json!({ "id": id }),
                action: None,
                timeout_ms: None,
            })
            .await
        else {
            continue;
        };
        if let Some(sibling) =
            same_event_join_sibling(&info, &req.trigger_type, &req.config, jid, jkey)
        {
            return Some(format!(
                "warning: join \"{jid}\" key \"{sibling}\" is already bound to this exact event \
                 source — one event then arrives for BOTH keys and the join completes instantly \
                 with duplicate payloads. Point each predecessor key at a distinct source."
            ));
        }
    }
    None
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
        "once": once,
        "function_id": req.function_id,
        "metadata": req.metadata,
    })
}

fn register_trigger_request(
    req: &SubscribeRequest,
    sub_id: &str,
    session_id: &str,
    once: bool,
    timeout_ms: u64,
) -> TriggerRequest {
    TriggerRequest {
        function_id: REGISTER_TRIGGER_ID.to_string(),
        payload: json!({
            "trigger_type": req.trigger_type,
            "function_id": NOTIFY_AGENT_ID,
            "config": req.config.clone(),
            "metadata": {
                "subscription_id": sub_id,
                "session_id": session_id,
                "label": req.label.clone(),
                "once": once,
            },
        }),
        action: None,
        timeout_ms: Some(timeout_ms),
    }
}

/// Trigger types `harness::react` may bind: the two turn-event types (react
/// has its own loop breakers — self-edge drop + depth cap) plus everything
/// that is not harness-internal.
fn react_target_type_allowed(trigger_type: &str) -> bool {
    trigger_type == crate::events::TURN_STARTED
        || trigger_type == crate::events::TURN_COMPLETED
        || !subscriptions::is_forbidden_trigger_type(trigger_type)
}

/// `harness::react` pass-through: the agent binds an event to a sub-agent
/// reaction instead of a notification. Turn-event trigger types are allowed
/// here — react has its own loop breakers (self-edge drop + depth cap) — while
/// every other harness-internal type stays forbidden. The reaction spec is
/// validated synchronously (shape, then model id against the live router
/// catalog) so a bad binding fails this call instead of no-oping at fire time.
async fn handle_react(
    deps: &Deps,
    mut req: SubscribeRequest,
    session_id: &str,
    caller: Option<CallerModel<'_>>,
    policy: &CompiledPolicy,
) -> Result<SubscribeResponse, HarnessError> {
    if !react_target_type_allowed(&req.trigger_type) {
        return Err(HarnessError::InvalidRequest(format!(
            "cannot bind harness-internal trigger type `{}` to `harness::react`",
            req.trigger_type
        )));
    }

    let once = react_once(&req);

    // Idempotency: same rule as the notify path — an identical re-registration
    // returns the standing subscription instead of a twin reaction that would
    // double-spawn on every fire. Keyed on the raw request BEFORE the owner and
    // model stamps: a model-less spec re-registered from a turn on a different
    // model must still match its standing twin, or every model switch would
    // wire a duplicate binding (and a duplicate join edge leaks its twin at
    // teardown — the accumulator keeps one binding id per key).
    let dedup = registration_dedup_key(&req, once);

    let call_mode = req
        .metadata
        .as_ref()
        .is_some_and(|m| m.get("call").is_some());
    if call_mode {
        // A call reaction runs with worker authority at fire time, so the
        // REGISTRANT's dispatch policy gates the target here — otherwise a
        // narrowed session could wire a reaction to any function on the bus.
        // Harness-internal targets are refused outright (a call re-entering
        // react/notify would be a self-dispatch loop the loop breakers were
        // never designed for).
        let target = req
            .metadata
            .as_ref()
            .and_then(|m| m.pointer("/call/function_id"))
            .and_then(Value::as_str)
            .unwrap_or("");
        if target.starts_with("harness::") {
            return Err(HarnessError::InvalidRequest(format!(
                "`call.function_id` cannot target harness-internal functions (got `{target}`)"
            )));
        }
        if !target.is_empty() && !policy.allows(target) {
            return Err(HarnessError::InvalidRequest(format!(
                "`call.function_id` `{target}` is not permitted by this session's dispatch \
                 policy — a call reaction can only target functions you can call yourself"
            )));
        }
    } else {
        inherit_model(&mut req.metadata, caller);
    }
    // Both modes: strips any smuggled registrant-policy stamp; inert for call
    // reactions (they dispatch with worker authority, no spawned turn).
    stamp_registrant_functions(&mut req.metadata, caller);
    crate::functions::react::validate_spec(req.metadata.as_ref())
        .map_err(HarnessError::InvalidRequest)?;
    if !call_mode {
        crate::functions::react::validate_model(&deps.iii, req.metadata.as_ref())
            .await
            .map_err(HarnessError::InvalidRequest)?;
    }
    // Join predecessors must agree on the reaction spec: the join fires with
    // the COMPLETING predecessor's spec, so a divergent spec on any
    // predecessor (e.g. a `task: "placeholder"` shorthand on later ones)
    // would silently replace the real reaction at fire time. Fingerprinted
    // post-`inherit_model` so same-session registrations compare stamped
    // metadata consistently.
    let join_probe = crate::functions::react::join_canon(req.metadata.as_ref());
    if let Some((join_id, canon)) = &join_probe {
        if let Some(conflict) = deps.subscriptions.conflicting_join_spec(join_id, canon) {
            return Err(HarnessError::InvalidRequest(format!(
                "join `{join_id}`: this predecessor's reaction spec differs from live \
                 predecessor {conflict}'s. Every predecessor of a join must carry the \
                 IDENTICAL spec (task/call, model, session_id, options, expect) — only \
                 `join.key` may differ. Repeat the full spec on every registration; the \
                 join fires with the completing predecessor's spec."
            )));
        }
    }
    if let Some(existing) = deps.subscriptions.find_duplicate(session_id, &dedup) {
        return Ok(SubscribeResponse {
            subscription_id: existing,
            once,
            note: None,
        });
    }

    let sub_id = format!("sub_{}", uuid::Uuid::new_v4().simple());
    // React bindings spawn reactions elsewhere — never a wake for the
    // registering session.
    deps.subscriptions
        .try_insert_keyed(
            &sub_id,
            session_id,
            subscriptions::MAX_SUBSCRIPTIONS_PER_SESSION,
            dedup,
            false,
        )
        .map_err(|_| {
            HarnessError::InvalidRequest(format!(
                "subscription cap reached ({} active for this session); unsubscribe first",
                subscriptions::MAX_SUBSCRIPTIONS_PER_SESSION
            ))
        })?;

    if let Some((join_id, canon)) = join_probe {
        deps.subscriptions.set_join_probe(
            &sub_id,
            crate::subscriptions::registry::JoinProbe { id: join_id, canon },
        );
    }

    // Stamp the owning session into the metadata so startup reconciliation can
    // GC this binding if the session is deleted while the harness is down.
    // The binding is durable engine-side but its in-memory session tracking is
    // wiped on restart, so this is the only durable owner reference. Also
    // stamp the local subscription handle: the turn-event fan-out overwrites
    // it with the engine binding id at fire time, but state/cron/stream fires
    // deliver metadata as stored — without this stamp their join edges record
    // no binding and a fired join cannot auto-unregister its predecessors.
    let mut metadata = req.metadata.clone().unwrap_or(Value::Null);
    if let Value::Object(m) = &mut metadata {
        m.insert(
            subscriptions::OWNER_SESSION_KEY.to_string(),
            Value::String(session_id.to_string()),
        );
        m.insert(
            "__subscription_id".to_string(),
            Value::String(sub_id.clone()),
        );
        // Harness stamps are never caller-supplied. The other stamps overwrite
        // unconditionally; `__once` is conditional, so a smuggled value would
        // make the binding retire while the response echoes standing.
        m.remove("__once");
        if once {
            m.insert("__once".to_string(), Value::Bool(true));
        }
    }

    let resp = deps
        .iii
        .trigger(TriggerRequest {
            function_id: REGISTER_TRIGGER_ID.to_string(),
            payload: json!({
                "trigger_type": req.trigger_type,
                "function_id": crate::functions::react::REACT_ID,
                "config": req.config,
                "metadata": metadata,
            }),
            action: None,
            timeout_ms: Some(deps.cfg().await.dispatch_timeout_ms),
        })
        .await;

    match resp.map(|v| v.get("id").and_then(Value::as_str).map(str::to_string)) {
        Ok(Some(trigger_id)) => {
            if !deps.subscriptions.set_trigger_id(&sub_id, &trigger_id) {
                // Documented race, not a failure: a `once` fire claimed the
                // slot inside the bind window, so the subscription already
                // delivered — only the orphan engine binding is left to clean
                // up. `Ok` is deliberate; erroring here would invite a
                // duplicate re-registration of work that already ran.
                unregister_engine_trigger(deps, &trigger_id).await;
            }
        }
        // Carry the engine's rejection reason (e.g. an unknown config key for
        // this trigger type) — an opaque "failed" sends the agent guess-looping.
        outcome => {
            deps.subscriptions.take(&sub_id);
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
        parent_filter_join_advisory(&req),
        session_filter_advisory(deps, &req).await,
        join_wiring_advisory(deps, &req).await,
        standing_binding_advisory(&req, once),
        state_catchall_advisory(&req),
    ]
    .into_iter()
    .flatten()
    .collect();
    let note = (!notes.is_empty()).then(|| notes.join(" "));
    Ok(SubscribeResponse {
        subscription_id: sub_id,
        once,
        note,
    })
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

    #[test]
    fn model_less_react_spec_inherits_the_registering_turn_model() {
        // The exact shape that used to fail registration with "missing field `model`".
        let mut md = Some(
            json!({ "task": "summarize", "join": { "id": "J", "expect": ["a"], "key": "a" } }),
        );
        inherit_model(
            &mut md,
            Some(CallerModel {
                model: "claude-opus-4-8",
                provider: None,
                functions: None,
            }),
        );
        assert_eq!(md.as_ref().unwrap()["model"], json!("claude-opus-4-8"));
        assert!(crate::functions::react::validate_spec(md.as_ref()).is_ok());

        // An explicit model wins; an empty one does not.
        let mut explicit = Some(json!({ "model": "kimi-k2", "task": "t" }));
        inherit_model(
            &mut explicit,
            Some(CallerModel {
                model: "claude-opus-4-8",
                provider: None,
                functions: None,
            }),
        );
        assert_eq!(explicit.as_ref().unwrap()["model"], json!("kimi-k2"));

        let mut empty = Some(json!({ "model": "", "task": "t" }));
        inherit_model(
            &mut empty,
            Some(CallerModel {
                model: "claude-opus-4-8",
                provider: None,
                functions: None,
            }),
        );
        assert_eq!(empty.as_ref().unwrap()["model"], json!("claude-opus-4-8"));
    }

    #[test]
    fn inherited_model_carries_the_caller_provider() {
        // A caller pinned to a provider must not have its reaction route the
        // same model id through a different provider.
        let mut md = Some(json!({ "task": "t" }));
        inherit_model(
            &mut md,
            Some(CallerModel {
                model: "m",
                provider: Some("anthropic"),
                functions: None,
            }),
        );
        assert_eq!(md.as_ref().unwrap()["model"], json!("m"));
        assert_eq!(md.as_ref().unwrap()["provider"], json!("anthropic"));

        // An explicit spec provider wins over the caller's.
        let mut pinned = Some(json!({ "task": "t", "provider": "openai" }));
        inherit_model(
            &mut pinned,
            Some(CallerModel {
                model: "m",
                provider: Some("anthropic"),
                functions: None,
            }),
        );
        assert_eq!(pinned.as_ref().unwrap()["provider"], json!("openai"));

        // An explicit model means nothing is inherited — not even the provider.
        let mut explicit = Some(json!({ "task": "t", "model": "kimi-k2" }));
        inherit_model(
            &mut explicit,
            Some(CallerModel {
                model: "m",
                provider: Some("anthropic"),
                functions: None,
            }),
        );
        assert!(explicit.as_ref().unwrap().get("provider").is_none());
    }

    #[test]
    fn mistyped_model_values_do_not_inherit() {
        // Present-but-invalid values must keep failing spec validation loudly
        // instead of silently running on the caller's model.
        for bad in [json!(null), json!(false), json!(0), json!({})] {
            let mut md = Some(json!({ "task": "t", "model": bad }));
            inherit_model(
                &mut md,
                Some(CallerModel {
                    model: "m",
                    provider: None,
                    functions: None,
                }),
            );
            assert_eq!(md.as_ref().unwrap()["model"], json!(bad), "{bad}");
            assert!(crate::functions::react::validate_spec(md.as_ref()).is_err());
        }
    }

    #[test]
    fn dedup_key_ignores_the_inherited_model() {
        // The same raw registration from turns on different models must produce
        // the SAME dedup key — handle_react computes it before inherit_model
        // stamps, so a model switch cannot wire a twin binding.
        let raw = json!({
            "trigger_type": "harness::turn-completed",
            "config": { "session_id": "child-1" },
            "function_id": crate::functions::react::REACT_ID,
            "metadata": { "task": "t" },
        });
        let req: SubscribeRequest = serde_json::from_value(raw).unwrap();
        let key_raw = registration_dedup_key(&req, react_once(&req));

        let mut stamped = req.clone();
        inherit_model(
            &mut stamped.metadata,
            Some(CallerModel {
                model: "claude-opus-4-8",
                provider: Some("anthropic"),
                functions: None,
            }),
        );
        let key_stamped_input = registration_dedup_key(&stamped, react_once(&stamped));

        assert_ne!(
            key_raw, key_stamped_input,
            "sanity: the stamp does change the metadata"
        );
        // The behavioral guarantee lives in handle_react's ordering (dedup key
        // from `req` BEFORE inherit_model) — this pins the raw key's stability.
        let req2: SubscribeRequest = serde_json::from_value(json!({
            "trigger_type": "harness::turn-completed",
            "config": { "session_id": "child-1" },
            "function_id": crate::functions::react::REACT_ID,
            "metadata": { "task": "t" },
        }))
        .unwrap();
        assert_eq!(key_raw, registration_dedup_key(&req2, react_once(&req2)));
    }

    #[test]
    fn react_target_allows_turn_events_and_external_types_only() {
        assert!(react_target_type_allowed(crate::events::TURN_COMPLETED));
        assert!(react_target_type_allowed(crate::events::TURN_STARTED));
        assert!(react_target_type_allowed("state"));
        assert!(react_target_type_allowed("cron"));
        assert!(!react_target_type_allowed("harness::hook::pre-generate"));
        assert!(!react_target_type_allowed("harness::notify_agent"));
    }

    #[test]
    fn same_event_join_sibling_matches_only_same_source_distinct_key() {
        let info = |ty: &str, cfg: serde_json::Value, jid: &str, jkey: &str| {
            json!({
                "trigger_type": ty,
                "config": cfg,
                "metadata": { "join": { "id": jid, "key": jkey, "expect": ["a","b"] } }
            })
        };
        let cfg = json!({ "scope": "probe", "key": "p1" });
        // Same join, same source, different key: the instant-complete miswire.
        let hit = info("state", cfg.clone(), "J", "a");
        assert_eq!(
            same_event_join_sibling(&hit, "state", &cfg, "J", "b").as_deref(),
            Some("a")
        );
        // Same key (the registration itself / a dup) is not a sibling.
        assert!(same_event_join_sibling(&hit, "state", &cfg, "J", "a").is_none());
        // Different join id, different config, or different type: no warning.
        assert!(same_event_join_sibling(&hit, "state", &cfg, "K", "b").is_none());
        assert!(same_event_join_sibling(
            &hit,
            "state",
            &json!({ "scope": "probe", "key": "p2" }),
            "J",
            "b"
        )
        .is_none());
        assert!(same_event_join_sibling(&hit, "cron", &cfg, "J", "b").is_none());
        // Non-join bindings never match.
        assert!(same_event_join_sibling(
            &json!({ "trigger_type": "state", "config": cfg, "metadata": {} }),
            "state",
            &cfg,
            "J",
            "b"
        )
        .is_none());
    }

    #[test]
    fn turn_event_session_filter_gates_on_type_and_config() {
        let mk = |ty: &str, cfg: serde_json::Value| -> SubscribeRequest {
            serde_json::from_value(json!({ "trigger_type": ty, "config": cfg })).unwrap()
        };
        // Turn-event types with a session filter are the starvation-prone shape.
        let r = mk(
            crate::events::TURN_COMPLETED,
            json!({ "session_id": "s_x" }),
        );
        assert_eq!(turn_event_session_filter(&r), Some("s_x"));
        let r = mk(crate::events::TURN_STARTED, json!({ "session_id": "s_x" }));
        assert_eq!(turn_event_session_filter(&r), Some("s_x"));
        // Other filters and other types are not advised on.
        let r = mk(
            crate::events::TURN_COMPLETED,
            json!({ "parent_session_id": "s_x" }),
        );
        assert_eq!(turn_event_session_filter(&r), None);
        let r = mk("state", json!({ "session_id": "s_x" }));
        assert_eq!(turn_event_session_filter(&r), None);
    }

    #[test]
    fn parent_filter_join_advisory_flags_the_starving_shape() {
        let mk = |config: serde_json::Value, join: serde_json::Value| -> SubscribeRequest {
            serde_json::from_value(json!({
                "trigger_type": crate::events::TURN_COMPLETED,
                "config": config,
                "function_id": "harness::react",
                "metadata": { "model": "m", "task": "t", "join": join },
            }))
            .unwrap()
        };
        let join2 = json!({ "id": "J", "expect": ["a", "b"], "key": "a" });
        // The live miswire: one parent-filtered binding for a 2-key join.
        let note =
            parent_filter_join_advisory(&mk(json!({ "parent_session_id": "s_p" }), join2.clone()));
        assert!(note.as_deref().unwrap_or("").contains("starves"));
        // session_id narrows to one child: fine.
        assert!(parent_filter_join_advisory(&mk(
            json!({ "parent_session_id": "s_p", "session_id": "s_c" }),
            join2.clone()
        ))
        .is_none());
        // Single-key join on a parent filter is a legitimate any-child watcher.
        assert!(parent_filter_join_advisory(&mk(
            json!({ "parent_session_id": "s_p" }),
            json!({ "id": "J", "expect": ["a"], "key": "a" })
        ))
        .is_none());
        // No join, or no parent filter: silent.
        let mut no_join = mk(json!({ "parent_session_id": "s_p" }), join2.clone());
        no_join.metadata = Some(json!({ "model": "m", "task": "t" }));
        assert!(parent_filter_join_advisory(&no_join).is_none());
        assert!(parent_filter_join_advisory(&mk(json!({}), join2.clone())).is_none());
        // Non-turn trigger types are not advised on.
        let mut state_req = mk(json!({ "parent_session_id": "s_p" }), join2);
        state_req.trigger_type = "state".into();
        assert!(parent_filter_join_advisory(&state_req).is_none());
    }

    /// The rctest-k7m3 miswire: a key-less state binding beside a keyed one
    /// fires for every key in the scope and burns the fire-rate budget twice.
    /// The registrant-policy stamp: written from the trusted caller, never
    /// from the agent's own metadata (a smuggled value is dropped even when
    /// there is nothing to stamp).
    #[test]
    fn stamp_registrant_functions_writes_trusted_policy_and_strips_smuggled() {
        let policy = crate::types::turn::FunctionPolicy {
            allow: vec!["database::query".into()],
            deny: vec![],
            expose: Default::default(),
        };
        let caller = CallerModel {
            model: "m",
            provider: None,
            functions: Some(&policy),
        };

        // Genuine stamp from the caller.
        let mut md = Some(json!({ "model": "m", "task": "t" }));
        stamp_registrant_functions(&mut md, Some(caller));
        assert_eq!(
            md.as_ref().unwrap()[REGISTRANT_FUNCTIONS_KEY]["allow"],
            json!(["database::query"])
        );

        // A smuggled stamp is replaced by the trusted one...
        let mut md = Some(json!({
            "model": "m", "task": "t",
            REGISTRANT_FUNCTIONS_KEY: { "allow": ["*"] }
        }));
        stamp_registrant_functions(&mut md, Some(caller));
        assert_eq!(
            md.as_ref().unwrap()[REGISTRANT_FUNCTIONS_KEY]["allow"],
            json!(["database::query"])
        );

        // ...and dropped outright when there is no caller policy.
        let mut md = Some(json!({
            "model": "m", "task": "t",
            REGISTRANT_FUNCTIONS_KEY: { "allow": ["*"] }
        }));
        stamp_registrant_functions(&mut md, None);
        assert!(md.as_ref().unwrap().get(REGISTRANT_FUNCTIONS_KEY).is_none());
    }

    #[test]
    fn state_catchall_advisory_flags_keyless_state_bindings() {
        let mk = |ty: &str, config: serde_json::Value| -> SubscribeRequest {
            serde_json::from_value(json!({
                "trigger_type": ty,
                "config": config,
                "function_id": "harness::react",
                "metadata": { "model": "m", "task": "t" },
            }))
            .unwrap()
        };
        // Scope without key: warned, naming the scope.
        let note = state_catchall_advisory(&mk("state", json!({ "scope": "run-1" })));
        assert!(note.as_deref().unwrap_or("").contains("run-1"));
        assert!(note.as_deref().unwrap_or("").contains("no `key` filter"));
        // No scope AND no key: the global catch-all, warned harder.
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
    fn standing_binding_advisory_notes_non_once_non_join_only() {
        let mk = |ty: &str, join: bool| -> SubscribeRequest {
            let mut metadata = json!({ "model": "m", "task": "t" });
            if join {
                metadata["join"] = json!({ "id": "J", "expect": ["a", "b"], "key": "a" });
            }
            serde_json::from_value(json!({
                "trigger_type": ty,
                "config": {},
                "function_id": "harness::react",
                "metadata": metadata,
            }))
            .unwrap()
        };
        // The leak shape: a state kickoff registered without once.
        let note = standing_binding_advisory(&mk("state", false), false);
        assert!(note.as_deref().unwrap_or("").contains("STANDING"));
        // once retires itself; joins own their lifecycle; cron is recurring by design.
        assert!(standing_binding_advisory(&mk("state", false), true).is_none());
        assert!(standing_binding_advisory(&mk("state", true), false).is_none());
        assert!(standing_binding_advisory(&mk("cron", false), false).is_none());
    }

    #[test]
    fn simple_reactions_default_once_while_cron_and_joins_stay_standing() {
        let mk = |ty: &str, once: Option<bool>, join: bool| -> SubscribeRequest {
            let mut metadata = json!({ "model": "m", "task": "t" });
            if join {
                metadata["join"] = json!({ "id": "J", "expect": ["a"], "key": "a" });
            }
            serde_json::from_value(json!({
                "trigger_type": ty,
                "config": { "scope": "s", "key": "k" },
                "function_id": "harness::react",
                "once": once,
                "metadata": metadata,
            }))
            .unwrap()
        };
        assert!(react_once(&mk("state", Some(true), false)));
        assert!(react_once(&mk("state", None, false)));
        // Explicit opt-out and naturally recurring cron bindings stay standing.
        assert!(!react_once(&mk("state", Some(false), false)));
        assert!(!react_once(&mk("cron", None, false)));
        // The join owns its predecessors' lifecycle.
        assert!(!react_once(&mk("state", Some(true), true)));
    }

    #[test]
    fn subscribe_request_accepts_react_target_fields() {
        let req: SubscribeRequest = serde_json::from_value(json!({
            "trigger_type": "harness::turn-completed",
            "config": { "session_id": "s_child" },
            "function_id": "harness::react",
            "metadata": { "model": "m", "task": "t" },
        }))
        .expect("react-shaped register args must parse");
        assert_eq!(req.function_id.as_deref(), Some("harness::react"));
        assert!(req.metadata.is_some());
    }

    #[test]
    fn register_request_stamps_trusted_target_and_session() {
        let req: SubscribeRequest = serde_json::from_value(json!({
            "trigger_type": "state",
            "config": { "scope": "job", "key": "42" },
            "label": "done",
            "once": false,
            "function_id": "evil::handler",
            "metadata": { "session_id": "s_attacker" }
        }))
        .unwrap();

        let request = register_trigger_request(&req, "sub_trusted", "s_trusted", false, 123);

        assert_eq!(request.function_id, REGISTER_TRIGGER_ID);
        assert_eq!(request.timeout_ms, Some(123));
        assert_eq!(request.payload["trigger_type"], "state");
        assert_eq!(request.payload["function_id"], NOTIFY_AGENT_ID);
        assert_eq!(
            request.payload["config"],
            json!({ "scope": "job", "key": "42" })
        );
        assert_eq!(
            request.payload["metadata"]["subscription_id"],
            "sub_trusted"
        );
        assert_eq!(request.payload["metadata"]["session_id"], "s_trusted");
        assert_eq!(request.payload["metadata"]["label"], "done");
        assert_eq!(request.payload["metadata"]["once"], false);
    }

    #[test]
    fn once_defaults_to_recurring_only_for_cron() {
        let state: SubscribeRequest =
            serde_json::from_value(json!({ "trigger_type": "state" })).unwrap();
        let cron: SubscribeRequest =
            serde_json::from_value(json!({ "trigger_type": "cron" })).unwrap();
        let explicit: SubscribeRequest =
            serde_json::from_value(json!({ "trigger_type": "cron", "once": true })).unwrap();

        assert!(effective_once(&state));
        assert!(!effective_once(&cron));
        assert!(effective_once(&explicit));
    }

    #[test]
    fn unregister_requires_string_subscription_id() {
        assert_eq!(
            unregister_subscription_id(&json!({})).unwrap_err(),
            "engine::unregister_trigger requires an `id`"
        );
        assert_eq!(
            unregister_subscription_id(&json!({ "id": 42 })).unwrap_err(),
            "engine::unregister_trigger requires an `id`"
        );
        assert_eq!(
            unregister_subscription_id(&json!({ "id": "sub_1" })).unwrap(),
            "sub_1"
        );
    }
}

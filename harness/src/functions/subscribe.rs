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
    /// recurring `cron`. Ignored when `function_id` targets `harness::react`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub once: Option<bool>,
    /// Target function fired on each event. Omit for a notification message
    /// into this session. The ONLY explicit target allowed is `harness::react`
    /// (spawn a sub-agent from the event) — pass the reaction spec in
    /// `metadata`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub function_id: Option<String>,
    /// `harness::react` reaction spec: `{ model, task, session_id?,
    /// parent_session_id?, options?, join? }`. Required (with `model` + `task`)
    /// when `function_id` is `harness::react`; forwarded verbatim. A
    /// trigger-fired sub-agent starts with only the read-only default policy —
    /// grant what the reaction needs via `options` (same shape as
    /// `harness::spawn` options, e.g. `{ "functions": { "allow":
    /// ["state::get"] } }`). Join predecessors auto-unregister after the join
    /// fires unless `join.rearm: true` keeps them registered for the next
    /// complete set.
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
    let is_turn_event = req.trigger_type == crate::events::TURN_STARTED
        || req.trigger_type == crate::events::TURN_COMPLETED;
    if !is_turn_event {
        return None;
    }
    req.config.get("session_id").and_then(Value::as_str)
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
) -> ResultData {
    match function_id {
        REGISTER_TRIGGER_ID => intercept_register(deps, arguments, session_id).await,
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

async fn intercept_register(deps: &Deps, args: &Value, session_id: &str) -> ResultData {
    let req: SubscribeRequest = match serde_json::from_value(args.clone()) {
        Ok(r) => r,
        Err(e) => return error_result(format!("invalid subscribe arguments: {e}")),
    };

    match handle(deps, req, session_id).await {
        Ok(resp) => ok_result(&resp),
        Err(e) => error_result(e.to_string()),
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
) -> Result<SubscribeResponse, HarnessError> {
    if let Some(fid) = req.function_id.as_deref() {
        if fid != crate::functions::react::REACT_ID {
            return Err(HarnessError::InvalidRequest(format!(
                "only `{}` may be a subscription target (got `{fid}`); omit `function_id` to be notified in this session instead",
                crate::functions::react::REACT_ID
            )));
        }
        return handle_react(deps, req, session_id).await;
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

    deps.subscriptions
        .try_insert_keyed(
            &sub_id,
            session_id,
            subscriptions::MAX_SUBSCRIPTIONS_PER_SESSION,
            dedup,
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

    match resp
        .ok()
        .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
    {
        Some(trigger_id) => {
            if !deps.subscriptions.set_trigger_id(&sub_id, &trigger_id) {
                unregister_engine_trigger(deps, &trigger_id).await;
            }
        }
        None => {
            deps.subscriptions.take(&sub_id);
            return Err(HarnessError::Dependency(format!(
                "{REGISTER_TRIGGER_ID} `{}` failed",
                req.trigger_type
            )));
        }
    }

    Ok(SubscribeResponse {
        subscription_id: sub_id,
        once,
        note: session_filter_advisory(deps, &req).await,
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
    req: SubscribeRequest,
    session_id: &str,
) -> Result<SubscribeResponse, HarnessError> {
    if !react_target_type_allowed(&req.trigger_type) {
        return Err(HarnessError::InvalidRequest(format!(
            "cannot bind harness-internal trigger type `{}` to `harness::react`",
            req.trigger_type
        )));
    }

    crate::functions::react::validate_spec(req.metadata.as_ref())
        .map_err(HarnessError::InvalidRequest)?;
    crate::functions::react::validate_model(&deps.iii, req.metadata.as_ref())
        .await
        .map_err(HarnessError::InvalidRequest)?;

    // Idempotency: same rule as the notify path — an identical re-registration
    // returns the standing subscription instead of a twin reaction that would
    // double-spawn on every fire. Keyed on the raw request (pre-owner-stamp).
    let dedup = registration_dedup_key(&req, false);
    if let Some(existing) = deps.subscriptions.find_duplicate(session_id, &dedup) {
        return Ok(SubscribeResponse {
            subscription_id: existing,
            once: false,
            note: None,
        });
    }

    let sub_id = format!("sub_{}", uuid::Uuid::new_v4().simple());
    deps.subscriptions
        .try_insert_keyed(
            &sub_id,
            session_id,
            subscriptions::MAX_SUBSCRIPTIONS_PER_SESSION,
            dedup,
        )
        .map_err(|_| {
            HarnessError::InvalidRequest(format!(
                "subscription cap reached ({} active for this session); unsubscribe first",
                subscriptions::MAX_SUBSCRIPTIONS_PER_SESSION
            ))
        })?;

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

    match resp
        .ok()
        .and_then(|v| v.get("id").and_then(Value::as_str).map(str::to_string))
    {
        Some(trigger_id) => {
            if !deps.subscriptions.set_trigger_id(&sub_id, &trigger_id) {
                unregister_engine_trigger(deps, &trigger_id).await;
            }
        }
        None => {
            deps.subscriptions.take(&sub_id);
            return Err(HarnessError::Dependency(format!(
                "{REGISTER_TRIGGER_ID} `{}` failed",
                req.trigger_type
            )));
        }
    }

    let note = match (
        session_filter_advisory(deps, &req).await,
        join_wiring_advisory(deps, &req).await,
    ) {
        (Some(a), Some(b)) => Some(format!("{a} {b}")),
        (a, b) => a.or(b),
    };
    Ok(SubscribeResponse {
        subscription_id: sub_id,
        once: false,
        note,
    })
}

pub async fn unregister_engine_trigger(deps: &Deps, trigger_id: &str) {
    if let Err(e) = deps
        .iii
        .trigger(TriggerRequest {
            function_id: UNREGISTER_TRIGGER_ID.to_string(),
            payload: json!({ "id": trigger_id }),
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        })
        .await
    {
        tracing::warn!(trigger_id, error = %e, "subscription trigger unregister failed");
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

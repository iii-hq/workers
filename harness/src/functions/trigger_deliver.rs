//! `harness::trigger::deliver` — the ONE fire handler for every agent-registered
//! binding (architecture/trigger-bindings.md).
//!
//! The engine hands back the trigger's stored metadata at fire time; ours holds
//! a single key, `__binding`. Everything else — owner, target, conditions,
//! lifecycle, frozen capability — is read from the durable record, so nothing an
//! agent can write into metadata is ever trusted or even read.
//!
//! Order is load-bearing:
//!
//! ```text
//! resolve → stale-target check → declared conditions → claim → project → dispatch → record
//! ```
//!
//! The claim (the persisted fire count) lands BEFORE dispatch: a redelivered
//! fire finds the budget already spent and stops. Retirement lands AFTER, so a
//! crash mid-dispatch loses nothing that was not already claimed. Dispatch has
//! exactly two shapes — wake the owner, or call a plain function — and never
//! creates an agent.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::RegisterFunction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::bindings::Binding;
use crate::conditions::{self, Skip};
use crate::deps::Deps;
use crate::policy::CompiledPolicy;
use crate::subscriptions::fired;
use crate::surface::schema_value;
use crate::types::message::AgentMessage;

pub const DELIVER_ID: &str = "harness::trigger::deliver";
pub const DELIVER_DESC: &str =
    "Internal fire handler for a harness-registered trigger binding: evaluates the binding's \
     conditions, projects the event into the target's payload, and dispatches it (a wake into \
     the owner session, or a plain function call). Never called directly — register bindings \
     with engine::register_trigger.";

/// The metadata the engine stores on the binding. A pointer and nothing else.
#[derive(Debug, Deserialize)]
struct DeliverMetadata {
    #[serde(rename = "__binding")]
    binding: String,
}

/// Fired-event payload. Trigger events are not always objects, so this keeps a
/// lossless `Value` while publishing a real schema (the registry publish gate
/// rejects `AnyValue`).
#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct DeliverEvent(pub Value);

impl JsonSchema for DeliverEvent {
    fn schema_name() -> String {
        "DeliverEvent".to_string()
    }

    fn json_schema(_: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        use schemars::schema::{InstanceType, Metadata, SchemaObject};
        SchemaObject {
            instance_type: Some(
                vec![
                    InstanceType::Null,
                    InstanceType::Boolean,
                    InstanceType::Number,
                    InstanceType::String,
                    InstanceType::Array,
                    InstanceType::Object,
                ]
                .into(),
            ),
            metadata: Some(Box::new(Metadata {
                description: Some(
                    "Arbitrary fired-event payload from the subscribed trigger.".to_string(),
                ),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct DeliverResult {
    /// Whether the target was dispatched this fire.
    pub delivered: bool,
    /// Which gate or condition stopped it. Present iff `!delivered`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gate: Option<String>,
    /// Why. Present iff `!delivered`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl DeliverResult {
    fn delivered() -> Self {
        Self {
            delivered: true,
            gate: None,
            note: None,
        }
    }
    fn stopped(gate: impl Into<String>, note: impl Into<String>) -> Self {
        Self {
            delivered: false,
            gate: Some(gate.into()),
            note: Some(note.into()),
        }
    }
}

pub fn register(deps: Arc<Deps>) {
    let iii = deps.iii.clone();
    iii.register_function(
        DELIVER_ID,
        RegisterFunction::new_async(move |event: Value, metadata: Option<Value>| {
            let deps = deps.clone();
            async move {
                let out = handle(&deps, event, metadata)
                    .await
                    .unwrap_or_else(|e| DeliverResult::stopped("harness", e.to_string()));
                Ok::<Value, Error>(json!(out))
            }
        })
        .description(DELIVER_DESC)
        .request_format(schema_value::<DeliverEvent>())
        .response_format(schema_value::<DeliverResult>())
        .metadata(json!({ "internal": true })),
    );
}

/// A binding fired. Never errors: an erroring trigger target only spams the
/// engine's dispatch log, so every failure becomes a recorded non-delivery.
pub async fn handle(
    deps: &Deps,
    event: Value,
    metadata: Option<Value>,
) -> Result<DeliverResult, crate::error::HarnessError> {
    let Some(meta) = metadata.and_then(|m| serde_json::from_value::<DeliverMetadata>(m).ok())
    else {
        tracing::warn!("{DELIVER_ID}: fire without a `__binding` key; dropping");
        return Ok(DeliverResult::stopped(
            "metadata",
            "no __binding in trigger metadata",
        ));
    };

    let store = deps.bindings().await;
    let binding = match store.get(&meta.binding).await {
        Ok(Some(b)) => b,
        Ok(None) => {
            // The record is the authority: no record means the binding was
            // retired (or predates this hop). Tear the engine side down so it
            // stops firing into nothing.
            tracing::info!(binding = %meta.binding, "fire for an unknown binding; unregistering");
            return Ok(DeliverResult::stopped(
                "unknown-binding",
                "no binding record",
            ));
        }
        Err(e) => {
            tracing::warn!(binding = %meta.binding, error = %e, "binding lookup failed; dropping fire");
            return Ok(DeliverResult::stopped(
                "store",
                format!("binding lookup failed: {e}"),
            ));
        }
    };

    // A record that predates the spawn-target removal must never dispatch:
    // retire it on both sides and tell the owner what to register instead.
    // The startup sweep mass-retires these; this is the race backstop for a
    // fire already in flight.
    if binding.target.function_id == crate::functions::SPAWN_ID {
        crate::bindings::gc::retire_stale_spawn_binding(deps, &binding).await;
        return Ok(DeliverResult::stopped(
            "stale-spawn-target",
            "spawn bindings were removed; the binding was retired and its owner notified",
        ));
    }

    let now = AgentMessage::now_ms();
    if binding.is_exhausted(now) {
        retire(deps, &binding).await;
        return Ok(record_stop(
            deps,
            &binding,
            &event,
            Skip {
                gate: "lifecycle",
                reason: "binding already spent".into(),
                retire: true,
            },
        )
        .await);
    }

    let event = match conditions::evaluate(deps, &binding, event).await {
        Ok(event) => event,
        Err(skip) => {
            // A skipped fire does NOT consume the lifecycle: a barrier that
            // answers "not yet" ten times still gets its one delivery.
            let e = json!({ "skipped": skip.reason });
            return Ok(record_stop(deps, &binding, &e, skip).await);
        }
    };

    // Claim before dispatch, atomically. The incremented count is what makes a
    // redelivered fire find the budget spent — and the compare-and-set is what
    // stops two SIMULTANEOUS fires taking the same ordinal, which lost a
    // delivery and let a bounded lifecycle over-spend.
    let claimed = match store.claim_fire(&binding).await {
        Ok(crate::bindings::ClaimOutcome::Claimed(b)) => *b,
        Ok(crate::bindings::ClaimOutcome::Exhausted) => {
            retire(deps, &binding).await;
            return Ok(record_stop(
                deps,
                &binding,
                &event,
                Skip {
                    gate: "lifecycle",
                    reason: "another fire spent the last of the budget".into(),
                    retire: true,
                },
            )
            .await);
        }
        Ok(crate::bindings::ClaimOutcome::Gone) => {
            return Ok(DeliverResult::stopped(
                "unknown-binding",
                "binding retired while this fire was in flight",
            ));
        }
        Err(e) => {
            tracing::warn!(binding = %binding.id, error = %e, "claim failed; dropping fire");
            return Ok(DeliverResult::stopped(
                "store",
                format!("claim failed: {e}"),
            ));
        }
    };
    let fires_after = claimed.fires;
    let retiring = binding.retires_after_fire(fires_after);

    let outcome = dispatch(deps, &claimed, &event).await;

    if retiring {
        retire(deps, &claimed).await;
    }

    let note = outcome
        .as_ref()
        .err()
        .map(|e| format!("dispatch failed: {e}"));
    fired::emit(
        &deps.session().await,
        &binding.owner.session_id,
        &record_entry_id(&binding.id, fires_after),
        fired::TriggerFired {
            subscription_id: &binding.id,
            trigger_id: binding.trigger_id.as_deref(),
            target: &binding.target.function_id,
            label: None,
            once: binding.lifecycle.once,
            retired: retiring,
            scope: fired::event_state_watch(&event).0,
            key: fired::event_state_watch(&event).1,
            note: note.as_deref(),
            fired_at: now,
        },
    )
    .await;

    Ok(match outcome {
        Ok(()) => DeliverResult::delivered(),
        Err(e) => DeliverResult::stopped("dispatch", e),
    })
}

/// Dispatch the projected event to the binding's target: a wake into the
/// owner session, or a plain function call. The selection depends ONLY on the
/// target function id — never on the event's source or shape, which is what
/// keeps delivery generic across state, database, queue, cron, timer, and
/// trigger types that do not exist yet.
async fn dispatch(deps: &Deps, binding: &Binding, event: &Value) -> Result<(), String> {
    match binding.target.function_id.as_str() {
        crate::functions::SEND_ID => wake_target(deps, binding, event).await,
        other => call_target(deps, binding, other, project(binding, event)).await,
    }
}

/// The target's payload template with the fired event injected at its pointer.
fn project(binding: &Binding, event: &Value) -> Value {
    let base = binding.target.payload.clone().unwrap_or_else(|| json!({}));
    inject_at(base, binding.target.event_pointer(), event.clone())
}

/// Set `value` at `pointer` inside `payload`, creating intermediate objects.
/// A pointer that cannot be created leaves the payload untouched — the target
/// still gets its template rather than nothing.
fn inject_at(payload: Value, pointer: &str, value: Value) -> Value {
    if pointer.is_empty() {
        return value;
    }
    let mut root = match payload {
        Value::Object(_) => payload,
        other => json!({ "_payload": other }),
    };
    let segments: Vec<String> = pointer
        .trim_start_matches('/')
        .split('/')
        .map(|s| s.replace("~1", "/").replace("~0", "~"))
        .collect();
    let mut cursor = &mut root;
    for segment in &segments[..segments.len() - 1] {
        let map = match cursor.as_object_mut() {
            Some(m) => m,
            None => return root,
        };
        cursor = map.entry(segment.clone()).or_insert_with(|| json!({}));
    }
    if let Some(map) = cursor.as_object_mut() {
        map.insert(segments[segments.len() - 1].clone(), value);
    }
    root
}

/// A wake: the event becomes a message in the owner's session. Routed through
/// `send::inject` rather than an engine call to `harness::send` so it keeps the
/// notification origin marker (clients must not render it as human-typed) and a
/// deterministic entry id (a redelivered fire appends nothing new).
async fn wake_target(deps: &Deps, binding: &Binding, event: &Value) -> Result<(), String> {
    let session_id = binding
        .target
        .payload
        .as_ref()
        .and_then(|p| p.get("session_id"))
        .and_then(Value::as_str)
        .unwrap_or(&binding.owner.session_id)
        .to_string();
    let message = AgentMessage::user_text(notification_text(binding, event));
    let entry_id = fire_entry_id(&binding.id, binding.fires);
    crate::functions::send::inject(
        deps,
        &session_id,
        message,
        Some(&entry_id),
        Some(&json!({ "notification": true, "binding": binding.id })),
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// The frozen-capability gate for a call dispatch, factored pure for testing:
/// the target is checked against what the registrant could call WHEN IT
/// REGISTERED — never the owner's policy now, which may have widened since.
fn call_allowed(binding: &Binding, target: &str) -> Result<(), String> {
    if !CompiledPolicy::from(binding.capability.as_ref()).allows(target) {
        return Err(format!(
            "`{target}` is outside the policy this binding was registered with"
        ));
    }
    Ok(())
}

/// A mechanical reaction: any ordinary function, checked against the
/// capability the binding froze at registration.
async fn call_target(
    deps: &Deps,
    binding: &Binding,
    target: &str,
    payload: Value,
) -> Result<(), String> {
    call_allowed(binding, target)?;
    // AWAITED, not fire-and-forget. A void dispatch reports success the moment
    // the engine accepts it, so a target that then fails — a bad statement, a
    // rejected payload — is recorded as delivered and "why did nothing
    // happen?" becomes unanswerable from the timeline. The dispatch timeout
    // bounds the wait.
    let timeout_ms = deps.cfg().await.dispatch_timeout_ms;
    deps.iii
        .trigger(TriggerRequest {
            function_id: target.to_string(),
            payload,
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn notification_text(binding: &Binding, event: &Value) -> String {
    const MAX: usize = 600;
    let rendered = match event {
        Value::Null => "event fired".to_string(),
        Value::String(s) => s.clone(),
        other => serde_json::to_string(other).unwrap_or_else(|_| "event fired".to_string()),
    };
    let summary = if rendered.chars().count() > MAX {
        let mut s: String = rendered.chars().take(MAX).collect();
        s.push_str(" …(truncated)");
        s
    } else {
        rendered
    };
    let _ = binding;
    format!("[notification] {summary}")
}

/// Deterministic per-fire transcript id: a redelivered fire with the same
/// ordinal appends nothing new.
fn fire_entry_id(binding_id: &str, ordinal: u64) -> String {
    format!("e_fire_{binding_id}_{ordinal}")
}

/// The delivery RECORD's id for the same fire. Distinct from [`fire_entry_id`]
/// by construction, and the distinction is load-bearing: session-manager is
/// idempotent on entry ids, so when the wake message and its record shared one
/// id, exactly one of the two appends survived per fire. Worse, WHICH one
/// depended on timing — an idle session appended the wake first and the record
/// was swallowed; a running turn PARKED the wake while the record appended
/// immediately, and the drained wake was then deduped as a replay. A burst of
/// three fires surfaced as one notification and two records.
fn record_entry_id(binding_id: &str, ordinal: u64) -> String {
    format!("e_trigfired_{binding_id}_{ordinal}")
}

/// Tear a binding down on both sides — engine first, then the record, so a
/// failure between them leaves a binding that still resolves rather than one
/// that fires into nothing.
async fn retire(deps: &Deps, binding: &Binding) {
    if let Some(trigger_id) = binding.trigger_id.as_deref() {
        crate::functions::subscribe::unregister_engine_trigger(deps, trigger_id).await;
    }
    if let Err(e) = deps.bindings().await.delete(&binding.id).await {
        tracing::warn!(binding = %binding.id, error = %e, "binding record delete failed");
    }
}

/// Record a non-delivery in the owner's timeline. This is the half today's
/// bookkeeping is missing: a binding that never fires and a binding that fires
/// and skips look identical from the outside, which is why a mis-wired
/// condition is so expensive to debug.
async fn record_stop(deps: &Deps, binding: &Binding, event: &Value, skip: Skip) -> DeliverResult {
    let (scope, key) = fired::event_state_watch(event);
    fired::emit(
        &deps.session().await,
        &binding.owner.session_id,
        &format!(
            "{}_{}",
            fire_entry_id(&binding.id, binding.fires),
            skip.gate
        ),
        fired::TriggerFired {
            subscription_id: &binding.id,
            trigger_id: binding.trigger_id.as_deref(),
            target: &binding.target.function_id,
            label: None,
            once: binding.lifecycle.once,
            retired: skip.retire,
            scope,
            key,
            note: Some(&skip.reason),
            fired_at: AgentMessage::now_ms(),
        },
    )
    .await;
    DeliverResult::stopped(skip.gate, skip.reason)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injection_creates_missing_objects_along_the_pointer() {
        let out = inject_at(json!({ "db": "primary" }), "/args/event", json!({ "n": 1 }));
        assert_eq!(out["db"], json!("primary"));
        assert_eq!(out["args"]["event"], json!({ "n": 1 }));
    }

    #[test]
    fn an_empty_pointer_replaces_the_whole_payload() {
        let out = inject_at(json!({ "ignored": true }), "", json!([1, 2]));
        assert_eq!(out, json!([1, 2]));
    }

    #[test]
    fn a_non_object_template_is_preserved_under_a_key() {
        // Nothing can be injected INTO a scalar; keeping the template under a
        // known key beats dropping either side silently.
        let out = inject_at(json!("literal"), "/event", json!({ "n": 1 }));
        assert_eq!(out["_payload"], json!("literal"));
        assert_eq!(out["event"], json!({ "n": 1 }));
    }

    #[test]
    fn escaped_pointer_segments_round_trip() {
        let out = inject_at(json!({}), "/a~1b/c", json!(1));
        assert_eq!(out["a/b"]["c"], json!(1));
    }

    #[test]
    fn fire_entry_ids_are_stable_per_ordinal() {
        assert_eq!(fire_entry_id("sub_1", 2), "e_fire_sub_1_2");
        assert_ne!(fire_entry_id("sub_1", 2), fire_entry_id("sub_1", 3));
    }

    #[test]
    fn notification_text_truncates_a_huge_event() {
        use crate::bindings::{BindingTarget, Causation, Lifecycle, OwnerScope};
        let b = Binding {
            id: "sub_1".into(),
            trigger_id: None,
            owner: OwnerScope {
                session_id: "s".into(),
                root_session_id: None,
            },
            target: BindingTarget::new("harness::send"),
            conditions: vec![],
            lifecycle: Lifecycle::default(),
            capability: None,
            causation: Causation::default(),
            dedup_key: None,
            fires: 0,
            created_at: 0,
        };
        let big = json!({ "blob": "x".repeat(5000) });
        let text = notification_text(&b, &big);
        assert!(text.starts_with("[notification] "));
        assert!(text.contains("…(truncated)"));
        assert!(text.chars().count() < 700);
    }

    #[test]
    fn the_delivery_record_never_shares_the_wake_entry_id() {
        // The regression this pins: both appends carried `e_fire_<b>_<n>`, and
        // session-manager's entry-id idempotence swallowed whichever landed
        // second — the record on an idle session, the WAKE when the turn was
        // running (the record claimed the id while the wake sat parked, and
        // the drain was then deduped as a replay). One binding, three fires,
        // one notification.
        for ordinal in [0, 1, 7] {
            let wake = fire_entry_id("sub_1", ordinal);
            let record = record_entry_id("sub_1", ordinal);
            assert_ne!(wake, record, "ordinal {ordinal}");
        }
        assert_eq!(record_entry_id("sub_1", 2), "e_trigfired_sub_1_2");
        // Redelivery of the same ordinal still dedupes against itself.
        assert_eq!(record_entry_id("sub_1", 2), record_entry_id("sub_1", 2));
    }

    fn wake_binding(trigger_type: &str) -> Binding {
        use crate::bindings::{BindingTarget, Causation, Lifecycle, OwnerScope};
        let mut target = BindingTarget::new(crate::functions::SEND_ID);
        target.payload = Some(json!({ "session_id": "s_owner", "label": null }));
        Binding {
            id: "sub_w".into(),
            trigger_id: None,
            owner: OwnerScope {
                session_id: "s_owner".into(),
                root_session_id: None,
            },
            target,
            conditions: vec![],
            lifecycle: Lifecycle {
                once: true,
                max_fires: None,
                expires_at: None,
            },
            capability: None,
            causation: Causation::default(),
            dedup_key: Some(json!({ "trigger_type": trigger_type, "config": {} })),
            fires: 0,
            created_at: 0,
        }
    }

    /// The medium-agnostic claim, pinned: nothing in the wake path reads the
    /// event's source or shape, so a trigger type that does not exist yet
    /// delivers through the identical owner-notification path as state,
    /// database, cron, and timer do today.
    #[test]
    fn a_wake_binding_delivers_any_source_shape() {
        let b = wake_binding("sensor::motion-detected");
        assert!(
            crate::bindings::is_armed_wake(&b, 0),
            "a once send-target binding is an armed wake whatever its source type"
        );
        let event = json!({ "sensor": "porch", "confidence": 0.93, "at": 1234 });
        let text = notification_text(&b, &event);
        assert!(text.starts_with("[notification] "));
        assert!(
            text.contains("porch"),
            "the arbitrary event renders: {text}"
        );
        // Non-object events deliver too — DeliverEvent is lossless.
        for odd in [json!("plain string"), json!(41.5), Value::Null] {
            assert!(notification_text(&b, &odd).starts_with("[notification] "));
        }
    }

    /// A call dispatch is checked against the capability frozen at
    /// registration — never the owner's policy now, and `None` (a registrant
    /// that ran with no policy) denies rather than passes.
    #[test]
    fn a_call_dispatch_is_checked_against_the_frozen_capability() {
        use crate::types::turn::FunctionPolicy;
        let mut b = wake_binding("state");
        b.target = crate::bindings::BindingTarget::new("run::record");
        b.capability = Some(FunctionPolicy {
            allow: vec!["run::*".into(), "state::get".into()],
            deny: vec![],
            expose: Default::default(),
        });
        assert!(call_allowed(&b, "run::record").is_ok());
        assert!(call_allowed(&b, "state::get").is_ok());
        let err = call_allowed(&b, "shell::run").unwrap_err();
        assert!(
            err.contains("outside the policy this binding was registered with"),
            "got: {err}"
        );

        b.capability = None;
        assert!(
            call_allowed(&b, "run::record").is_err(),
            "no frozen capability must deny, not pass"
        );
    }
}

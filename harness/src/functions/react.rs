//! `harness::react` — the reactive-subscription bridge (harness.md § Triggers).
//!
//! A trigger delivers the event type's OWN payload to its bound function; a
//! `harness::turn-completed` or `state` event carries no `task`/`model`, so it
//! cannot be bound straight to `harness::spawn`. This function is the shaping
//! hop: the agent binds an event to `harness::react` via `engine::register_trigger`
//! and puts the sub-agent it wants in the trigger's `metadata` (a [`ReactSpec`]).
//! When the event fires, the engine calls this function with the event as the
//! payload and the spec as the metadata sidecar; we reshape the two into a
//! `harness::spawn` so the reaction runs as a SUB-AGENT.
//!
//! Two modes, selected by the spec:
//!   * Simple edge (no `join`): one event → spawn one sub-agent, event appended.
//!   * Join edge (`join` set): the emergent-DAG barrier. Every predecessor of a
//!     join binds its own subscription carrying the SAME `join.id` + `expect`
//!     set and its own `key`. Each firing merges its result into a durable
//!     accumulator in iii-state; the downstream sub-agent spawns EXACTLY ONCE,
//!     when the last predecessor arrives, fed ALL predecessors' results. The
//!     fire-once guard is an atomic `state::update` Increment (only the caller
//!     that flips `fire` to 1 spawns), so concurrent completions and
//!     at-least-once re-delivery cannot double-spawn while the join record
//!     exists. When the join fires, every predecessor subscription (each firing
//!     records its own id, stamped into the sidecar by the turn-event fan-out)
//!     is auto-unregistered before the record is GC'd, so nothing re-fires
//!     after cleanup.
//!
//! Trigger-fired calls are dispatched engine-side and bypass the per-turn
//! dispatch policy, so this may fire `state::update` and dispatch
//! `harness::spawn` outside the dispatcher's pending path. The function stays
//! visible in the catalog (the system prompt names its id), but a direct
//! agent call arrives without the trigger's metadata sidecar and is a no-op
//! by design; deployments additionally deny it in their permissions policy
//! (see the repo's iii-permissions.yaml conventions).
//!
//! ponytail: a trigger fires with no live parent turn, so spawned sub-agents
//! are unparented (depth 0). Emergent joins are fixed-arity (`expect` lists the
//! predecessors) — no fan-out over arrays, no retries, no central run record;
//! that heavier machinery stays in the `workflow` worker.

use iii_helpers::observability::opentelemetry::trace::{Status, TraceContextExt as _};
use iii_helpers::observability::opentelemetry::{Context, KeyValue};
use iii_sdk::protocol::TriggerRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::deps::Deps;
use crate::error::HarnessError;

pub const REACT_ID: &str = "harness::react";
pub const REACT_DESC: &str =
    "Internal trigger bridge: reshape a subscribed event into a harness::spawn (a sub-agent), \
     optionally behind a join barrier. Not called directly — THE way to spin up sub-agents on \
     events and callbacks: bind it via engine::register_trigger with the sub-agent spec in \
     `metadata`.";

/// Fired-event payload of a reactive subscription: arbitrary JSON produced by
/// the originating trigger, then appended to the spawned sub-agent task.
///
/// The handler keeps a lossless `serde_json::Value` payload internally because
/// trigger events are not always objects. This wrapper exists so registration
/// publishes a real schema instead of `AnyValue`, which the registry publish
/// gate rejects.
#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct ReactEvent(pub Value);

impl JsonSchema for ReactEvent {
    fn schema_name() -> String {
        "ReactEvent".to_string()
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
                    "Arbitrary fired-event payload from the subscribed trigger; appended to the \
                     spawned sub-agent task."
                        .to_string(),
                ),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

/// iii-state scope holding join accumulator records (one key per `join.id`).
const JOIN_SCOPE: &str = "harness::react_join";

/// Loop breaker #3: per-binding fire-rate cap. The reactive depth cap (loop
/// breaker #2) only guards chains that stay on turn events — a cycle routed
/// through an agent's `state::set` re-enters at depth 0, so a runaway there
/// shows up as raw fire RATE instead of depth. Cap fires per subscription.
pub const MAX_FIRES_PER_WINDOW: usize = 10;
pub const FIRE_WINDOW_MS: i64 = 60_000;

/// The latest fire dropped by a capped binding, held for the trailing
/// coalesced fire. Only the newest event is kept: react patterns are
/// recompute-style (aggregate from source of truth), so the last event
/// subsumes the dropped ones; `dropped` carries how many were collapsed.
pub struct PendingFire {
    pub event: Value,
    pub metadata: Value,
    pub dropped: usize,
}

/// Outcome of deferring a capped fire: how many fires the pending slot has
/// collapsed so far, and — on the FIRST deferral of a window — the delay
/// after which the caller must run the trailing fire.
pub struct Deferred {
    pub dropped: usize,
    pub schedule_delay_ms: Option<i64>,
}

#[derive(Default)]
struct GateSlot {
    fires: std::collections::VecDeque<i64>,
    pending: Option<PendingFire>,
    /// A trailing task is already scheduled for this key — deferrals only
    /// update `pending` until it drains.
    trailing_scheduled: bool,
}

/// Sliding-window fire counter per subscription (or per spec-hash for
/// bindings that carry no `__subscription_id`, e.g. `state`-provider ones).
///
/// Capped fires COALESCE instead of dropping: the newest event parks in the
/// slot and one trailing fire delivers it when the window frees. Bounded: at
/// most one pending event and one trailing task per key, so a true runaway
/// still costs at most `MAX_FIRES_PER_WINDOW + 1` reactions per window.
#[derive(Default)]
pub struct FireGate {
    inner: std::sync::Mutex<std::collections::HashMap<String, GateSlot>>,
}

impl FireGate {
    /// Record a fire attempt for `key` at `now_ms`; `false` when the key has
    /// exhausted its window budget — the caller must defer instead of react.
    pub fn admit(&self, key: &str, now_ms: i64) -> bool {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        // Opportunistic GC so dead keys can't grow the map unboundedly. Keys
        // with a parked pending fire stay: a trailing task will drain them.
        if map.len() > 1024 {
            map.retain(|_, s| {
                s.pending.is_some() || s.fires.back().is_some_and(|t| now_ms - t < FIRE_WINDOW_MS)
            });
        }
        let s = map.entry(key.to_string()).or_default();
        while s
            .fires
            .front()
            .is_some_and(|t| now_ms - t >= FIRE_WINDOW_MS)
        {
            s.fires.pop_front();
        }
        if s.fires.len() >= MAX_FIRES_PER_WINDOW {
            return false;
        }
        s.fires.push_back(now_ms);
        true
    }

    /// Park a capped fire as the key's pending trailing event (newest wins,
    /// prior drops accumulate). Returns the trailing delay exactly once per
    /// drain cycle — the caller that receives `Some` owns scheduling.
    pub fn defer(&self, key: &str, event: Value, metadata: Value, now_ms: i64) -> Deferred {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let s = map.entry(key.to_string()).or_default();
        let dropped = s.pending.as_ref().map_or(0, |p| p.dropped) + 1;
        s.pending = Some(PendingFire {
            event,
            metadata,
            dropped,
        });
        let schedule_delay_ms = if s.trailing_scheduled {
            None
        } else {
            s.trailing_scheduled = true;
            // Fire when the oldest window entry expires (budget frees). The
            // floor guards clock skew; the ceiling guards a corrupt queue.
            let delay = s
                .fires
                .front()
                .map_or(FIRE_WINDOW_MS, |t| t + FIRE_WINDOW_MS - now_ms);
            Some(delay.clamp(1_000, FIRE_WINDOW_MS))
        };
        Deferred {
            dropped,
            schedule_delay_ms,
        }
    }

    /// Drain the key's pending fire for the trailing task. Clears the
    /// scheduled flag: a later deferral starts a new drain cycle.
    pub fn take_pending(&self, key: &str) -> Option<PendingFire> {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        let s = map.get_mut(key)?;
        s.trailing_scheduled = false;
        s.pending.take()
    }
}

/// A join barrier: the downstream spawns only after every `expect` predecessor
/// has fired. Every predecessor's subscription carries the same `id` + `expect`
/// and its own `key`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct JoinSpec {
    /// Shared id for this join across all its predecessor subscriptions (the
    /// accumulator record key).
    pub id: String,
    /// Every predecessor key that must arrive before the downstream spawns.
    pub expect: Vec<String>,
    /// This predecessor's key (should be one of `expect`).
    pub key: String,
    /// Keep the predecessor subscriptions registered after the join fires so
    /// it can fire again on the next complete set (standing watchers). By
    /// default they auto-unregister after one fire.
    #[serde(default)]
    pub rearm: bool,
}

/// A function-call reaction: on fire, dispatch `function_id` with the fired
/// event injected into `payload` at `event_into` instead of spawning a
/// sub-agent. Deterministic, zero-token reactions — the mechanical-validator
/// primitive (e.g. a `fp::pipe` that counts rows and conditionally writes a
/// `turn_complete` state key). The target is validated against the
/// REGISTRANT's dispatch policy when the registration comes through the
/// harness interceptor; raw engine-side registrations are operator-trusted,
/// exactly like spawn-mode `options` today.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct CallSpec {
    /// The function to dispatch when the subscription fires.
    pub function_id: String,
    /// Base payload for the call (default `{}`).
    #[serde(default)]
    pub payload: Option<Value>,
    /// JSON pointer where the fired event lands inside the payload (default
    /// `/event`). A completed join's downstream call receives
    /// `{ results: { <key>: <event>, … } }` at the same pointer instead.
    #[serde(default)]
    pub event_into: Option<String>,
}

/// The reaction to run when the subscription fires, carried in the trigger's
/// `metadata` and delivered to this handler as the metadata sidecar. Two
/// modes, exactly one of which must be set: `task` spawns a sub-agent;
/// `call` dispatches a function.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ReactSpec {
    /// Model for the reacting sub-agent (task mode; required by
    /// `harness::spawn`). Agent registrations that omit it inherit the
    /// registering turn's model — and its provider, when `provider` is also
    /// unset — stamped by the `engine::register_trigger` interceptor. Raw
    /// engine-side registrations have no turn to inherit from and must pass
    /// one. Meaningless (and rejected) in call mode.
    #[serde(default)]
    pub model: Option<String>,
    /// The sub-agent's opening task; the event (simple) or all predecessor
    /// results (join) are appended fenced so it sees its inputs. Mutually
    /// exclusive with `call`.
    #[serde(default)]
    pub task: Option<String>,
    /// Function-call reaction (no sub-agent, no model). Mutually exclusive
    /// with `task`.
    #[serde(default)]
    pub call: Option<CallSpec>,
    /// Spawn into this session (e.g. a fork); when omitted, defaults to the
    /// registering session (the pipeline's owner) so the result lands back
    /// as a turn in that chat — a fresh detached child only for raw
    /// registrations that carry no owner stamp.
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    /// `harness::spawn` `options` passthrough (system_prompt, mode, max_turns,
    /// output contract, narrowed functions policy, …).
    #[serde(default)]
    pub options: Option<Value>,
    /// Display-only root for the console session tree. When omitted, the
    /// reaction nests under the ROOT of the firing session (its topmost
    /// ancestor) — or of the registering session when the event carries no
    /// session id (state/cron/stream) — so the whole reactive flow collapses
    /// under one root rather than a deep per-edge chain. Set it to pin a
    /// specific root.
    #[serde(default)]
    pub parent_session_id: Option<String>,
    /// By default, a failed/cancelled turn completion (or a completed turn
    /// carrying `result_error`) does not start the success-path reaction.
    /// Set true only for an explicit error-handler/reviewer stage that needs
    /// the failed event and any preserved partial result.
    #[serde(default)]
    pub continue_on_error: bool,
    /// When present, this subscription is one predecessor of a join barrier.
    #[serde(default)]
    pub join: Option<JoinSpec>,
    /// Stamped by the harness's turn-event fan-out (never caller-supplied): the
    /// firing subscription's registration id, so a completed join can
    /// auto-unregister its predecessor subscriptions.
    #[serde(default, rename = "__subscription_id")]
    pub subscription_id: Option<String>,
    /// Effective one-shot policy stamped by the registration interceptor
    /// (never caller-supplied): retire this binding after its first successful
    /// spawn or gated upstream failure. Meaningless on join edges — the join
    /// lifecycle owns predecessor bindings.
    #[serde(default, rename = "__once")]
    pub once: bool,
    /// Stamped by the interceptor at registration (never caller-supplied): the
    /// registering session. Console-tree parent fallback for fires whose event
    /// carries no session id (state/cron/stream).
    #[serde(default, rename = "__owner_session_id")]
    pub owner_session_id: Option<String>,
    /// Stamped by the interceptor at registration (never caller-supplied): the
    /// registering turn's dispatch policy. A reaction with no
    /// `options.functions` inherits it; explicit options are subset against
    /// it (narrow, never escalate) — the in-turn child rule. Absent on raw
    /// engine-side registrations, which keep the read-only-baseline fallback.
    #[serde(default, rename = "__registrant_functions")]
    pub registrant_functions: Option<crate::types::turn::FunctionPolicy>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReactResult {
    /// Whether a `harness::spawn` was dispatched this call (task mode).
    pub spawned: bool,
    /// Whether a function was dispatched this call (call mode).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub called: bool,
    /// The spawned sub-agent's child session id, when spawn returned one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    /// Why nothing spawned/called (missing spec, join not yet complete,
    /// already fired, error). Present iff `!spawned && !called`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The spawned turn id — per-fire unique even when delivery reuses the
    /// pinned/owner session (the default). Local bookkeeping for fired-record
    /// entry ids only; kept off the wire.
    #[serde(skip)]
    #[schemars(skip)]
    pub child_turn_id: Option<String>,
}

impl ReactResult {
    fn spawned(child: Option<String>, turn: Option<String>) -> Self {
        Self {
            spawned: true,
            called: false,
            child_session_id: child,
            note: None,
            child_turn_id: turn,
        }
    }
    fn called() -> Self {
        Self {
            spawned: false,
            called: true,
            child_session_id: None,
            note: None,
            child_turn_id: None,
        }
    }
    fn note(msg: impl Into<String>) -> Self {
        Self {
            spawned: false,
            called: false,
            child_session_id: None,
            note: Some(msg.into()),
            child_turn_id: None,
        }
    }
}

/// Canonical reaction-spec fingerprint for a join predecessor registration:
/// the metadata minus this predecessor's own `join.key` and the harness
/// stamps (which legitimately differ per predecessor). `None` when the spec
/// is not a join predecessor.
///
/// All predecessors of one join id must agree on this fingerprint — the join
/// fires with the COMPLETING predecessor's spec (`join_edge` → `spec.task`),
/// so a divergent spec on a later predecessor (e.g. `task: "placeholder"`)
/// silently replaces the real reaction. The registration interceptor rejects
/// mismatches against live predecessors using it.
pub fn join_canon(metadata: Option<&Value>) -> Option<(String, Value)> {
    let m = metadata?;
    let join_id = m.pointer("/join/id")?.as_str()?.to_string();
    let mut canon = m.clone();
    if let Some(obj) = canon.as_object_mut() {
        // Stamps differ per registration by design; strip them defensively
        // even though the interceptor fingerprints pre-stamp metadata.
        obj.remove("__subscription_id");
        obj.remove("__once");
        obj.remove(crate::subscriptions::OWNER_SESSION_KEY);
        if let Some(j) = obj.get_mut("join").and_then(Value::as_object_mut) {
            j.remove("key");
        }
    }
    Some((join_id, canon))
}

/// Registration-time validation for subscriptions targeting `harness::react`:
/// once bound, a bad spec would only surface as a silent no-op when the event
/// fires, so reject it loudly at `engine::register_trigger` time instead.
pub fn validate_spec(metadata: Option<&Value>) -> Result<(), String> {
    let Some(m) = metadata else {
        return Err(
            "harness::react needs the reaction spec in the registration `metadata`: \
             { model, task, session_id?, join?: { id, expect: [\"key\", ...], key } } for an \
             agent reaction, or { call: { function_id, payload? }, join? } for a function \
             reaction"
                .into(),
        );
    };
    let spec: ReactSpec = serde_json::from_value(m.clone()).map_err(|e| {
        format!(
            "invalid harness::react metadata spec: {e}. Expected \
             {{ model, task, session_id?, join?: {{ id, expect: [\"key\", ...], key }} }} — \
             `join.expect` is the array of ALL predecessor keys, not a count — or \
             {{ call: {{ function_id, payload? }}, join? }} for a function reaction."
        )
    })?;
    match (&spec.task, &spec.call) {
        (None, None) => {
            return Err(
                "a react spec needs exactly one reaction: `task` (spawn a sub-agent) or \
                 `call` (dispatch a function)"
                    .into(),
            )
        }
        (Some(_), Some(_)) => {
            return Err("`task` and `call` are mutually exclusive — pick one reaction".into())
        }
        (Some(_), None) => {
            if spec.model.as_deref().is_none_or(str::is_empty) {
                return Err(
                    "a task reaction needs a `model` (agent registrations inherit the \
                     registering turn's automatically)"
                        .into(),
                );
            }
        }
        (None, Some(call)) => {
            if call.function_id.is_empty() {
                return Err("`call.function_id` must name the function to dispatch".into());
            }
            // Spawn-only knobs are meaningless without a sub-agent; rejecting
            // them loudly beats silently ignoring a mis-designed spec.
            for (set, name) in [
                (spec.model.is_some(), "model"),
                (spec.session_id.is_some(), "session_id"),
                (spec.parent_session_id.is_some(), "parent_session_id"),
                (spec.options.is_some(), "options"),
                (spec.provider.is_some(), "provider"),
            ] {
                if set {
                    return Err(format!(
                        "`{name}` is a sub-agent (task-mode) field — a call reaction takes \
                         only {{ call, join?, continue_on_error? }}"
                    ));
                }
            }
        }
    }
    if let Some(j) = &spec.join {
        if j.expect.is_empty() {
            return Err(format!(
                "join {}: `expect` must list every predecessor key",
                j.id
            ));
        }
        if !j.expect.contains(&j.key) {
            return Err(format!(
                "join {}: `key` \"{}\" is not in `expect` {:?} — each predecessor's `key` must \
                 be one of the expected keys",
                j.id, j.key, j.expect
            ));
        }
    }
    Ok(())
}

/// Type-erased re-entry for the trailing coalesced fire: `handle` recursing
/// through `tokio::spawn` can't prove its own future `Send` (the bound is
/// self-referential); the `dyn` boxing breaks the inference cycle.
fn handle_boxed<'a>(
    deps: &'a Deps,
    event: Value,
    metadata: Option<Value>,
) -> std::pin::Pin<
    Box<dyn std::future::Future<Output = Result<ReactResult, HarnessError>> + Send + 'a>,
> {
    Box::pin(handle(deps, event, metadata))
}

pub async fn handle(
    deps: &Deps,
    event: Value,
    metadata: Option<Value>,
) -> Result<ReactResult, HarnessError> {
    // A bad/absent spec must never error out: an erroring trigger target just
    // spams the engine's dispatch log. Log and no-op instead. The raw metadata
    // is kept beside the parsed spec: a capped fire parks it for the trailing
    // coalesced re-entry.
    let raw_metadata = metadata.clone();
    let spec: ReactSpec = match metadata {
        Some(m) => match serde_json::from_value(m) {
            Ok(s) => s,
            Err(e) => {
                tracing::warn!(error = %e, "harness::react: unparseable metadata spec; ignoring");
                return Ok(ReactResult::note(format!("invalid react spec: {e}")));
            }
        },
        None => {
            tracing::warn!("harness::react fired without a metadata spec; ignoring");
            return Ok(ReactResult::note("no metadata spec"));
        }
    };

    // Loop breaker #3: refuse runaway fire rates before touching anything
    // else (a tripped binding must not even cost catalog lookups).
    let gate_key = spec.subscription_id.clone().unwrap_or_else(|| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        // Join predecessors share the WHOLE downstream spec except their
        // `key` — without it in the hash every predecessor of one join
        // shares a single fire budget and a wide join trips the breaker.
        (
            &spec.model,
            &spec.task,
            &spec.session_id,
            spec.call
                .as_ref()
                .map(|c| (&c.function_id, c.payload.as_ref().map(Value::to_string))),
            spec.join.as_ref().map(|j| (&j.id, &j.key)),
        )
            .hash(&mut h);
        format!("spec:{:016x}", h.finish())
    });
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0);
    if !deps.react_gate.admit(&gate_key, now_ms) {
        // Coalesce instead of dropping: react patterns are recompute-style,
        // so the newest event subsumes the capped ones. Park it and let one
        // trailing fire deliver it when the window frees — without this, a
        // burst's TAIL is exactly what gets dropped and aggregates freeze
        // one step behind the source of truth (rctest-k7m3 postmortem).
        let deferred = deps.react_gate.defer(
            &gate_key,
            event.clone(),
            raw_metadata.unwrap_or(Value::Null),
            now_ms,
        );
        tracing::warn!(
            subscription = %gate_key,
            dropped = deferred.dropped,
            "harness::react: fire-rate breaker tripped ({MAX_FIRES_PER_WINDOW} fires/{FIRE_WINDOW_MS}ms); coalescing"
        );
        if let Some(delay_ms) = deferred.schedule_delay_ms {
            let deps = deps.clone();
            let key = gate_key.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms as u64)).await;
                let Some(pending) = deps.react_gate.take_pending(&key) else {
                    return;
                };
                let mut event = pending.event;
                if let Some(obj) = event.as_object_mut() {
                    // Tell the reaction it stands in for a collapsed burst.
                    obj.insert("__coalesced_fires".to_string(), json!(pending.dropped));
                }
                // Re-enters the gate: if the window is still hot the fire
                // parks again and a fresh trailing task takes over.
                if let Err(e) = handle_boxed(&deps, event, Some(pending.metadata)).await {
                    tracing::warn!(subscription = %key, error = %e,
                        "harness::react: trailing coalesced fire failed");
                }
            });
        }
        let note = format!(
            "fire-rate cap ({MAX_FIRES_PER_WINDOW} per {FIRE_WINDOW_MS}ms) reached; coalescing — \
             the latest event fires once the window frees ({} collapsed so far)",
            deferred.dropped
        );
        record_reaction_outcome(deps, &spec, &event, "waiting", &note, None).await;
        return Ok(ReactResult::note(note));
    }

    // Fire-time model check (task mode only — call reactions have no model):
    // registrations made through OTHER trigger-type providers (e.g. `state`)
    // never pass the harness's registration-time validation, so a
    // memory-written model id ("gpt-4o") would spawn a turn that instantly
    // fails on every event. Refuse to spawn instead; fail open when the
    // catalog is unreachable.
    if spec.call.is_none() {
        let model = spec.model.clone().unwrap_or_default();
        if model.is_empty() {
            // Raw registrations skip registration-time validation; a task
            // reaction without a model can never spawn.
            let note = "task reaction without a model; not spawning".to_string();
            record_reaction_outcome(deps, &spec, &event, "failed", &note, None).await;
            return Ok(ReactResult::note(note));
        }
        if let Some(ids) = known_model_ids(&deps.iii).await {
            if !ids.iter().any(|id| id == &model) {
                tracing::warn!(
                    model = %model,
                    "harness::react: unknown model in reaction spec; not spawning"
                );
                let note = format!(
                    "unknown model \"{model}\" (not in router::models::list); not spawning"
                );
                record_reaction_outcome(deps, &spec, &event, "failed", &note, None).await;
                return Ok(ReactResult::note(note));
            }
        }
    }

    // Loop breaker #2 (backstop): every react-spawned turn carries a reactive
    // depth; its completion event echoes it. A chain past the cap is refused
    // no matter which subscriptions form it (self-edge, A→B→A ping-pong,
    // instant-fail respawn storms). Call reactions spawn nothing and carry no
    // depth — their runaway shape (call → state write → another call edge) is
    // bounded by the per-subscription fire-rate gate above.
    let incoming_depth = event_reactive_depth(&event);
    if spec.call.is_none() && incoming_depth >= MAX_REACTIVE_DEPTH {
        tracing::warn!(
            reactive_depth = incoming_depth,
            "harness::react: reactive depth cap reached; refusing to spawn"
        );
        let note = format!("reactive depth cap ({MAX_REACTIVE_DEPTH}) reached; not spawning");
        record_reaction_outcome(deps, &spec, &event, "failed", &note, None).await;
        return Ok(ReactResult::note(note));
    }
    let spawn_depth = incoming_depth + 1;

    // Console-tree parent: an explicit spec value pins a root; otherwise nest
    // under the ROOT of the anchor session (walk up its parent chain) so the
    // whole reactive flow collapses under one root instead of a deep per-edge
    // chain.
    let parent = match spec.parent_session_id.clone() {
        Some(p) => Some(p),
        None => match parent_anchor(&event, &spec) {
            Some(sid) => Some(resolve_root(deps, &sid).await),
            None => None,
        },
    };

    // Delivery session, resolved once for BOTH dispatch paths below: an
    // explicit pin wins; otherwise the registering (owner) session, so a
    // reaction with no pin lands as a turn in the chat that wired it instead
    // of a detached child nobody watches.
    let mut spec = spec;
    spec.session_id = reaction_delivery_session(&spec);

    let completion_failure = turn_completion_failure(&event);
    match spec.join.clone() {
        None if completion_failure.is_some() && !spec.continue_on_error => {
            if spec.once {
                once_unregister(deps, &spec).await;
            }
            let note = format!(
                "upstream turn did not complete successfully; reaction stopped: {}",
                completion_failure.as_deref().unwrap_or("unknown failure")
            );
            record_reaction_outcome(deps, &spec, &event, "blocked", &note, None).await;
            Ok(ReactResult::note(note))
        }
        None if spec.call.is_some() => {
            let call = spec.call.clone().expect("guarded");
            call_edge(deps, &spec, &call, &event, event.clone()).await
        }
        None => {
            let task = spec.task.clone().unwrap_or_default();
            let res = spawn_reaction(
                deps,
                single_event_task(&task, &event),
                &spec,
                parent,
                spawn_depth,
            )
            .await;
            if let Ok(r) = &res {
                if r.spawned {
                    let sub = spec.subscription_id.as_deref().unwrap_or("sub");
                    // Per-fire suffix: the spawned turn id is unique even when
                    // delivery reuses the pinned/owner session (the default),
                    // where the child session id repeats and would dedup every
                    // recurring fire after the first into one record.
                    // ponytail: `spawn` fallback when the spawn returned no
                    // ids — such fires dedup to one record, bounded by the
                    // fire-rate gate.
                    let fire_key = r
                        .child_turn_id
                        .as_deref()
                        .or(r.child_session_id.as_deref())
                        .unwrap_or("spawn");
                    let entry_id = format!("e_trigfired_{sub}_{fire_key}");
                    // Resolve the engine trigger id while the binding is live,
                    // tear the once-binding down, then record what actually
                    // happened: a failed unregister must not claim `retired` —
                    // the row is still live in the panel and must keep its
                    // real unregister action (the retained mapping retries on
                    // the next fire, whose record then carries retired:true).
                    let trigger_id = spec
                        .subscription_id
                        .as_deref()
                        .and_then(|s| deps.subscriptions.trigger_id_of(s));
                    let retired = spec.once && once_unregister(deps, &spec).await;
                    emit_fired(
                        deps,
                        &spec,
                        &event,
                        &entry_id,
                        r.child_session_id.as_deref(),
                        retired,
                        trigger_id,
                        None,
                        None,
                    )
                    .await;
                }
            }
            if let Ok(outcome) = &res {
                let status = if outcome.spawned { "spawned" } else { "failed" };
                let summary = outcome.note.clone().unwrap_or_else(|| {
                    outcome
                        .child_session_id
                        .as_deref()
                        .map(|id| format!("Reaction spawned child session {id}."))
                        .unwrap_or_else(|| "Reaction spawned a downstream turn.".to_string())
                });
                record_reaction_outcome(
                    deps,
                    &spec,
                    &event,
                    status,
                    &summary,
                    outcome.child_session_id.as_deref(),
                )
                .await;
            }
            res
        }
        Some(join) => {
            let blocked_by = if spec.continue_on_error {
                None
            } else {
                completion_failure.as_deref()
            };
            join_edge(deps, event, &spec, &join, parent, spawn_depth, blocked_by).await
        }
    }
}

/// One predecessor of a join fired: record it durably, and spawn the downstream
/// exactly once when the last predecessor arrives.
async fn join_edge(
    deps: &Deps,
    event: Value,
    spec: &ReactSpec,
    join: &JoinSpec,
    parent: Option<String>,
    spawn_depth: u32,
    blocked_by: Option<&str>,
) -> Result<ReactResult, HarnessError> {
    // Step 1 — record this predecessor idempotently (Merge, so re-delivery of
    // the same key overwrites its own slot and never inflates the count), and
    // read the accumulator back.
    let mut ops = vec![
        merge_op("results", json!({ &join.key: event.clone() })),
        merge_op("arrived", json!({ &join.key: true })),
    ];
    if let Some(sid) = &spec.subscription_id {
        ops.push(merge_op("bindings", json!({ &join.key: sid })));
    }
    if let Some(reason) = blocked_by {
        ops.push(merge_op("failures", json!({ &join.key: reason })));
    }
    let rec = match state_update(deps, &join.id, ops).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, join = %join.id, "harness::react: join record update failed");
            let note = format!("join update failed: {e}");
            record_reaction_outcome(deps, spec, &event, "failed", &note, None).await;
            return Ok(ReactResult::note(note));
        }
    };

    let arrived = arrived_count(&rec);
    let expected = join.expect.len();
    if arrived < expected {
        let note = format!("{arrived}/{expected} arrived");
        // ponytail: these join entry ids are cycle-invariant, so a re-armed
        // join's cycle ≥2 records dedup away (append_custom is idempotent on
        // entry_id — the same property that absorbs engine redelivery). Key in
        // a cycle counter if later cycles ever need their own notices.
        emit_fired(
            deps,
            spec,
            &event,
            &format!("e_trigfired_join_{}_{}", join.id, join.key),
            None,  // nothing spawned yet
            false, // predecessor stays registered until the join completes
            None,  // binding live — resolve inside
            Some(crate::subscriptions::fired::JoinProgress {
                id: &join.id,
                key: &join.key,
                arrived,
                expected,
                completed: false,
            }),
            Some(&note),
        )
        .await;
        let outcome = format!("join {}: {note}", join.id);
        record_reaction_outcome(deps, spec, &event, "waiting", &outcome, None).await;
        return Ok(ReactResult::note(format!(
            "join {}: {arrived}/{expected} arrived",
            join.id
        )));
    }

    // Step 2 — atomic fire-once guard. Increment starts a missing counter at
    // `by`, so exactly one caller sees `fire == 1`; concurrent completers and
    // later re-deliveries get ≥2 and stop.
    let guard = match state_update(deps, &join.id, vec![incr_op("fire", 1)]).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, join = %join.id, "harness::react: join fire-guard failed");
            let note = format!("join fire-guard failed: {e}");
            record_reaction_outcome(deps, spec, &event, "failed", &note, None).await;
            return Ok(ReactResult::note(note));
        }
    };
    if guard.get("fire").and_then(Value::as_i64) != Some(1) {
        return Ok(ReactResult::note(format!("join {} already fired", join.id)));
    }

    // The join is committed. Unless it re-arms, auto-unregister every
    // predecessor subscription (recorded per key) so nothing re-fires after
    // the accumulator is GC'd, and evict each binding's local slot (created
    // when the agent registered through the engine::register_trigger
    // interceptor) so fired joins don't leak the session's subscription cap.
    // Best-effort — a failed unregister never blocks the downstream spawn.
    let mut retired = !join.rearm;
    if join.rearm {
        tracing::info!(join = %join.id, "harness::react: join re-armed; predecessor subscriptions stay registered");
    } else {
        for id in join_binding_ids(&rec) {
            if let Err(e) = retire_binding(deps, &id).await {
                retired = false;
                tracing::warn!(error = %e, join = %join.id, subscription = %id, "harness::react: join predecessor auto-unregister failed");
            }
        }
    }

    let failed = join_failure_keys(&rec);
    if !failed.is_empty() {
        cleanup_join_record(deps, join).await;
        let note = format!(
            "join {} stopped because predecessors failed: {}",
            join.id,
            failed.join(", ")
        );
        record_reaction_outcome(deps, spec, &event, "blocked", &note, None).await;
        return Ok(ReactResult::note(note));
    }

    // Fire the downstream fed ALL predecessors' results, then GC the
    // accumulator record. The delivery session (owner fallback when unpinned)
    // is already resolved by the caller. Joins fire once, so this cannot spam.
    let res = match spec.call.clone() {
        // Call downstream: the accumulated results map replaces the single
        // fired event as the injected value.
        Some(call) => {
            let results = json!({
                "results": rec.get("results").cloned().unwrap_or_else(|| json!({}))
            });
            dispatch_call(deps, spec, &call, &event, results).await
        }
        None => {
            let task = gather_inputs_task(spec.task.as_deref().unwrap_or_default(), &rec);
            spawn_reaction(deps, task, spec, parent, spawn_depth).await
        }
    };
    // The join committed: the downstream spawned and (unless re-armed) the
    // predecessors were torn down above — `retired` carries the real outcome,
    // so a failed unregister is never reported as gone. One completion record
    // lets the console mark the whole join fired + retired and post the
    // notice. Gated on `spawned` like the simple edge — spawn_reaction
    // swallows dispatch errors into `spawned: false`, and a record claiming
    // "spawned" for a spawn that never happened would mislead the chat.
    if let Ok(r) = &res {
        if r.spawned || r.called {
            let note = if r.called {
                format!("{expected}/{expected} arrived — called")
            } else {
                format!("{expected}/{expected} arrived — spawned")
            };
            emit_fired(
                deps,
                spec,
                &event,
                &format!("e_trigfired_join_{}_done", join.id),
                r.child_session_id.as_deref(),
                retired,
                None, // predecessors already retired; sub-keyed ghost is right
                Some(crate::subscriptions::fired::JoinProgress {
                    id: &join.id,
                    key: &join.key,
                    arrived: expected,
                    expected,
                    completed: true,
                }),
                Some(&note),
            )
            .await;
        }
    }
    if let Ok(outcome) = &res {
        let status = if outcome.spawned {
            "spawned"
        } else if outcome.called {
            "called"
        } else {
            "failed"
        };
        let summary = outcome.note.clone().unwrap_or_else(|| {
            if outcome.called {
                let target = spec
                    .call
                    .as_ref()
                    .map(|c| c.function_id.as_str())
                    .unwrap_or("function");
                format!("Join {} dispatched {target}.", join.id)
            } else {
                outcome
                    .child_session_id
                    .as_deref()
                    .map(|id| format!("Join {} spawned child session {id}.", join.id))
                    .unwrap_or_else(|| format!("Join {} spawned its downstream turn.", join.id))
            }
        });
        record_reaction_outcome(
            deps,
            spec,
            &event,
            status,
            &summary,
            outcome.child_session_id.as_deref(),
        )
        .await;
    }
    // The delete is the cycle reset: a stale record (fire=1, all keys arrived)
    // makes the next cycle's fire-guard land on 2 and refuse forever — for a
    // rearmed join that is a permanent, silent wedge. Retry transient state
    // errors before giving up.
    // ponytail: 3 fixed retries; a generation-stamped fire guard if state
    // outages ever outlast them.
    cleanup_join_record(deps, join).await;
    res
}

async fn cleanup_join_record(deps: &Deps, join: &JoinSpec) {
    let mut cleanup = state_delete(deps, &join.id).await;
    for _ in 0..2 {
        if cleanup.is_ok() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        cleanup = state_delete(deps, &join.id).await;
    }
    if let Err(e) = cleanup {
        if join.rearm {
            tracing::error!(error = %e, join = %join.id, "harness::react: join record cleanup failed after retries — this REARMED join will not fire again until the record is deleted (scope harness::react_join)");
        } else {
            tracing::warn!(error = %e, join = %join.id, "harness::react: join record cleanup failed after retries (one-shot join; record is orphaned, not blocking)");
        }
    }
}

/// Build + fire the `harness::spawn`. Fire-and-forget; swallow errors so one bad
/// reaction never wedges the engine's trigger dispatch.
/// A simple call edge fired: dispatch the function, then handle the
/// once-retire + fired record + outcome record — the call-mode mirror of the
/// simple spawn edge.
async fn call_edge(
    deps: &Deps,
    spec: &ReactSpec,
    call: &CallSpec,
    event: &Value,
    injected: Value,
) -> Result<ReactResult, HarnessError> {
    let res = dispatch_call(deps, spec, call, event, injected).await;
    if let Ok(r) = &res {
        if r.called {
            let sub = spec.subscription_id.as_deref().unwrap_or("sub");
            // Per-fire suffix: calls have no child ids, so a random suffix
            // keeps recurring fires from deduping into one record.
            let entry_id = format!("e_trigfired_{sub}_{}", uuid::Uuid::new_v4().simple());
            let trigger_id = spec
                .subscription_id
                .as_deref()
                .and_then(|s| deps.subscriptions.trigger_id_of(s));
            let retired = spec.once && once_unregister(deps, spec).await;
            emit_fired(
                deps, spec, event, &entry_id, None, retired, trigger_id, None, None,
            )
            .await;
        }
        let status = if r.called { "called" } else { "failed" };
        let summary = r
            .note
            .clone()
            .unwrap_or_else(|| format!("Reaction dispatched {}.", call.function_id));
        record_reaction_outcome(deps, spec, event, status, &summary, None).await;
    }
    res
}

/// Dispatch a call reaction: the injected value (the fired event, or a
/// completed join's results map) lands in the base payload at `event_into`
/// (default `/event`), then the function is triggered with worker authority —
/// the registrant's policy was enforced at registration. Errors are swallowed
/// into a note, mirroring spawn_reaction: one bad reaction must never wedge
/// the engine's trigger dispatch.
async fn dispatch_call(
    deps: &Deps,
    spec: &ReactSpec,
    call: &CallSpec,
    _event: &Value,
    injected: Value,
) -> Result<ReactResult, HarnessError> {
    let base = call.payload.clone().unwrap_or_else(|| json!({}));
    let pointer = call.event_into.as_deref().unwrap_or("/event");
    let payload = match inject_at(base, pointer, injected) {
        Ok(p) => p,
        Err(msg) => {
            tracing::warn!(function_id = %call.function_id, error = %msg, "harness::react: call payload injection failed");
            return Ok(ReactResult::note(format!(
                "call payload injection failed: {msg}"
            )));
        }
    };
    // SECURITY: a reaction's call payload is model-authored at registration
    // time and dispatches OUTSIDE the turn loop, so it never passes the
    // trusted fs_scope stamp path. Strip any authored scope from stamped
    // targets (shell::*/coder::*/fp::pipe) — root=None is the fail-closed
    // strip — AFTER event injection, so a scope threaded via `event_into`
    // cannot survive either. Scoped calls from reactions are therefore
    // refused downstream until a trusted per-reaction scope source exists.
    let payload = crate::filesystem_scope::inject(
        &call.function_id,
        payload,
        None,
        &[],
        crate::filesystem_scope::FilesystemBoundary::ConfiguredRoots,
    );
    match deps
        .iii
        .trigger(TriggerRequest {
            function_id: call.function_id.clone(),
            payload,
            action: None,
            timeout_ms: Some(CALL_TIMEOUT_MS),
        })
        .await
    {
        Ok(_) => {
            tracing::info!(
                function_id = %call.function_id,
                subscription = spec.subscription_id.as_deref(),
                "harness::react: reaction called"
            );
            Ok(ReactResult::called())
        }
        Err(e) => {
            tracing::warn!(error = %e, function_id = %call.function_id, "harness::react: call dispatch failed");
            Ok(ReactResult::note(format!(
                "call {} failed: {e}",
                call.function_id
            )))
        }
    }
}

/// Reaction calls are bounded — a hung target must not pin the fire handler.
const CALL_TIMEOUT_MS: u64 = 120_000;

/// Set `value` at `pointer` inside `payload`, creating intermediate objects
/// (object keys only — same rules as fp::pipe's `into`).
fn inject_at(payload: Value, pointer: &str, value: Value) -> Result<Value, String> {
    let parts: Vec<String> = pointer
        .strip_prefix('/')
        .map(|p| {
            p.split('/')
                .map(|t| t.replace("~1", "/").replace("~0", "~"))
                .collect()
        })
        .unwrap_or_default();
    if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
        return Err(format!(
            "`event_into` must be a JSON pointer like \"/event\", got {pointer:?}"
        ));
    }
    let mut root = match payload {
        Value::Object(m) => Value::Object(m),
        Value::Null => json!({}),
        other => {
            return Err(format!(
                "call payload must be an object to receive `event_into`, got {}",
                kind_name(&other)
            ))
        }
    };
    let mut cursor = &mut root;
    for part in &parts[..parts.len() - 1] {
        let map = cursor.as_object_mut().expect("object cursor");
        cursor = map.entry(part.clone()).or_insert_with(|| json!({}));
        if !cursor.is_object() {
            return Err(format!(
                "`event_into` path {pointer:?} crosses a non-object at {part:?}"
            ));
        }
    }
    let map = cursor.as_object_mut().expect("object cursor");
    map.insert(parts[parts.len() - 1].clone(), value);
    Ok(root)
}

pub(crate) fn kind_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

async fn spawn_reaction(
    deps: &Deps,
    task: String,
    spec: &ReactSpec,
    parent: Option<String>,
    reactive_depth: u32,
) -> Result<ReactResult, HarnessError> {
    let mut payload = build_spawn_payload(task, spec, parent.as_deref());
    payload["reactive_depth"] = json!(reactive_depth);
    match deps
        .iii
        .trigger(TriggerRequest {
            function_id: super::SPAWN_ID.to_string(),
            payload,
            action: None,
            timeout_ms: None,
        })
        .await
    {
        Ok(v) => {
            let child = v
                .get("child_session_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            let turn = v
                .get("child_turn_id")
                .and_then(Value::as_str)
                .map(str::to_string);
            tracing::info!(
                child_session_id = child.as_deref(),
                model = spec.model.as_deref(),
                subscription = spec.subscription_id.as_deref(),
                reactive_depth,
                "harness::react: reaction spawned"
            );
            Ok(ReactResult::spawned(child, turn))
        }
        Err(e) => {
            tracing::warn!(error = %e, "harness::react: harness::spawn dispatch failed");
            Ok(ReactResult::note(format!("spawn failed: {e}")))
        }
    }
}

async fn state_update(deps: &Deps, key: &str, ops: Vec<Value>) -> Result<Value, HarnessError> {
    let resp = deps
        .iii
        .trigger(TriggerRequest {
            function_id: "state::update".to_string(),
            payload: json!({ "scope": JOIN_SCOPE, "key": key, "ops": ops }),
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| HarnessError::Dependency(format!("state::update: {e}")))?;
    Ok(resp.get("new_value").cloned().unwrap_or(Value::Null))
}

/// The session a reaction's spawn delivers into — simple edge or a completed
/// join's downstream alike: an explicit spec pin wins; otherwise the
/// registering session (the pipeline's owner), so the result lands as a turn
/// in the chat that wired it. `None` (a fresh detached child) only for raw
/// registrations that carry no owner stamp.
fn reaction_delivery_session(spec: &ReactSpec) -> Option<String> {
    spec.session_id
        .clone()
        .or_else(|| spec.owner_session_id.clone())
}

async fn record_reaction_outcome(
    deps: &Deps,
    spec: &ReactSpec,
    event: &Value,
    status: &str,
    summary: &str,
    child_session_id: Option<&str>,
) {
    let cx = Context::current();
    let span = cx.span();
    if span.span_context().is_valid() {
        span.set_attribute(KeyValue::new("iii.reaction.outcome", status.to_string()));
        if let Some(id) = spec.subscription_id.as_deref() {
            span.set_attribute(KeyValue::new(
                "iii.reaction.subscription_id",
                id.to_string(),
            ));
        }
        if let Some(join) = spec.join.as_ref() {
            span.set_attribute(KeyValue::new("iii.reaction.join_id", join.id.clone()));
            span.set_attribute(KeyValue::new("iii.reaction.join_key", join.key.clone()));
        }
        if matches!(status, "blocked" | "failed") {
            let error_type = if status == "blocked" {
                "harness.reaction_blocked"
            } else {
                "harness.reaction_failed"
            };
            span.set_attribute(KeyValue::new("error.type", error_type));
            span.set_attribute(KeyValue::new("error.message", summary.to_string()));
            span.set_attribute(KeyValue::new("iii.tag.outcome", "failed"));
            span.set_status(Status::error(summary.to_string()));
        }
        span.add_event(
            "harness.reaction.outcome",
            vec![
                KeyValue::new("status", status.to_string()),
                KeyValue::new("summary", summary.to_string()),
            ],
        );
    }

    // Outcomes belong in the chat that wired the pipeline even when the
    // downstream turn is explicitly delivered somewhere else. Raw external
    // registrations have no owner stamp, so fall back to their target session.
    let Some(session_id) = spec
        .owner_session_id
        .clone()
        .or_else(|| spec.session_id.clone())
    else {
        return;
    };
    let entry_id = reaction_outcome_entry_id(spec, event, status);
    let source_session_id = event.get("session_id").and_then(Value::as_str);
    let source_turn_id = event.get("turn_id").and_then(Value::as_str);
    let join_id = spec.join.as_ref().map(|join| join.id.as_str());
    let join_key = spec.join.as_ref().map(|join| join.key.as_str());
    let timestamp = event
        .get("timestamp")
        .and_then(Value::as_i64)
        .unwrap_or_else(crate::types::message::AgentMessage::now_ms);
    let origin = json!({ "reaction": true });
    let _ = deps
        .session()
        .await
        .append_custom(
            &session_id,
            "reaction",
            json!({
                "status": status,
                "summary": summary,
                "subscription_id": spec.subscription_id,
                "source_session_id": source_session_id,
                "source_turn_id": source_turn_id,
                "join_id": join_id,
                "join_key": join_key,
                "child_session_id": child_session_id,
                "timestamp": timestamp,
            }),
            &entry_id,
            Some(&origin),
        )
        .await;
}

fn reaction_outcome_entry_id(spec: &ReactSpec, event: &Value, status: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut h = std::collections::hash_map::DefaultHasher::new();
    spec.subscription_id.hash(&mut h);
    spec.join
        .as_ref()
        .map(|join| (&join.id, &join.key))
        .hash(&mut h);
    status.hash(&mut h);
    event.to_string().hash(&mut h);
    format!("e_reaction_outcome_{:016x}", h.finish())
}

/// The session anchoring the console-tree parent when the spec doesn't pin
/// one: the firing session when the event carries one (turn events), else the
/// registering session stamped on the binding — state/cron/stream events carry
/// no session id, which used to strand those reactions as top-level roots.
fn parent_anchor(event: &Value, spec: &ReactSpec) -> Option<String> {
    event
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| spec.owner_session_id.clone())
}

/// Walk up the firing session's `parent_session_id` chain to the topmost
/// ancestor, so every reactive spawn nests under one root (not a deep per-edge
/// chain). Bounded against cycles; on any read failure returns the best root so
/// far — a rootless session is its own root.
async fn resolve_root(deps: &Deps, session_id: &str) -> String {
    let mut current = session_id.to_string();
    for _ in 0..32 {
        let resp = match deps
            .iii
            .trigger(TriggerRequest {
                function_id: "session::get".to_string(),
                payload: json!({ "session_id": current }),
                action: None,
                timeout_ms: None,
            })
            .await
        {
            Ok(v) => v,
            Err(_) => break,
        };
        match resp
            .pointer("/meta/metadata/parent_session_id")
            .and_then(Value::as_str)
        {
            Some(p) if !p.is_empty() && p != current => current = p.to_string(),
            _ => break,
        }
    }
    current
}

/// Retire a fired binding: engine unregister FIRST, local eviction only after
/// it succeeds — evicting first would orphan the durable engine binding as a
/// standing refire if the unregister call failed, with the `sub_` mapping gone
/// so no later retry could resolve it. Turn-event fires stamp the engine
/// binding id directly; state/cron/stream fires deliver the interceptor's
/// local `sub_` handle — resolve it through the registry first.
async fn retire_binding(deps: &Deps, id: &str) -> Result<(), HarnessError> {
    let engine_id = if id.starts_with("sub_") {
        match deps.subscriptions.trigger_id_of(id) {
            Some(t) => t,
            // Bind window: the binding fired before the registration
            // round-trip recorded its engine id. Evict the slot so
            // `set_trigger_id` finds it gone and the registration path
            // unregisters the orphan engine trigger itself.
            None if deps.subscriptions.session_of(id).is_some() => {
                deps.subscriptions.take(id);
                return Ok(());
            }
            None => {
                return Err(HarnessError::Dependency(format!(
                    "no local binding for subscription `{id}`"
                )));
            }
        }
    } else {
        id.to_string()
    };
    unregister_subscription(deps, &engine_id).await?;
    if id.starts_with("sub_") {
        deps.subscriptions.take(id);
    } else {
        deps.subscriptions.take_by_trigger_id(id);
    }
    Ok(())
}

/// A `once: true` simple edge spawned: retire its binding so it never refires.
/// Best-effort — a failed unregister only risks an extra fire, never the
/// spawn, and the retained mapping lets the next fire retry the retirement.
/// Returns whether the binding was actually retired, so the fired record's
/// `retired` flag reflects reality (a live binding mislabeled retired would
/// render dismiss-only in the console while it keeps firing).
async fn once_unregister(deps: &Deps, spec: &ReactSpec) -> bool {
    let Some(id) = spec.subscription_id.as_deref() else {
        tracing::warn!(
            "harness::react: once-binding fired without a subscription id; cannot auto-unregister"
        );
        return false;
    };
    match retire_binding(deps, id).await {
        Ok(()) => true,
        Err(e) => {
            tracing::warn!(error = %e, subscription = %id, "harness::react: once-binding auto-unregister failed; retrying on the next fire");
            false
        }
    }
}

/// Append a durable `trigger_fired` record into the owner (registering) chat so
/// the console renders a turn-less notice and keeps a fired binding visible in
/// the panel after teardown. Best-effort; owner-less raw registrations (no chat
/// to surface into) are skipped. Callers that tear a binding down pass the
/// pre-resolved `trigger_id` (read while the binding was live); `None` falls
/// back to resolving the still-live binding here.
#[allow(clippy::too_many_arguments)]
async fn emit_fired(
    deps: &Deps,
    spec: &ReactSpec,
    event: &Value,
    entry_id: &str,
    child_session_id: Option<&str>,
    retired: bool,
    trigger_id: Option<String>,
    join: Option<crate::subscriptions::fired::JoinProgress<'_>>,
    note: Option<&str>,
) {
    use crate::subscriptions::fired;
    let Some(owner) = spec.owner_session_id.as_deref() else {
        return;
    };
    let sub = spec.subscription_id.as_deref().unwrap_or("");
    let trigger_id = trigger_id.or_else(|| {
        spec.subscription_id
            .as_deref()
            .and_then(|s| deps.subscriptions.trigger_id_of(s))
    });
    let (scope, key) = fired::event_state_watch(event);
    let session = deps.session().await;
    fired::emit(
        &session,
        owner,
        entry_id,
        fired::TriggerFired {
            subscription_id: sub,
            trigger_id: trigger_id.as_deref(),
            target: if spec.call.is_some() { "call" } else { "spawn" },
            label: None,
            model: spec.model.as_deref(),
            once: spec.once,
            retired,
            scope,
            key,
            child_session_id,
            join,
            note,
            fired_at: fired::now_ms(),
        },
    )
    .await;
}

async fn unregister_subscription(deps: &Deps, id: &str) -> Result<(), HarnessError> {
    deps.iii
        .trigger(TriggerRequest {
            function_id: "engine::unregister_trigger".to_string(),
            payload: json!({ "id": id }),
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| HarnessError::Dependency(format!("engine::unregister_trigger: {e}")))?;
    Ok(())
}

async fn state_delete(deps: &Deps, key: &str) -> Result<(), HarnessError> {
    deps.iii
        .trigger(TriggerRequest {
            function_id: "state::delete".to_string(),
            payload: json!({ "scope": JOIN_SCOPE, "key": key }),
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| HarnessError::Dependency(format!("state::delete: {e}")))?;
    Ok(())
}

// --- pure helpers (unit-tested) ---------------------------------------------

fn single_event_task(base: &str, event: &Value) -> String {
    format!(
        "{base}\n\n<event>\n```json\n{}\n```\n</event>",
        pretty(event)
    )
}

fn gather_inputs_task(base: &str, rec: &Value) -> String {
    let results = rec.get("results").cloned().unwrap_or_else(|| json!({}));
    format!(
        "{base}\n\n<inputs>\n```json\n{}\n```\n</inputs>",
        pretty(&results)
    )
}

/// Reactive chains stop here: a react-spawned turn whose completion fires
/// react again past this depth is refused. Catches every runaway shape the
/// self-edge drop cannot (A→B→A ping-pong, instant-fail respawn storms).
pub const MAX_REACTIVE_DEPTH: u32 = 8;

/// Model ids currently served by the router, or `None` when the catalog is
/// unreachable (callers fail OPEN on `None` — a router blip must not block
/// registrations or reactions; a definitively unknown id must).
async fn known_model_ids(iii: &iii_sdk::IIIClient) -> Option<Vec<String>> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "router::models::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .ok()?;
    Some(parse_model_ids(&resp))
}

/// Validate `metadata.model` against the live router catalog. Models written
/// from memory (e.g. "gpt-4o" with no provider registered) would otherwise
/// make every reaction fail at spawn time — reject them at registration with
/// the valid ids in the error. Fails open when the catalog is unreachable.
pub async fn validate_model(
    iii: &std::sync::Arc<iii_sdk::IIIClient>,
    metadata: Option<&Value>,
) -> Result<(), String> {
    let Some(model) = metadata
        .and_then(|m| m.get("model"))
        .and_then(Value::as_str)
    else {
        return Ok(()); // shape errors are validate_spec's job
    };
    let Some(ids) = known_model_ids(iii).await else {
        tracing::warn!(
            model,
            "harness::react: model catalog unreachable; accepting unverified"
        );
        return Ok(());
    };
    if ids.iter().any(|id| id == model) {
        return Ok(());
    }
    let mut listed: Vec<&str> = ids.iter().map(String::as_str).take(8).collect();
    listed.sort_unstable();
    Err(format!(
        "unknown model \"{model}\" — not in router::models::list (never write model ids from \
         memory). Use the bare id; the provider goes in the separate `provider` field. \
         Available: {}{}",
        listed.join(", "),
        if ids.len() > 8 { ", …" } else { "" }
    ))
}

fn parse_model_ids(resp: &Value) -> Vec<String> {
    resp.get("models")
        .and_then(Value::as_array)
        .map(|ms| {
            ms.iter()
                .filter_map(|m| {
                    m.get("id")
                        .and_then(Value::as_str)
                        .or_else(|| m.as_str())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The firing event's reactive depth: 0 for organic events (state changes,
/// user-driven turns), N for the completion of a react-spawned turn.
fn event_reactive_depth(event: &Value) -> u32 {
    event
        .get("reactive_depth")
        .and_then(Value::as_u64)
        .unwrap_or(0) as u32
}

/// Classify only harness turn-completed payloads. Arbitrary state/cron events
/// may also contain a `status` field, so the session + turn identity is the
/// guard that keeps success gating specific to turn reactions.
fn turn_completion_failure(event: &Value) -> Option<String> {
    event.get("session_id").and_then(Value::as_str)?;
    event.get("turn_id").and_then(Value::as_str)?;
    let status = event.get("status").and_then(Value::as_str)?;
    let result_error = event.get("result_error").and_then(Value::as_str);
    if status == "completed" && result_error.is_none() {
        return None;
    }
    Some(
        event
            .get("reason")
            .and_then(Value::as_str)
            .or(result_error)
            .map(str::to_string)
            .unwrap_or_else(|| format!("status={status}")),
    )
}

fn join_failure_keys(rec: &Value) -> Vec<String> {
    let mut keys: Vec<String> = rec
        .get("failures")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default();
    keys.sort();
    keys
}

fn build_spawn_payload(task: String, spec: &ReactSpec, parent_session_id: Option<&str>) -> Value {
    let mut payload = json!({ "task": task, "model": spec.model });
    if let Some(sid) = &spec.session_id {
        payload["session_id"] = json!(sid);
    }
    if let Some(p) = &spec.provider {
        payload["provider"] = json!(p);
    }
    if let Some(o) = &spec.options {
        payload["options"] = o.clone();
    }
    // Interceptor-registered reactions inherit the REGISTRANT's dispatch
    // policy, subset when the spec narrows it (the in-turn child rule).
    // Without this the spawned turn is parentless and lands on the read-only
    // baseline — a wrap-up reaction delivered into the registrant's own chat
    // suddenly can't call what every earlier turn there could (rctest-k7m3:
    // database::query / state::set / engine::unregister_trigger all denied).
    // Raw engine-side registrations carry no stamp and keep the baseline.
    if let Some(registrant) = &spec.registrant_functions {
        let requested: Option<crate::types::turn::FunctionPolicy> = spec
            .options
            .as_ref()
            .and_then(|o| o.get("functions"))
            .and_then(|f| serde_json::from_value(f.clone()).ok());
        if let Some(effective) = crate::policy::subset_policy(Some(registrant), requested.as_ref())
        {
            if let Ok(v) = serde_json::to_value(&effective) {
                if !payload["options"].is_object() {
                    payload["options"] = json!({});
                }
                payload["options"]["functions"] = v;
            }
        }
    }
    if let Some(pp) = parent_session_id {
        payload["parent_session_id"] = json!(pp);
    }
    if let Some(sub) = &spec.subscription_id {
        payload["spawned_by_subscription_id"] = json!(sub);
    }
    payload
}

/// Every predecessor subscription id recorded in the join accumulator.
fn join_binding_ids(rec: &Value) -> Vec<String> {
    rec.get("bindings")
        .and_then(Value::as_object)
        .map(|m| {
            m.values()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

fn arrived_count(rec: &Value) -> usize {
    rec.get("arrived")
        .and_then(Value::as_object)
        .map(|m| m.len())
        .unwrap_or(0)
}

fn merge_op(path: &str, value: Value) -> Value {
    json!({ "type": "merge", "path": path, "value": value })
}

fn incr_op(path: &str, by: i64) -> Value {
    json!({ "type": "increment", "path": path, "by": by })
}

fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn join_canon_none_for_non_join_specs() {
        assert!(join_canon(None).is_none());
        let md = serde_json::json!({"model": "m", "task": "t"});
        assert!(join_canon(Some(&md)).is_none());
    }

    #[test]
    fn join_canon_strips_per_predecessor_fields_only() {
        let md = |key: &str, sub: &str| {
            serde_json::json!({
                "model": "m", "task": "the real task",
                "join": {"id": "j1", "expect": ["a", "b"], "key": key},
                "__subscription_id": sub, "__once": true,
                "__owner_session_id": "s1",
            })
        };
        let (id_a, canon_a) = join_canon(Some(&md("a", "sub_1"))).unwrap();
        let (id_b, canon_b) = join_canon(Some(&md("b", "sub_2"))).unwrap();
        assert_eq!(id_a, "j1");
        assert_eq!(id_b, "j1");
        // Same reaction, different key/stamps → identical fingerprint.
        assert_eq!(canon_a, canon_b);
        // The reaction fields survive; the per-predecessor ones are gone.
        assert_eq!(canon_a["task"], "the real task");
        assert_eq!(canon_a["join"]["expect"], serde_json::json!(["a", "b"]));
        assert!(canon_a["join"].get("key").is_none());
        assert!(canon_a.get("__subscription_id").is_none());
    }

    /// The rctest-x7k2 failure shape: full task on one predecessor,
    /// "placeholder" on another — the fingerprints must differ so the
    /// interceptor can reject the second registration.
    #[test]
    fn join_canon_differs_when_the_reaction_differs() {
        let full = serde_json::json!({
            "model": "m", "task": "verify and report",
            "join": {"id": "j1", "expect": ["w1", "w2"], "key": "w1"},
        });
        let placeholder = serde_json::json!({
            "model": "m", "task": "placeholder",
            "join": {"id": "j1", "expect": ["w1", "w2"], "key": "w2"},
        });
        let (_, canon_full) = join_canon(Some(&full)).unwrap();
        let (_, canon_ph) = join_canon(Some(&placeholder)).unwrap();
        assert_ne!(canon_full, canon_ph);
    }

    #[test]
    fn fire_gate_caps_per_key_within_window_and_recovers() {
        let gate = FireGate::default();
        let t0 = 1_000_000;
        for i in 0..MAX_FIRES_PER_WINDOW {
            assert!(gate.admit("sub-1", t0 + i as i64), "fire {i} must pass");
        }
        // Budget exhausted inside the window.
        assert!(!gate.admit("sub-1", t0 + 100));
        // Other keys are unaffected.
        assert!(gate.admit("sub-2", t0 + 100));
        // Once the window slides past the early fires, the key recovers.
        assert!(gate.admit("sub-1", t0 + FIRE_WINDOW_MS + 1));
    }

    /// The coalescing contract: capped fires park (newest event wins, drops
    /// accumulate), exactly one caller per drain cycle owns scheduling, and
    /// draining resets the cycle. Without this, a burst's TAIL is exactly
    /// what gets dropped and recompute-style reactions freeze one step
    /// behind the source of truth (rctest-k7m3 postmortem).
    #[test]
    fn fire_gate_coalesces_capped_fires_newest_wins() {
        let gate = FireGate::default();
        let t0 = 1_000_000;
        for i in 0..MAX_FIRES_PER_WINDOW {
            assert!(gate.admit("sub-1", t0 + i as i64));
        }
        assert!(!gate.admit("sub-1", t0 + 100));

        // First deferral owns scheduling; the delay lands when the oldest
        // window entry expires.
        let d1 = gate.defer(
            "sub-1",
            serde_json::json!({"seq": 11}),
            serde_json::json!({"task": "t"}),
            t0 + 100,
        );
        assert_eq!(d1.dropped, 1);
        let delay = d1.schedule_delay_ms.expect("first deferral schedules");
        assert_eq!(delay, FIRE_WINDOW_MS - 100);

        // Later deferrals only update the pending slot.
        let d2 = gate.defer(
            "sub-1",
            serde_json::json!({"seq": 12}),
            serde_json::json!({"task": "t"}),
            t0 + 200,
        );
        assert_eq!(d2.dropped, 2);
        assert!(
            d2.schedule_delay_ms.is_none(),
            "one trailing task per cycle"
        );

        // The trailing task drains the NEWEST event with the full drop count.
        let pending = gate.take_pending("sub-1").expect("pending fire parked");
        assert_eq!(pending.event["seq"], 12);
        assert_eq!(pending.dropped, 2);

        // Drained: nothing pending, and a new deferral starts a new cycle.
        assert!(gate.take_pending("sub-1").is_none());
        let d3 = gate.defer(
            "sub-1",
            serde_json::json!({"seq": 13}),
            serde_json::Value::Null,
            t0 + 300,
        );
        assert_eq!(d3.dropped, 1, "drop count resets after a drain");
        assert!(d3.schedule_delay_ms.is_some(), "new cycle reschedules");
    }

    #[test]
    fn fire_gate_gc_keeps_keys_with_parked_fires() {
        let gate = FireGate::default();
        let t0 = 1_000_000;
        gate.admit("parked", t0);
        gate.defer("parked", serde_json::json!({}), serde_json::Value::Null, t0);
        // Blow past the GC threshold with dead keys, long after the window.
        let later = t0 + 10 * FIRE_WINDOW_MS;
        for i in 0..1030 {
            gate.admit(&format!("dead-{i}"), later);
        }
        gate.admit("trigger-gc", later);
        assert!(
            gate.take_pending("parked").is_some(),
            "GC must never drop a key holding a parked trailing fire"
        );
    }

    #[test]
    fn join_rearm_parses_and_defaults_off() {
        let j: JoinSpec = serde_json::from_value(json!({
            "id": "J", "expect": ["a"], "key": "a"
        }))
        .unwrap();
        assert!(!j.rearm);
        let j: JoinSpec = serde_json::from_value(json!({
            "id": "J", "expect": ["a"], "key": "a", "rearm": true
        }))
        .unwrap();
        assert!(j.rearm);
    }

    fn spec() -> ReactSpec {
        ReactSpec {
            model: Some("claude-sonnet-5".into()),
            task: Some("summarize".into()),
            call: None,
            session_id: Some("s_run".into()),
            provider: None,
            options: None,
            parent_session_id: None,
            continue_on_error: false,
            join: None,
            subscription_id: None,
            once: false,
            owner_session_id: None,
            registrant_functions: None,
        }
    }

    /// The rctest-k7m3 wrap-up failure: a reaction registered with no
    /// `options` used to spawn parentless onto the read-only baseline —
    /// denied database::query / state::set / engine::unregister_trigger in
    /// the very chat whose earlier turns could call all three. With the
    /// registrant stamp, the reaction inherits that policy; explicit options
    /// subset it and can never escalate past it.
    #[test]
    fn spawn_payload_inherits_registrant_policy_and_subsets_requests() {
        let registrant = crate::types::turn::FunctionPolicy {
            allow: vec!["database::query".into(), "state::set".into()],
            deny: vec!["shell::run".into()],
            expose: Default::default(),
        };

        // No options: full inheritance.
        let mut s = spec();
        s.registrant_functions = Some(registrant.clone());
        let payload = build_spawn_payload("t".into(), &s, None);
        assert_eq!(
            payload["options"]["functions"]["allow"],
            json!(["database::query", "state::set"])
        );
        assert_eq!(
            payload["options"]["functions"]["deny"],
            json!(["shell::run"])
        );

        // Requested subset survives; escalation beyond the registrant is
        // filtered out.
        let mut s = spec();
        s.registrant_functions = Some(registrant);
        s.options = Some(json!({
            "functions": { "allow": ["state::set", "engine::unregister_trigger"] },
            "max_turns": 3
        }));
        let payload = build_spawn_payload("t".into(), &s, None);
        assert_eq!(
            payload["options"]["functions"]["allow"],
            json!(["state::set"]),
            "an option the registrant never held must not survive"
        );
        assert_eq!(
            payload["options"]["max_turns"],
            json!(3),
            "other options pass through"
        );

        // No stamp (raw engine-side registration): payload untouched, the
        // read-only baseline fallback stays in force downstream.
        let s = spec();
        let payload = build_spawn_payload("t".into(), &s, None);
        assert!(payload.get("options").is_none());
    }

    #[test]
    fn validate_spec_call_mode_exclusivity_and_spawn_knob_rejection() {
        // A call reaction needs no model/task.
        assert!(validate_spec(Some(&json!({
            "call": { "function_id": "fp::pipe", "payload": { "through": [] } }
        })))
        .is_ok());
        // Join downstreams may be calls too.
        assert!(validate_spec(Some(&json!({
            "call": { "function_id": "state::set" },
            "join": { "id": "J", "expect": ["a"], "key": "a" }
        })))
        .is_ok());
        // Exactly one of task | call.
        assert!(validate_spec(Some(&json!({})))
            .unwrap_err()
            .contains("exactly one reaction"));
        assert!(validate_spec(Some(&json!({
            "model": "m", "task": "t", "call": { "function_id": "state::set" }
        })))
        .unwrap_err()
        .contains("mutually exclusive"));
        // Task mode still requires a model.
        assert!(validate_spec(Some(&json!({ "task": "t" })))
            .unwrap_err()
            .contains("needs a `model`"));
        // Spawn-only knobs are rejected in call mode, teachably.
        for knob in [
            json!({ "call": { "function_id": "f::g" }, "model": "m" }),
            json!({ "call": { "function_id": "f::g" }, "session_id": "s" }),
            json!({ "call": { "function_id": "f::g" }, "options": {} }),
            json!({ "call": { "function_id": "f::g" }, "parent_session_id": "p" }),
        ] {
            assert!(validate_spec(Some(&knob))
                .unwrap_err()
                .contains("task-mode"));
        }
        assert!(
            validate_spec(Some(&json!({ "call": { "function_id": "" } })))
                .unwrap_err()
                .contains("must name the function")
        );
    }

    #[test]
    fn call_dispatch_strips_an_authored_fs_scope_from_stamped_targets() {
        // The dispatch_call composition: a model-authored scope in the call
        // payload — literal, or threaded to /fs_scope via event_into — must
        // not survive to a stamped target (the reaction path never runs the
        // turn loop's trusted stamp, so anything left here would arrive at
        // shell/fp as trusted).
        let forged = json!({ "root": "/", "grants": ["/"], "boundary": "configured_roots" });
        let authored = json!({ "path": "/etc/hosts", "fs_scope": forged });
        let threaded = inject_at(authored, "/fs_scope", json!({ "root": "/" })).unwrap();
        let out = crate::filesystem_scope::inject(
            "shell::fs::read",
            threaded,
            None,
            &[],
            crate::filesystem_scope::FilesystemBoundary::ConfiguredRoots,
        );
        assert_eq!(out, json!({ "path": "/etc/hosts" }));
        // fp::pipe is a stamped target too; non-stamped targets pass through.
        let out = crate::filesystem_scope::inject(
            "fp::pipe",
            json!({ "through": [], "fs_scope": { "root": "/" } }),
            None,
            &[],
            crate::filesystem_scope::FilesystemBoundary::ConfiguredRoots,
        );
        assert_eq!(out, json!({ "through": [] }));
        let unscoped = json!({ "scope": "s", "key": "k", "fs_scope": { "root": "/" } });
        let out = crate::filesystem_scope::inject(
            "state::set",
            unscoped.clone(),
            None,
            &[],
            crate::filesystem_scope::FilesystemBoundary::ConfiguredRoots,
        );
        assert_eq!(out, unscoped);
    }

    #[test]
    fn inject_at_places_the_event_and_creates_intermediates() {
        let out = inject_at(
            json!({ "through": [1] }),
            "/event",
            json!({ "session_id": "s" }),
        )
        .unwrap();
        assert_eq!(
            out,
            json!({ "through": [1], "event": { "session_id": "s" } })
        );
        // Nested pointer creates intermediate objects; null base becomes {}.
        let out = inject_at(Value::Null, "/meta/event", json!(1)).unwrap();
        assert_eq!(out, json!({ "meta": { "event": 1 } }));
        // Teachable failures: bad pointer, non-object payload.
        assert!(inject_at(json!({}), "value", json!(1))
            .unwrap_err()
            .contains("JSON pointer"));
        assert!(inject_at(json!([1]), "/event", json!(1))
            .unwrap_err()
            .contains("must be an object"));
    }

    #[test]
    fn once_stamp_parses_and_defaults_off() {
        let s: ReactSpec = serde_json::from_value(json!({ "model": "m", "task": "t" })).unwrap();
        assert!(!s.once);
        let s: ReactSpec =
            serde_json::from_value(json!({ "model": "m", "task": "t", "__once": true })).unwrap();
        assert!(s.once);
    }

    #[test]
    fn continue_on_error_is_explicit_and_defaults_off() {
        let s: ReactSpec = serde_json::from_value(json!({ "model": "m", "task": "t" })).unwrap();
        assert!(!s.continue_on_error);
        let s: ReactSpec = serde_json::from_value(json!({
            "model": "m", "task": "t", "continue_on_error": true
        }))
        .unwrap();
        assert!(s.continue_on_error);
    }

    #[test]
    fn reaction_outcome_ids_are_idempotent_per_event_and_outcome() {
        let mut s = spec();
        s.subscription_id = Some("sub-1".into());
        let event = json!({
            "session_id": "s-upstream",
            "turn_id": "t-1",
            "timestamp": 42,
        });
        assert_eq!(
            reaction_outcome_entry_id(&s, &event, "blocked"),
            reaction_outcome_entry_id(&s, &event, "blocked")
        );
        assert_ne!(
            reaction_outcome_entry_id(&s, &event, "blocked"),
            reaction_outcome_entry_id(&s, &event, "spawned")
        );
    }

    #[test]
    fn simple_event_task_embeds_event() {
        let ev = json!({ "session_id": "s_child", "turn_id": "t1", "status": "completed" });
        let t = single_event_task(spec().task.as_deref().unwrap(), &ev);
        assert!(t.contains("summarize"));
        assert!(t.contains("\"turn_id\""));
        assert!(t.contains("<event>"));
    }

    #[test]
    fn spawn_payload_has_required_fields_no_send_leftovers() {
        let p = build_spawn_payload("do it".into(), &spec(), None);
        assert_eq!(p["model"], "claude-sonnet-5");
        assert_eq!(p["task"], "do it");
        assert_eq!(p["session_id"], "s_run");
        assert!(p.get("idempotency_key").is_none());
        assert!(p.get("message").is_none());
        assert!(p.get("parent_session_id").is_none());
    }

    #[test]
    fn options_and_provider_pass_through() {
        let mut s = spec();
        s.provider = Some("anthropic".into());
        s.options = Some(json!({ "functions": { "allow": ["shell::*"] }, "max_turns": 4 }));
        let p = build_spawn_payload("t".into(), &s, None);
        assert_eq!(p["provider"], "anthropic");
        assert_eq!(p["options"]["max_turns"], 4);
    }

    #[test]
    fn parent_session_id_passes_through_for_tree() {
        let p = build_spawn_payload("t".into(), &spec(), Some("s_root"));
        assert_eq!(p["parent_session_id"], "s_root");
    }

    #[test]
    fn reaction_delivery_session_prefers_pin_then_owner_stamp() {
        // Applies uniformly to both dispatch paths: a simple (non-join) edge
        // and a join's downstream spawn both resolve through this function.
        let mut s = spec();
        s.owner_session_id = Some("console-owner".into());
        // Explicit pin wins.
        assert_eq!(reaction_delivery_session(&s).as_deref(), Some("s_run"));
        // No pin: a non-join reaction lands in the chat that wired it, same
        // as a join's fan-in result would.
        s.session_id = None;
        assert_eq!(
            reaction_delivery_session(&s).as_deref(),
            Some("console-owner")
        );
        // Raw registration without an owner stamp: fresh child stands.
        s.owner_session_id = None;
        assert_eq!(reaction_delivery_session(&s), None);
    }

    #[test]
    fn parent_anchor_prefers_event_session_then_owner_stamp() {
        let mut s = spec();
        s.owner_session_id = Some("s_owner".into());
        // Turn events carry the firing session — it wins.
        let ev = json!({ "session_id": "s_firing" });
        assert_eq!(parent_anchor(&ev, &s), Some("s_firing".into()));
        // State/cron/stream events carry no session — the registering session anchors.
        let ev = json!({ "scope": "research", "key": "article" });
        assert_eq!(parent_anchor(&ev, &s), Some("s_owner".into()));
        // Neither: no anchor, spawn stays a root.
        s.owner_session_id = None;
        assert_eq!(parent_anchor(&ev, &s), None);
    }

    #[test]
    fn owner_session_stamp_deserializes() {
        let s: ReactSpec = serde_json::from_value(json!({
            "model": "m", "task": "t", "__owner_session_id": "console-abc"
        }))
        .unwrap();
        assert_eq!(s.owner_session_id.as_deref(), Some("console-abc"));
    }

    #[test]
    fn merge_and_increment_op_wire_shapes() {
        assert_eq!(
            merge_op("results", json!({ "x1": 1 })),
            json!({ "type": "merge", "path": "results", "value": { "x1": 1 } })
        );
        assert_eq!(
            incr_op("fire", 1),
            json!({ "type": "increment", "path": "fire", "by": 1 })
        );
    }

    #[test]
    fn arrived_count_reads_accumulator() {
        let rec = json!({ "arrived": { "x1": true, "x2": true }, "results": {} });
        assert_eq!(arrived_count(&rec), 2);
        assert_eq!(arrived_count(&json!({})), 0);
    }

    #[test]
    fn turn_completion_failure_gates_failed_and_invalid_results_only() {
        assert_eq!(
            turn_completion_failure(&json!({
                "session_id": "s", "turn_id": "t", "status": "failed",
                "reason": "provider stream ended"
            }))
            .as_deref(),
            Some("provider stream ended")
        );
        assert_eq!(
            turn_completion_failure(&json!({
                "session_id": "s", "turn_id": "t", "status": "completed",
                "result_error": "schema mismatch"
            }))
            .as_deref(),
            Some("schema mismatch")
        );
        assert!(turn_completion_failure(&json!({
            "session_id": "s", "turn_id": "t", "status": "completed",
            "result": "ok"
        }))
        .is_none());
        // A state payload with its own status field is not a turn completion.
        assert!(turn_completion_failure(&json!({ "status": "failed" })).is_none());
    }

    #[test]
    fn join_failure_keys_are_sorted_and_optional() {
        assert_eq!(
            join_failure_keys(&json!({ "failures": { "fetch": "timeout", "cache": "denied" } })),
            vec!["cache".to_string(), "fetch".to_string()]
        );
        assert!(join_failure_keys(&json!({})).is_empty());
    }

    #[test]
    fn gather_inputs_feeds_all_predecessor_results() {
        let rec = json!({
            "results": { "x1": { "result": "A" }, "x2": { "result": "B" } },
            "arrived": { "x1": true, "x2": true }
        });
        let t = gather_inputs_task("combine", &rec);
        assert!(t.contains("combine"));
        assert!(t.contains("\"x1\""));
        assert!(t.contains("\"x2\""));
        assert!(t.contains("<inputs>"));
    }

    #[test]
    fn join_spec_parses_from_metadata() {
        let s: ReactSpec = serde_json::from_value(json!({
            "model": "claude-sonnet-5",
            "task": "combine",
            "join": { "id": "J", "expect": ["x1", "x2"], "key": "x1" }
        }))
        .unwrap();
        let j = s.join.unwrap();
        assert_eq!(j.id, "J");
        assert_eq!(j.expect, vec!["x1", "x2"]);
        assert_eq!(j.key, "x1");
    }

    #[test]
    fn missing_spec_notes_not_spawned() {
        let r = ReactResult::note("no metadata spec");
        assert!(!r.spawned);
        assert_eq!(r.note.as_deref(), Some("no metadata spec"));
    }

    #[test]
    fn parse_model_ids_reads_objects_and_bare_strings() {
        let ids = parse_model_ids(&json!({
            "models": [ { "id": "claude-sonnet-5" }, "bare-id", { "no_id": true } ]
        }));
        assert_eq!(
            ids,
            vec!["claude-sonnet-5".to_string(), "bare-id".to_string()]
        );
        assert!(parse_model_ids(&json!({})).is_empty());
    }

    #[test]
    fn event_reactive_depth_defaults_to_zero_and_reads_value() {
        assert_eq!(event_reactive_depth(&json!({})), 0);
        assert_eq!(event_reactive_depth(&json!({ "reactive_depth": 3 })), 3);
        assert_eq!(event_reactive_depth(&json!({ "reactive_depth": "x" })), 0);
    }

    #[test]
    fn spawn_payload_stamps_spawning_subscription() {
        let mut s = spec();
        s.subscription_id = Some("sub-7".into());
        let p = build_spawn_payload("t".into(), &s, None);
        assert_eq!(p["spawned_by_subscription_id"], "sub-7");
        let p = build_spawn_payload("t".into(), &spec(), None);
        assert!(p.get("spawned_by_subscription_id").is_none());
    }

    #[test]
    fn join_binding_ids_collects_recorded_subscriptions() {
        let rec = json!({
            "bindings": { "x1": "sub-1", "x2": "sub-2" },
            "arrived": { "x1": true, "x2": true }
        });
        let mut ids = join_binding_ids(&rec);
        ids.sort();
        assert_eq!(ids, vec!["sub-1".to_string(), "sub-2".to_string()]);
        assert!(join_binding_ids(&json!({})).is_empty());
    }

    #[test]
    fn sidecar_subscription_id_parses_and_is_optional() {
        let s: ReactSpec = serde_json::from_value(json!({
            "model": "m", "task": "t", "__subscription_id": "sub-9"
        }))
        .unwrap();
        assert_eq!(s.subscription_id.as_deref(), Some("sub-9"));
        let s: ReactSpec = serde_json::from_value(json!({ "model": "m", "task": "t" })).unwrap();
        assert!(s.subscription_id.is_none());
    }

    #[test]
    fn validate_spec_accepts_simple_and_join() {
        assert!(validate_spec(Some(&json!({ "model": "m", "task": "t" }))).is_ok());
        assert!(validate_spec(Some(&json!({
            "model": "m", "task": "t",
            "join": { "id": "J", "expect": ["a", "b"], "key": "a" }
        })))
        .is_ok());
    }

    #[test]
    fn validate_spec_rejects_expect_as_count() {
        let err = validate_spec(Some(&json!({
            "model": "m", "task": "t",
            "join": { "id": "J", "expect": 3, "key": "a" }
        })))
        .unwrap_err();
        assert!(err.contains("not a count"), "{err}");
    }

    #[test]
    fn validate_spec_rejects_key_outside_expect_and_empty_expect() {
        let err = validate_spec(Some(&json!({
            "model": "m", "task": "t",
            "join": { "id": "J", "expect": ["a", "b"], "key": "z" }
        })))
        .unwrap_err();
        assert!(err.contains("not in `expect`"), "{err}");

        let err = validate_spec(Some(&json!({
            "model": "m", "task": "t",
            "join": { "id": "J", "expect": [], "key": "a" }
        })))
        .unwrap_err();
        assert!(err.contains("every predecessor key"), "{err}");
    }

    #[test]
    fn validate_spec_rejects_missing_metadata_and_missing_task() {
        assert!(validate_spec(None).unwrap_err().contains("metadata"));
        let err = validate_spec(Some(&json!({ "model": "m" }))).unwrap_err();
        assert!(err.contains("task"), "{err}");
    }
}

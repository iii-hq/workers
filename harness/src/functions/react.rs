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

/// Sliding-window fire counter per subscription (or per spec-hash for
/// bindings that carry no `__subscription_id`, e.g. `state`-provider ones).
#[derive(Default)]
pub struct FireGate {
    inner: std::sync::Mutex<std::collections::HashMap<String, std::collections::VecDeque<i64>>>,
}

impl FireGate {
    /// Record a fire attempt for `key` at `now_ms`; `false` when the key has
    /// exhausted its window budget — the caller must refuse to react.
    pub fn admit(&self, key: &str, now_ms: i64) -> bool {
        let mut map = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        // Opportunistic GC so dead keys can't grow the map unboundedly.
        if map.len() > 1024 {
            map.retain(|_, q| q.back().is_some_and(|t| now_ms - t < FIRE_WINDOW_MS));
        }
        let q = map.entry(key.to_string()).or_default();
        while q.front().is_some_and(|t| now_ms - t >= FIRE_WINDOW_MS) {
            q.pop_front();
        }
        if q.len() >= MAX_FIRES_PER_WINDOW {
            return false;
        }
        q.push_back(now_ms);
        true
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

/// The sub-agent to spawn when the subscription fires, carried in the trigger's
/// `metadata` and delivered to this handler as the metadata sidecar.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ReactSpec {
    /// Model for the reacting sub-agent (required by `harness::spawn`). Agent
    /// registrations that omit it inherit the registering turn's model, stamped
    /// by the `engine::register_trigger` interceptor.
    pub model: String,
    /// The sub-agent's opening task; the event (simple) or all predecessor
    /// results (join) are appended fenced so it sees its inputs.
    pub task: String,
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
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReactResult {
    /// Whether a `harness::spawn` was dispatched this call.
    pub spawned: bool,
    /// The spawned sub-agent's child session id, when spawn returned one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub child_session_id: Option<String>,
    /// Why nothing spawned (missing spec, join not yet complete, already fired,
    /// error). Present iff `!spawned`.
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
            child_session_id: child,
            note: None,
            child_turn_id: turn,
        }
    }
    fn note(msg: impl Into<String>) -> Self {
        Self {
            spawned: false,
            child_session_id: None,
            note: Some(msg.into()),
            child_turn_id: None,
        }
    }
}

/// Registration-time validation for subscriptions targeting `harness::react`:
/// once bound, a bad spec would only surface as a silent no-op when the event
/// fires, so reject it loudly at `engine::register_trigger` time instead.
pub fn validate_spec(metadata: Option<&Value>) -> Result<(), String> {
    let Some(m) = metadata else {
        return Err(
            "harness::react needs the sub-agent spec in the registration `metadata`: \
             { model, task, session_id?, join?: { id, expect: [\"key\", ...], key } }"
                .into(),
        );
    };
    let spec: ReactSpec = serde_json::from_value(m.clone()).map_err(|e| {
        format!(
            "invalid harness::react metadata spec: {e}. Expected \
             {{ model, task, session_id?, join?: {{ id, expect: [\"key\", ...], key }} }} — \
             `join.expect` is the array of ALL predecessor keys, not a count."
        )
    })?;
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

pub async fn handle(
    deps: &Deps,
    event: Value,
    metadata: Option<Value>,
) -> Result<ReactResult, HarnessError> {
    // A bad/absent spec must never error out: an erroring trigger target just
    // spams the engine's dispatch log. Log and no-op instead.
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
        tracing::warn!(
            subscription = %gate_key,
            "harness::react: fire-rate breaker tripped ({MAX_FIRES_PER_WINDOW} fires/{FIRE_WINDOW_MS}ms); not reacting"
        );
        let note = format!(
            "fire-rate cap ({MAX_FIRES_PER_WINDOW} per {FIRE_WINDOW_MS}ms) reached for this subscription; not spawning"
        );
        record_reaction_outcome(deps, &spec, &event, "failed", &note, None).await;
        return Ok(ReactResult::note(note));
    }

    // Fire-time model check: registrations made through OTHER trigger-type
    // providers (e.g. `state`) never pass the harness's registration-time
    // validation, so a memory-written model id ("gpt-4o") would spawn a turn
    // that instantly fails on every event. Refuse to spawn instead; fail open
    // when the catalog is unreachable.
    if let Some(ids) = known_model_ids(&deps.iii).await {
        if !ids.iter().any(|id| id == &spec.model) {
            tracing::warn!(
                model = %spec.model,
                "harness::react: unknown model in reaction spec; not spawning"
            );
            let note = format!(
                "unknown model \"{}\" (not in router::models::list); not spawning",
                spec.model
            );
            record_reaction_outcome(deps, &spec, &event, "failed", &note, None).await;
            return Ok(ReactResult::note(note));
        }
    }

    // Loop breaker #2 (backstop): every react-spawned turn carries a reactive
    // depth; its completion event echoes it. A chain past the cap is refused
    // no matter which subscriptions form it (self-edge, A→B→A ping-pong,
    // instant-fail respawn storms).
    let incoming_depth = event_reactive_depth(&event);
    if incoming_depth >= MAX_REACTIVE_DEPTH {
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
        None => {
            let res = spawn_reaction(
                deps,
                single_event_task(&spec.task, &event),
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

    // Fire the downstream sub-agent fed ALL predecessors' results, then GC the
    // accumulator record. The delivery session (owner fallback when unpinned)
    // is already resolved by the caller. Joins fire once, so this cannot spam.
    let task = gather_inputs_task(&spec.task, &rec);
    let res = spawn_reaction(deps, task, spec, parent, spawn_depth).await;
    // The join committed: the downstream spawned and (unless re-armed) the
    // predecessors were torn down above — `retired` carries the real outcome,
    // so a failed unregister is never reported as gone. One completion record
    // lets the console mark the whole join fired + retired and post the
    // notice. Gated on `spawned` like the simple edge — spawn_reaction
    // swallows dispatch errors into `spawned: false`, and a record claiming
    // "spawned" for a spawn that never happened would mislead the chat.
    if let Ok(r) = &res {
        if r.spawned {
            let note = format!("{expected}/{expected} arrived — spawned");
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
        let status = if outcome.spawned { "spawned" } else { "failed" };
        let summary = outcome.note.clone().unwrap_or_else(|| {
            outcome
                .child_session_id
                .as_deref()
                .map(|id| format!("Join {} spawned child session {id}.", join.id))
                .unwrap_or_else(|| format!("Join {} spawned its downstream turn.", join.id))
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
                model = %spec.model,
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
            target: "spawn",
            label: None,
            model: Some(&spec.model),
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
            model: "claude-sonnet-5".into(),
            task: "summarize".into(),
            session_id: Some("s_run".into()),
            provider: None,
            options: None,
            parent_session_id: None,
            continue_on_error: false,
            join: None,
            subscription_id: None,
            once: false,
            owner_session_id: None,
        }
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
        let t = single_event_task(&spec().task, &ev);
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

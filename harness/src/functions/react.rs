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
    /// Model for the reacting sub-agent (required by `harness::spawn`).
    pub model: String,
    /// The sub-agent's opening task; the event (simple) or all predecessor
    /// results (join) are appended fenced so it sees its inputs.
    pub task: String,
    /// Spawn into this session (e.g. a fork); omit for a fresh child session.
    /// Exception: a completed JOIN's downstream defaults to the registering
    /// session when omitted, so the fan-in result lands back in that chat.
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
    /// When present, this subscription is one predecessor of a join barrier.
    #[serde(default)]
    pub join: Option<JoinSpec>,
    /// Stamped by the harness's turn-event fan-out (never caller-supplied): the
    /// firing subscription's registration id, so a completed join can
    /// auto-unregister its predecessor subscriptions.
    #[serde(default, rename = "__subscription_id")]
    pub subscription_id: Option<String>,
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
}

impl ReactResult {
    fn spawned(child: Option<String>) -> Self {
        Self {
            spawned: true,
            child_session_id: child,
            note: None,
        }
    }
    fn note(msg: impl Into<String>) -> Self {
        Self {
            spawned: false,
            child_session_id: None,
            note: Some(msg.into()),
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
        (&spec.model, &spec.task, &spec.session_id).hash(&mut h);
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
        return Ok(ReactResult::note(format!(
            "fire-rate cap ({MAX_FIRES_PER_WINDOW} per {FIRE_WINDOW_MS}ms) reached for this subscription; not spawning"
        )));
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
            return Ok(ReactResult::note(format!(
                "unknown model \"{}\" (not in router::models::list); not spawning",
                spec.model
            )));
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
        return Ok(ReactResult::note(format!(
            "reactive depth cap ({MAX_REACTIVE_DEPTH}) reached; not spawning"
        )));
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

    match spec.join.clone() {
        None => {
            spawn_reaction(
                deps,
                single_event_task(&spec.task, &event),
                &spec,
                parent,
                spawn_depth,
            )
            .await
        }
        Some(join) => join_edge(deps, event, &spec, &join, parent, spawn_depth).await,
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
) -> Result<ReactResult, HarnessError> {
    // Step 1 — record this predecessor idempotently (Merge, so re-delivery of
    // the same key overwrites its own slot and never inflates the count), and
    // read the accumulator back.
    let mut ops = vec![
        merge_op("results", json!({ &join.key: event })),
        merge_op("arrived", json!({ &join.key: true })),
    ];
    if let Some(sid) = &spec.subscription_id {
        ops.push(merge_op("bindings", json!({ &join.key: sid })));
    }
    let rec = match state_update(deps, &join.id, ops).await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, join = %join.id, "harness::react: join record update failed");
            return Ok(ReactResult::note(format!("join update failed: {e}")));
        }
    };

    let arrived = arrived_count(&rec);
    let expected = join.expect.len();
    if arrived < expected {
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
            return Ok(ReactResult::note(format!("join fire-guard failed: {e}")));
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
    if join.rearm {
        tracing::info!(join = %join.id, "harness::react: join re-armed; predecessor subscriptions stay registered");
    } else {
        for id in join_binding_ids(&rec) {
            // Turn-event edges record the ENGINE binding id (stamped by the
            // fan-out); state/cron/stream edges record the interceptor's
            // local `sub_` handle — resolve it through the registry first.
            let engine_id = if id.starts_with("sub_") {
                deps.subscriptions.take(&id).and_then(|(_, t)| t)
            } else {
                deps.subscriptions.take_by_trigger_id(&id);
                Some(id.clone())
            };
            let Some(engine_id) = engine_id else {
                tracing::warn!(join = %join.id, subscription = %id, "harness::react: join predecessor has no resolvable engine binding; skipping unregister");
                continue;
            };
            if let Err(e) = unregister_subscription(deps, &engine_id).await {
                tracing::warn!(error = %e, join = %join.id, subscription = %engine_id, "harness::react: join subscription auto-unregister failed");
            }
        }
    }

    // Fire the downstream sub-agent fed ALL predecessors' results, then GC the
    // accumulator record. A fan-in's result belongs to whoever wired the
    // pipeline: without an explicit `session_id` pin, deliver INTO the owner
    // session — a new turn in the chat that registered the join — instead of
    // a detached child nobody reads. Joins fire once, so this cannot spam.
    let mut spec = spec.clone();
    spec.session_id = join_delivery_session(&spec);
    let task = gather_inputs_task(&spec.task, &rec);
    let res = spawn_reaction(deps, task, &spec, parent, spawn_depth).await;
    if let Err(e) = state_delete(deps, &join.id).await {
        tracing::warn!(error = %e, join = %join.id, "harness::react: join record cleanup failed");
    }
    res
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
            tracing::info!(
                child_session_id = child.as_deref(),
                model = %spec.model,
                subscription = spec.subscription_id.as_deref(),
                reactive_depth,
                "harness::react: reaction spawned"
            );
            Ok(ReactResult::spawned(child))
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

/// The session a completed join's downstream spawns into: an explicit spec pin
/// wins; otherwise the registering session (the pipeline's owner), so the
/// fan-in result lands as a turn in the chat that wired it. `None` (a fresh
/// detached child) only for raw registrations that carry no owner stamp.
fn join_delivery_session(spec: &ReactSpec) -> Option<String> {
    spec.session_id
        .clone()
        .or_else(|| spec.owner_session_id.clone())
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
            join: None,
            subscription_id: None,
            owner_session_id: None,
        }
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
    fn join_downstream_delivers_into_the_owner_session_by_default() {
        let mut s = spec();
        s.owner_session_id = Some("console-owner".into());
        // Explicit pin wins.
        assert_eq!(join_delivery_session(&s).as_deref(), Some("s_run"));
        // No pin: the fan-in result lands in the chat that wired the join.
        s.session_id = None;
        assert_eq!(join_delivery_session(&s).as_deref(), Some("console-owner"));
        // Raw registration without an owner stamp: fresh child stands.
        s.owner_session_id = None;
        assert_eq!(join_delivery_session(&s), None);
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

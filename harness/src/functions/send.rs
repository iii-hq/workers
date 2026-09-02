//! `harness::send` — the entry point: ensure the session, persist the incoming
//! user message, and CAS-seed the first turn step (or merge into a running
//! turn). Returns fast (harness.md § `harness::send`).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::WorkerConfig;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::ids;
use crate::policy;
use crate::prompt::{self, Mode, SystemPromptOpts, SystemPromptStrategy};
use crate::turn_loop;
use crate::types::message::{AgentMessage, UserMessage, UserRoleTag};
use crate::types::model::ThinkingLevel;
use crate::types::output::OutputContract;
use crate::types::turn::{
    FunctionPolicy, IdemRecord, ParentLink, SkillContext, TurnOptions, TurnRecord, TurnStatus,
};

/// `message` is either a plain string (sugar for a user text message) or a
/// full `AgentMessage`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MessageInput {
    Text(String),
    Message(Box<AgentMessage>),
}

impl From<String> for MessageInput {
    fn from(text: String) -> Self {
        MessageInput::Text(text)
    }
}

/// Per-send options frozen onto the turn record (harness.md § `harness::send`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SendOptions {
    /// Run the session as a directory agent profile (`directory::agents::*`
    /// id). Session-creating sends only — the profile's resolved system
    /// prompt (its `extends` chain composed by the directory) REPLACES the
    /// built-in identity (only the `mode` paragraph is prepended), its skill
    /// filter becomes the session's skill selection, its `model` is the
    /// fallback when this send names none, and its identity sticks like the
    /// system prompt (later sends inherit; naming an explicit prompt field
    /// sheds it). Refused on an existing session, combined with either
    /// prompt field, or when the profile's `extends` chain does not resolve.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// How `system_prompt` combines with the built-in prompt: `override`
    /// replaces it; `enrich` (default) appends to it; `disabled` omits it.
    /// When BOTH prompt fields are omitted on an existing session, the prior
    /// turn's resolved prompt is inherited; naming a strategy (even bare)
    /// resolves fresh — the reset-to-default escape hatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt_strategy: Option<SystemPromptStrategy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
    /// Per-generation output-token ceiling forwarded to the router.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    /// Hard input-plus-output token budget for the complete root-and-subagent
    /// session tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_tokens: Option<u64>,
    /// Hard USD budget for the complete root-and-subagent session tree.
    /// Every model used by the tree must advertise catalog pricing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_usd: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    /// Provider-native per-call options, namespaced by provider id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<BTreeMap<String, Value>>,
    /// The turn's deliverable; default `{ type: "text" }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputContract>,
    /// The fail-closed dispatch policy. Omitted on a NEW session → deny every
    /// call; omitted when steering an EXISTING session → inherit the prior
    /// turn's policy (a nudge must not disarm a live run). Pass
    /// `{ allow: [] }` to strip explicitly. On a NEW `ask`-mode turn the
    /// effective policy is capped at the configured default policy; a
    /// steer folded into an already-running turn keeps that turn's frozen
    /// policy until it finalises.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions: Option<FunctionPolicy>,
    /// Exact skill ids advertised to the model. On a fresh session, omitted or
    /// empty means all. On an existing session, omitted inherits its filter and
    /// empty resets to all. Explicit changes require no active turn. This is
    /// index curation, not authorization.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    /// Tracing passthrough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
    /// Per-turn override of the configured validation-retry budget (also the
    /// bound on `harness::hook::post-turn` deny re-prompts).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_validation_retries: Option<u32>,
}

/// Session create/ensure options applied when this send creates the session.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SessionInit {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SendRequest {
    /// Omit to create a new session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The incoming message; a string is sugar for a user text message. The
    /// role must be `user` or `custom`.
    pub message: MessageInput,
    /// Required to start a NEW session unless `options.agent` supplies a
    /// model. Steering or waking an EXISTING session may omit it — the
    /// session's last turn's model (and provider, unless overridden) is
    /// inherited, the same rule the notification inject path uses. A model
    /// declared by the selected agent profile is authoritative.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    /// Webhook dedupe: a repeated key returns the original `{session_id,
    /// turn_id}` and appends nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Applied when this send creates/ensures the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<SessionInit>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<SendOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SendResponse {
    pub session_id: String,
    pub turn_id: String,
    pub accepted: bool,
    /// True when folded into an in-flight turn (steering).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub merged: Option<bool>,
    /// True when the message was queued while a step was streaming; it lands
    /// in the transcript when the stream ends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queued: Option<bool>,
    /// True when `idempotency_key` matched an earlier send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deduplicated: Option<bool>,
}

/// The shared result of starting (or merging) a turn.
pub struct StartOutcome {
    pub session_id: String,
    pub turn_id: String,
    pub merged: bool,
    pub queued: bool,
    pub deduplicated: bool,
}

pub async fn handle(deps: &Deps, req: SendRequest) -> Result<SendResponse, HarnessError> {
    handle_with_delivery_lock(deps, req, false).await
}

/// Trusted in-process entry from the invocation chokepoint. Lock ownership is
/// derived from its caller and never deserialized from the send request.
pub(crate) async fn handle_from_invoke(
    deps: &Deps,
    req: SendRequest,
    caller_session_id: &str,
    caller_holds_session_lock: bool,
) -> Result<SendResponse, HarnessError> {
    let caller_holds_target_session_lock = caller_holds_target_session_lock(
        caller_session_id,
        req.session_id.as_deref(),
        caller_holds_session_lock,
    );
    handle_with_delivery_lock(deps, req, caller_holds_target_session_lock).await
}

async fn handle_with_delivery_lock(
    deps: &Deps,
    req: SendRequest,
    caller_holds_target_session_lock: bool,
) -> Result<SendResponse, HarnessError> {
    let out = start_with_delivery_lock(deps, req, caller_holds_target_session_lock).await?;
    Ok(SendResponse {
        session_id: out.session_id,
        turn_id: out.turn_id,
        accepted: true,
        merged: out.merged.then_some(true),
        queued: out.queued.then_some(true),
        deduplicated: out.deduplicated.then_some(true),
    })
}

/// Ensure the session, persist the message, and seed/merge the turn.
pub async fn start(deps: &Deps, req: SendRequest) -> Result<StartOutcome, HarnessError> {
    start_with_delivery_lock(deps, req, false).await
}

async fn start_with_delivery_lock(
    deps: &Deps,
    req: SendRequest,
    caller_holds_target_session_lock: bool,
) -> Result<StartOutcome, HarnessError> {
    let cfg = deps.cfg().await;
    let session = deps.session().await;
    let idempotent = match &req.idempotency_key {
        Some(key) => crate::state::get_idem(&deps.iii, key, cfg.session_timeout_ms).await?,
        None => None,
    };

    // Steering an EXISTING session may omit `model`: inherit the last turn's
    // model (and provider, unless overridden) instead of failing on a raw
    // missing-field error — a user nudging a live run should not have to
    // re-name the model every time. A NEW session has nothing to inherit.
    let prev = match req.session_id.as_deref() {
        Some(sid) => crate::state::get_turn(&deps.iii, sid, cfg.session_timeout_ms).await?,
        None => None,
    };
    if let Some(outcome) = resolve_send_gate(
        idempotent,
        prev.as_ref()
            .is_some_and(|record| !record.status.is_terminal()),
        req.options
            .as_ref()
            .is_some_and(|options| options.skills.is_some()),
    )? {
        return Ok(outcome);
    }
    // Resolve the agent profile (if named) BEFORE the model gate and session
    // creation: the profile's model is a fallback for a session-creating send,
    // and a failed resolve must leave no session or budget ledger behind.
    let agent = resolve_send_agent(deps, &cfg, &req, prev.as_ref()).await?;

    // A profile-associated model is authoritative for that profile. Console
    // locks its picker to the same value, while this server-side precedence
    // keeps non-Console callers from accidentally running the identity on a
    // different model. Catalog keys may carry `provider::model`; split them
    // before routing.
    let agent_route = agent
        .as_ref()
        .and_then(|profile| profile.model_and_provider());
    let (model, provider) = match (agent_route, req.model.clone(), &prev) {
        (Some((model, profile_provider)), _, _) => {
            (model, profile_provider.or_else(|| req.provider.clone()))
        }
        (None, Some(model), _) => (model, req.provider.clone()),
        (None, None, Some(prev)) => (
            prev.options.model.clone(),
            req.provider
                .clone()
                .or_else(|| prev.options.provider.clone()),
        ),
        (None, None, None) if req.session_id.is_some() => {
            return Err(HarnessError::InvalidRequest(
                "harness::send without `model` inherits from the session's prior \
                 turn, but this session has none — name a `model`"
                    .into(),
            ))
        }
        (None, None, None) => {
            return Err(HarnessError::InvalidRequest(
                "harness::send creating a NEW session requires `model` (steering an \
                 existing session may omit it)"
                    .into(),
            ))
        }
    };

    // Freeze the per-send options before moving the message out of `req`.
    let inherits_prompt = prev.is_some() && prompt_fields_omitted(req.options.as_ref());
    // A profile IS the identity, so the stored/embedded default is only
    // fetched (a directory round trip) when no profile is set.
    let identity = if agent.is_some() {
        String::new()
    } else {
        crate::prompt::effective_default(&deps.iii).await.identity
    };
    let mut options = build_options(&cfg, &req, model, provider, agent.as_ref(), &identity);
    inherit_prior_functions(
        &cfg,
        &mut options,
        prev.as_ref()
            .and_then(|p| p.options.functions.as_ref())
            // An agent send with no explicit policy gets the configured
            // default instead of deny-all: an identity picked to DO something
            // must be able to dispatch. `prev` and `agent` are mutually
            // exclusive (resolve_send_agent), and the ask-mode clamp inside
            // still caps the result.
            .or_else(|| agent.as_ref().and(cfg.default_functions.as_ref())),
    );
    if let (true, Some(prev)) = (inherits_prompt, prev.as_ref()) {
        inherit_prior_system_prompt(&mut options, &prev.options);
    }
    prepare_skill_context(
        deps,
        &mut options,
        prev.as_ref().map(|record| &record.options),
        req.options
            .as_ref()
            .and_then(|options| options.skills.as_deref())
            .or_else(|| agent.as_ref().and_then(|a| a.skills.as_deref())),
    )
    .await?;

    // Normalise the incoming message and validate its role.
    let message = normalize_message(req.message)?;
    tag_send_span_with_message(&message);

    // Resolve the session (ensure if id given, else create).
    let (title, metadata) = req
        .session
        .as_ref()
        .map(|s| (s.title.clone(), s.metadata.clone()))
        .unwrap_or((None, None));
    let metadata = session_metadata_with_agent(metadata, agent.as_ref());
    let session_id = match &req.session_id {
        Some(id) => {
            let ensured = session
                .ensure(id, title.as_deref(), metadata.as_ref())
                .await?;
            // Console materialises its draft before calling harness::send, so
            // ensure cannot apply the authoritative Directory snapshot on
            // creation. Merge it into the stored whole-object metadata once.
            if let Some(agent) = agent.as_ref() {
                let snapshot = agent.session_metadata();
                if ensured.metadata.get("agent_profile") != Some(&snapshot) {
                    let mut stored = ensured.metadata;
                    stored.insert("agent_profile".into(), snapshot);
                    session.set_metadata(id, stored).await?;
                }
            }
            id.clone()
        }
        None => session.create(title.as_deref(), metadata.as_ref()).await?,
    };
    tag_failed_send_with_session(
        &session_id,
        crate::budget::prepare_root(deps, &session_id, &mut options, prev.as_ref()).await,
    )?;

    // Entry id: idempotent when a dedupe key is set.
    let entry_id = req
        .idempotency_key
        .as_ref()
        .map(|k| ids::idem_user_entry_id(k));

    // Queue path: a `Running` step may be streaming — park the message as a
    // durable queue row the loop drains after the stream ends, instead of
    // appending mid-transcript.
    let (outcome, entry) = tag_failed_send_with_session(
        &session_id,
        deliver(
            deps,
            &cfg,
            &session_id,
            options,
            Delivery {
                message: &message,
                entry_id: entry_id.as_deref(),
                origin: None,
                lineage: &TurnLineage::default(),
                caller_holds_session_lock: caller_holds_target_session_lock,
                skills_explicit: req
                    .options
                    .as_ref()
                    .is_some_and(|options| options.skills.is_some()),
            },
        )
        .await,
    )?;

    // Record the idempotency mapping (TTL-bound by contract).
    if let Some(key) = &req.idempotency_key {
        let rec = IdemRecord {
            session_id: session_id.clone(),
            turn_id: outcome.turn_id.clone(),
            entry_id: entry,
            ts: AgentMessage::now_ms(),
        };
        let _ = crate::state::put_idem(&deps.iii, key, &rec, cfg.session_timeout_ms).await;
    }

    Ok(outcome)
}

/// Label the send's own root span with the message preview. The turn step
/// stamps the same `iii.tag.message` through baggage, but only once the queue
/// delivers it — well after `execute harness::send` has closed and been
/// listed. The Console's message-labelled trace rows read the tag from the
/// trace's merged tags, so without this the row first appeared as
/// `execute harness::send` and relabelled itself a second later (MOT-4621).
/// Only the label rides here: session/message identity stays on the step
/// (see `tag_failed_send_with_session`).
fn tag_send_span_with_message(message: &AgentMessage) {
    if let Some(preview) = message_preview(message) {
        iii_helpers::observability::set_current_span_attribute("iii.tag.message", preview);
    }
}

/// Attribute failures that happen after session creation but before the turn
/// trace exists. Successful sends deliberately stay untagged here: their
/// `harness::turn step` trace is the one the session-scoped Console should
/// list, without a duplicate `harness::send` row.
fn tag_failed_send_with_session<T>(
    session_id: &str,
    result: Result<T, HarnessError>,
) -> Result<T, HarnessError> {
    if result.is_err() {
        iii_helpers::observability::set_current_span_attribute(
            "iii.session.id",
            session_id.to_string(),
        );
    }
    result
}

fn caller_holds_target_session_lock(
    caller_session_id: &str,
    target_session_id: Option<&str>,
    caller_holds_session_lock: bool,
) -> bool {
    caller_holds_session_lock && target_session_id == Some(caller_session_id)
}

/// Inject a message into an EXISTING session and wake/steer a turn, reusing the
/// session's last turn options (model / provider / dispatch policy / prompt) so
/// a woken turn keeps the agent's capabilities. Used by ephemeral subscriptions
/// to deliver a notification without polling — agents are never parked; events
/// arrive as messages.
///
/// Unlike [`start`], this never creates a session. It errors if the session has
/// no prior turn to inherit options from — in practice a session always has a
/// turn before it subscribes. A deterministic `entry_id` makes a redelivered
/// identical fire idempotent.
pub async fn inject(
    deps: &Deps,
    session_id: &str,
    message: AgentMessage,
    entry_id: Option<&str>,
    origin: Option<&Value>,
) -> Result<StartOutcome, HarnessError> {
    let cfg = deps.cfg().await;

    let options = crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms)
        .await?
        .map(|rec| rec.options)
        .ok_or_else(|| {
            HarnessError::InvalidRequest(format!(
                "cannot deliver notification to session `{session_id}`: it has no prior turn to \
                 inherit model/options from"
            ))
        })?;

    deliver(
        deps,
        &cfg,
        session_id,
        options,
        Delivery {
            message: &message,
            entry_id,
            origin,
            lineage: &TurnLineage::default(),
            caller_holds_session_lock: false,
            skills_explicit: false,
        },
    )
    .await
    .map(|(outcome, _)| outcome)
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct UnqueueRequest {
    pub session_id: String,
    /// The queued row's transcript entry id, as surfaced by `harness::status`
    /// → `queued[].entry_id`. Stable and client-visible (the internal row id
    /// is not), so removals target it.
    pub entry_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UnqueueResponse {
    /// False when no still-parked row matched — already drained or unknown.
    pub removed: bool,
}

/// Remove a still-parked message from a session's mid-turn queue so a client
/// can pull it back (the console's "press ↑ to edit"). Best-effort: a row that
/// already drained into the turn is simply `removed: false`. The queue is
/// keyed by an internal row id, so match on the client-visible `entry_id` and
/// delete by the row's own id.
pub async fn unqueue(deps: &Deps, req: UnqueueRequest) -> Result<UnqueueResponse, HarnessError> {
    let cfg = deps.cfg().await;
    let rows =
        crate::state::list_queued(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?;
    let Some(row) = rows.into_iter().find(|r| r.entry_id == req.entry_id) else {
        return Ok(UnqueueResponse { removed: false });
    };
    crate::state::delete_queued(&deps.iii, &req.session_id, &row.id, cfg.session_timeout_ms)
        .await?;
    Ok(UnqueueResponse { removed: true })
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct EditQueuedRequest {
    pub session_id: String,
    /// The queued row to edit (its client-visible `entry_id`).
    pub entry_id: String,
    /// The replacement message (string sugar or a full user/custom message).
    pub message: MessageInput,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct EditQueuedResponse {
    /// False when no still-parked row matched — already drained or unknown.
    pub updated: bool,
}

/// Edit a still-parked queued message IN PLACE — the console's "edit queued
/// message" preserving its delivery position. Replaces the row's `message`
/// while keeping the same internal id, `entry_id`, `queued_at`, and `origin`,
/// so re-writing the same state key leaves the message where it was in the
/// queue order (unlike remove + re-queue, which moves it to the tail). Emits
/// `harness::message-queued` so other clients refetch the new content.
pub async fn edit_queued(
    deps: &Deps,
    req: EditQueuedRequest,
) -> Result<EditQueuedResponse, HarnessError> {
    let cfg = deps.cfg().await;
    let rows =
        crate::state::list_queued(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?;
    let Some(mut row) = rows.into_iter().find(|r| r.entry_id == req.entry_id) else {
        return Ok(EditQueuedResponse { updated: false });
    };
    row.message = normalize_message(req.message)?;
    // Same `id` → same state key → overwrite in place (position preserved).
    crate::state::enqueue_message(&deps.iii, &row, cfg.session_timeout_ms).await?;
    deps.events
        .emit_queued(&req.session_id, &row.entry_id, row.queued_at)
        .await;
    Ok(EditQueuedResponse { updated: true })
}

/// The queue path: while a turn step is `Running` a stream may be in flight,
/// so the message parks as a durable `harness_queue` row the loop drains after
/// the stream ends (harness.md § Concurrency & steering). Returns `None` when
/// no step is running — the caller appends to the transcript as before.
///
/// After the (lock-free, blind-key) enqueue the turn record is re-read: a
/// still-live turn drains the row at its next step. A turn that went terminal
/// in the window is rechecked under the session lock: seed if still terminal,
/// or merge if another delivery already seeded. A row landing after the loop's
/// last queue check is appended by the finalize drain — queued messages are
/// never silently dropped.
async fn try_enqueue(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    options: &TurnOptions,
    d: &Delivery<'_>,
) -> Result<Option<(StartOutcome, String)>, HarnessError> {
    let Some(existing) =
        crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms).await?
    else {
        return Ok(None);
    };
    if existing.status != TurnStatus::Running {
        return Ok(None);
    }
    validate_active_skill_request(true, d.skills_explicit)?;

    let id = ids::new_queued_id();
    let entry_id = d
        .entry_id
        .map(str::to_string)
        .unwrap_or_else(|| ids::queued_entry_id(&id));
    let row = crate::state::QueuedMessage {
        id,
        session_id: session_id.to_string(),
        message: d.message.clone(),
        entry_id: entry_id.clone(),
        origin: d.origin.cloned(),
        queued_at: AgentMessage::now_ms(),
    };
    crate::state::enqueue_message(&deps.iii, &row, cfg.session_timeout_ms).await?;
    // Fire-and-forget: lets clients (e.g. the console's queued strip) refresh
    // `harness::status` → `queued` without polling.
    deps.events
        .emit_queued(session_id, &entry_id, row.queued_at)
        .await;

    let recheck = crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms).await?;
    let outcome = match recheck {
        Some(r) if !r.status.is_terminal() => {
            if filesystem_root_refresh_needed(&r.options, options) {
                seed_or_merge_queued(deps, cfg, session_id, options, d).await?
            } else {
                StartOutcome {
                    session_id: session_id.to_string(),
                    turn_id: r.turn_id,
                    merged: true,
                    queued: true,
                    deduplicated: false,
                }
            }
        }
        _ => seed_or_merge_queued(deps, cfg, session_id, options, d).await?,
    };
    Ok(Some((outcome, entry_id)))
}

fn filesystem_root_refresh_needed(current: &TurnOptions, incoming: &TurnOptions) -> bool {
    incoming
        .filesystem_root()
        .is_some_and(|root| current.filesystem_root() != Some(root))
}

async fn seed_or_merge_queued(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    options: &TurnOptions,
    d: &Delivery<'_>,
) -> Result<StartOutcome, HarnessError> {
    let mut outcome =
        with_delivery_guard(&deps.locks, session_id, d.caller_holds_session_lock, || {
            seed_or_merge(
                deps,
                cfg,
                session_id,
                options.clone(),
                message_preview(d.message),
                d.lineage,
                d.skills_explicit,
            )
        })
        .await?;
    outcome.queued = true;
    Ok(outcome)
}

fn latest_seed_record<'a>(
    initial: &'a TurnRecord,
    recheck: Option<&'a TurnRecord>,
) -> &'a TurnRecord {
    recheck.unwrap_or(initial)
}

/// The lineage a seeded turn carries: empty for a top-level send or a
/// notification wake, populated for a spawned child. It exists so ONE seeding
/// path can serve every entry point — before this, the child path hand-rolled
/// its own `TurnRecord` and `put_turn`, which skipped the CAS/merge check and
/// clobbered a running turn whenever a spawn reused a live session id.
#[derive(Debug, Clone, Default)]
pub(crate) struct TurnLineage {
    pub depth: u32,
    pub parent: Option<ParentLink>,
    pub display_parent_session_id: Option<String>,
}

/// One message on its way into a session: what to append, how to mark it, and
/// the lineage the seeded turn inherits. Bundled so every seeding path takes
/// the same shape.
pub(crate) struct Delivery<'a> {
    pub message: &'a AgentMessage,
    pub entry_id: Option<&'a str>,
    pub origin: Option<&'a Value>,
    pub lineage: &'a TurnLineage,
    pub caller_holds_session_lock: bool,
    pub skills_explicit: bool,
}

async fn delivery_guard(
    locks: &crate::locks::SessionLocks,
    session_id: &str,
    caller_holds_session_lock: bool,
) -> Option<tokio::sync::OwnedMutexGuard<()>> {
    if caller_holds_session_lock {
        None
    } else {
        Some(locks.guard(session_id).await)
    }
}

async fn with_delivery_guard<T, F, Fut>(
    locks: &crate::locks::SessionLocks,
    session_id: &str,
    caller_holds_session_lock: bool,
    operation: F,
) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let _guard = delivery_guard(locks, session_id, caller_holds_session_lock).await;
    operation().await
}

/// The shared delivery tail of every turn-seeding path: park the message when a
/// step is streaming, else append it and CAS-seed the turn (or merge into the
/// running one). `start`, `inject` and the sub-agent spawn all end here, so
/// mid-stream parking, the merge double-check and the record shape are
/// identical for a user message, a notification wake and a child's opening
/// task. Returns the outcome and the transcript entry id the message landed on.
pub(crate) async fn deliver(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    options: TurnOptions,
    d: Delivery<'_>,
) -> Result<(StartOutcome, String), HarnessError> {
    if d.skills_explicit {
        let active = crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms)
            .await?
            .is_some_and(|record| !record.status.is_terminal());
        validate_active_skill_request(active, true)?;
    }
    if let Some((outcome, row_entry)) = try_enqueue(deps, cfg, session_id, &options, &d).await? {
        return Ok((outcome, row_entry));
    }
    let preview = message_preview(d.message);
    if d.skills_explicit {
        return with_delivery_guard(
            &deps.locks,
            session_id,
            d.caller_holds_session_lock,
            || async move {
                let mut options = options;
                let previous =
                    crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms).await?;
                let session = deps.session().await;
                let appended =
                    append_explicit_after_terminal_rebase(&mut options, previous.as_ref(), || {
                        session.append(session_id, d.message, d.entry_id, None, d.origin)
                    })
                    .await?;
                let outcome =
                    seed_or_merge(deps, cfg, session_id, options, preview, d.lineage, true).await?;
                Ok((outcome, appended))
            },
        )
        .await;
    }
    let appended = deps
        .session()
        .await
        .append(session_id, d.message, d.entry_id, None, d.origin)
        .await?;
    // Every whole-record seed/merge writer uses the turn loop's session lock.
    // An in-turn spawn targeting its own session already holds that
    // non-reentrant lock; every other target acquires it here.
    let outcome = with_delivery_guard(&deps.locks, session_id, d.caller_holds_session_lock, || {
        seed_or_merge(
            deps,
            cfg,
            session_id,
            options,
            preview,
            d.lineage,
            d.skills_explicit,
        )
    })
    .await?;
    Ok((outcome, appended))
}

pub(crate) fn normalize_message(input: MessageInput) -> Result<AgentMessage, HarnessError> {
    match input {
        MessageInput::Text(text) => Ok(AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![crate::types::content::ContentBlock::text(text)],
            timestamp: AgentMessage::now_ms(),
        })),
        MessageInput::Message(m) => match *m {
            m @ (AgentMessage::User(_) | AgentMessage::Custom(_)) => Ok(m),
            other => Err(HarnessError::InvalidMessageRole(format!(
                "message role must be `user` or `custom`, got `{}`",
                other.role_str()
            ))),
        },
    }
}

/// First 30 chars of the user message, collapsed to a single line — the
/// `iii.tag.message` trace tag that labels message-grouped traces in the
/// console. `None` for non-user messages and empty text.
pub(crate) fn message_preview(message: &AgentMessage) -> Option<String> {
    const PREVIEW_CHARS: usize = 30;
    let AgentMessage::User(user) = message else {
        return None;
    };
    let text = crate::types::content::ContentBlock::join_text(&user.content);
    let collapsed = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let preview: String = collapsed.chars().take(PREVIEW_CHARS).collect();
    (!preview.is_empty()).then_some(preview)
}

fn build_options(
    cfg: &WorkerConfig,
    req: &SendRequest,
    model: String,
    provider: Option<String>,
    agent: Option<&crate::agents::ResolvedAgent>,
    identity: &str,
) -> TurnOptions {
    let opts = req.options.clone().unwrap_or_default();
    let functions = clamp_for_mode(cfg, opts.mode, opts.functions);
    let mut thinking_level = opts.thinking_level;
    let mut provider_options = opts.provider_options;
    if let Some(agent) = agent {
        agent.apply_reasoning(
            provider.as_deref(),
            &mut thinking_level,
            &mut provider_options,
        );
    }
    TurnOptions {
        model,
        provider,
        // An agent profile supplies the prompt (validated exclusive with the
        // explicit prompt fields in resolve_send_agent) and IS the identity:
        // nothing built-in underneath, and the mode paragraph applies to it
        // exactly as it would to the built-in identity.
        system_prompt: match agent {
            Some(a) => Some(prompt::build_system_prompt(SystemPromptOpts {
                mode: opts.mode,
                identity: &a.prompt,
            })),
            None => prompt::resolve_system_prompt(
                opts.system_prompt,
                opts.system_prompt_strategy.unwrap_or_default(),
                opts.mode,
                identity,
            ),
        },
        skills_prompt: None,
        skill_context: None,
        mode: opts.mode,
        max_turns: opts.max_turns.unwrap_or(cfg.default_max_turns),
        max_output_tokens: opts.max_output_tokens,
        max_total_tokens: opts.max_total_tokens,
        max_cost_usd: opts.max_cost_usd,
        budget_root_session_id: None,
        thinking_level,
        provider_options,
        output: opts.output.unwrap_or_default(),
        functions,
        metadata: opts.metadata,
        agent: agent.map(|a| a.identity.clone()),
        max_validation_retries: opts
            .max_validation_retries
            .unwrap_or(cfg.max_validation_retries),
        max_transient_resumes: cfg.max_transient_resumes,
    }
}

/// Merge the frozen agent display/configuration snapshot into session
/// metadata without disturbing console-owned or tenancy keys.
pub(crate) fn session_metadata_with_agent(
    metadata: Option<Value>,
    agent: Option<&crate::agents::ResolvedAgent>,
) -> Option<Value> {
    let Some(agent) = agent else {
        return metadata;
    };
    let mut object = metadata
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    object.insert("agent_profile".into(), agent.session_metadata());
    Some(Value::Object(object))
}

/// The single chokepoint for the ask-mode policy cap:
/// every turn-seeding path (a fresh send, an inherited steer, a spawned child)
/// routes its resolved dispatch policy through here, so ask mode is capped at
/// the configured default policy no matter how the policy was assembled.
/// A non-ask turn passes through untouched.
pub(crate) fn clamp_for_mode(
    cfg: &WorkerConfig,
    mode: Option<Mode>,
    functions: Option<FunctionPolicy>,
) -> Option<FunctionPolicy> {
    match mode {
        Some(Mode::Ask) => policy::clamp_policy(cfg.default_functions.as_ref(), functions.as_ref()),
        _ => functions,
    }
}

/// A steer also inherits the prior turn's dispatch policy unless this send
/// names its own: `functions` is fail-closed, so leaving it `None` on a fresh
/// steer record would silently DISARM a live run — every turn from the nudge
/// onward denied all dispatch. Explicit strip stays possible
/// (`options.functions: { allow: [] }`). An ask-mode steer stays armed but
/// read-only via the shared [`clamp_for_mode`] cap.
fn inherit_prior_functions(
    cfg: &WorkerConfig,
    options: &mut TurnOptions,
    prev_functions: Option<&FunctionPolicy>,
) {
    if options.functions.is_some() {
        return;
    }
    options.functions = clamp_for_mode(cfg, options.mode, prev_functions.cloned());
}

fn select_skill_context(
    previous: Option<&TurnOptions>,
    requested: Option<&[String]>,
    view: &crate::skills::EffectiveView,
) -> Result<Option<SkillContext>, HarnessError> {
    match previous.and_then(|options| options.skill_context.as_ref()) {
        Some(previous) => Ok(Some(crate::skills::next_context(previous, requested))),
        None if previous.is_some() && requested.is_some() => Err(HarnessError::InvalidRequest(
            "cannot change `skills` on a legacy session; start a new session to use the names-only skill index"
                .into(),
        )),
        None if previous.is_some() => Ok(None),
        None => Ok(Some(crate::skills::new_context(requested, view))),
    }
}

pub(crate) fn validate_active_skill_request(
    active: bool,
    explicit: bool,
) -> Result<(), HarnessError> {
    if active && explicit {
        return Err(HarnessError::InvalidRequest(
            "cannot change `skills` while the session turn is active; retry after the turn finishes"
                .into(),
        ));
    }
    Ok(())
}

fn resolve_send_gate(
    idempotent: Option<IdemRecord>,
    active: bool,
    skills_explicit: bool,
) -> Result<Option<StartOutcome>, HarnessError> {
    if let Some(existing) = idempotent {
        return Ok(Some(StartOutcome {
            session_id: existing.session_id,
            turn_id: existing.turn_id,
            merged: false,
            queued: false,
            deduplicated: true,
        }));
    }
    validate_active_skill_request(active, skills_explicit)?;
    Ok(None)
}

fn merge_explicit_skill_filter(
    active: &mut TurnOptions,
    requested: &TurnOptions,
    explicit: bool,
) -> Result<bool, HarnessError> {
    if !explicit {
        return Ok(false);
    }
    let requested = requested.skill_context.as_ref().ok_or_else(|| {
        HarnessError::InvalidRequest(
            "cannot change `skills` on a legacy session; start a new session to use the names-only skill index"
                .into(),
        )
    })?;
    let active = active.skill_context.as_mut().ok_or_else(|| {
        HarnessError::InvalidRequest(
            "cannot change `skills` on a legacy session; start a new session to use the names-only skill index"
                .into(),
        )
    })?;
    if active.filter == requested.filter {
        return Ok(false);
    }
    active.filter = requested.filter.clone();
    Ok(true)
}

pub(crate) async fn prepare_skill_context(
    deps: &Deps,
    options: &mut TurnOptions,
    previous: Option<&TurnOptions>,
    requested: Option<&[String]>,
) -> Result<(), HarnessError> {
    let functions = deps.functions().await;
    let skills = deps.skills().await;
    let policy = policy::CompiledPolicy::from(options.functions.as_ref());
    let view = crate::skills::effective_view(&skills, requested, &policy, &functions.functions);
    options.skill_context = select_skill_context(previous, requested, &view)?;
    if options.skill_context.is_none() {
        options.skills_prompt = previous.and_then(|options| options.skills_prompt.clone());
    } else {
        options.skills_prompt = None;
    }
    Ok(())
}

/// True when the send names neither `system_prompt` nor
/// `system_prompt_strategy` — the condition under which an existing session
/// inherits the prior turn's prompt instead of resolving fresh.
fn prompt_fields_omitted(opts: Option<&SendOptions>) -> bool {
    opts.is_none_or(|o| o.system_prompt.is_none() && o.system_prompt_strategy.is_none())
}

/// A send that names no prompt fields inherits the prior turn's RESOLVED
/// system prompt, the same way model/provider/functions are inherited. A
/// prior `disabled` turn's `None` inherits too — disabled stays disabled.
/// Any explicit prompt field resolves fresh; a bare `system_prompt_strategy`
/// is the reset-to-default escape hatch. The frozen agent identity travels
/// with the prompt: inherited together, shed together (an explicit prompt
/// field also drops the identity).
fn inherit_prior_system_prompt(options: &mut TurnOptions, prev: &TurnOptions) {
    options.system_prompt = prev.system_prompt.clone();
    options.skills_prompt = prev.skills_prompt.clone();
    options.agent = prev.agent.clone();
}

/// Resolve `options.agent` for this send, or `None` when absent. Validation
/// happens before any session/budget side effects.
async fn resolve_send_agent(
    deps: &Deps,
    cfg: &WorkerConfig,
    req: &SendRequest,
    prev: Option<&TurnRecord>,
) -> Result<Option<crate::agents::ResolvedAgent>, HarnessError> {
    let Some(id) = agent_send_id(req.options.as_ref(), prev.is_some())? else {
        return Ok(None);
    };
    crate::agents::resolve(deps, cfg, id).await.map(Some)
}

/// The pre-fetch half of agent-send validation: which id (if any) this send
/// resolves, or why it may not name one.
fn agent_send_id(
    options: Option<&SendOptions>,
    has_prev: bool,
) -> Result<Option<&str>, HarnessError> {
    let Some(id) = options.and_then(|o| o.agent.as_deref()) else {
        return Ok(None);
    };
    if has_prev {
        return Err(HarnessError::InvalidRequest(
            "`options.agent` applies only when starting a session; later sends inherit the \
             frozen identity — start a new session to use a different agent profile"
                .into(),
        ));
    }
    if !prompt_fields_omitted(options) {
        return Err(HarnessError::InvalidRequest(
            "`options.agent` selects an agent profile that supplies the system prompt; drop `system_prompt` / \
             `system_prompt_strategy` or drop `agent`"
                .into(),
        ));
    }
    Ok(Some(id))
}

/// A send whose metadata does not name the `fs_scope` key inherits the prior
/// turn's working directory — the same omitted-field rule as model, prompt,
/// and policy. The working-dir line in the system prompt must not flip when a
/// client omits `fs_scope` on a later send (it invalidates the provider's
/// prompt-cache prefix). An explicit `fs_scope` — even an empty `{}` — is an
/// intentional clear and blocks inheritance.
///
/// Called from `seed_new` only, with the freshest prior record read under the
/// delivery guard: an inherited root must never travel into the active-turn
/// merge path, where `refresh_filesystem_root_from` would let a stale
/// inherited value overwrite a root the running turn just changed.
fn inherit_prior_filesystem_root(options: &mut TurnOptions, prev: &TurnOptions) {
    let names_fs_scope = options
        .metadata
        .as_ref()
        .and_then(Value::as_object)
        .is_some_and(|m| m.contains_key(crate::types::turn::FS_SCOPE_KEY));
    if names_fs_scope {
        return;
    }
    if let Some(root) = prev.filesystem_root() {
        options.set_filesystem_root(root);
    }
}

/// Default a BRAND-NEW session's working directory (MOT-3897): when the very
/// first turn arrives without `metadata.fs_scope.root`, scope it to the
/// configured default (the stack's launch folder). Existing sessions are never
/// retroactively scoped — their unscoped turns stay unscoped — and an explicit
/// root on the send always wins.
fn apply_default_filesystem_root(
    options: &mut TurnOptions,
    is_new_session: bool,
    default_root: Option<&str>,
) {
    if !is_new_session || options.filesystem_root().is_some() {
        return;
    }
    if let Some(root) = default_root {
        options.set_filesystem_root(root);
    }
}

/// The turn CAS + merge double-check (harness.md § Concurrency & steering).
async fn seed_or_merge(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    mut options: TurnOptions,
    message_preview: Option<String>,
    lineage: &TurnLineage,
    skills_explicit: bool,
) -> Result<StartOutcome, HarnessError> {
    let existing = crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms).await?;
    apply_default_filesystem_root(
        &mut options,
        existing.is_none(),
        cfg.resolved_default_filesystem_root().as_deref(),
    );
    match existing {
        Some(rec) if !rec.status.is_terminal() => {
            // Merge path: the message is already appended; the running loop's
            // steering check folds it in. Double-check the record didn't go
            // terminal in the append window.
            let recheck =
                crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms).await?;
            match recheck {
                Some(mut r) if !r.status.is_terminal() => {
                    let changed = r.options.refresh_filesystem_root_from(&options)
                        | merge_explicit_skill_filter(&mut r.options, &options, skills_explicit)?;
                    if changed {
                        r.updated_at = AgentMessage::now_ms();
                        crate::state::put_turn(&deps.iii, &r, cfg.session_timeout_ms).await?;
                    }
                    Ok(StartOutcome {
                        session_id: session_id.to_string(),
                        turn_id: r.turn_id,
                        merged: true,
                        queued: false,
                        deduplicated: false,
                    })
                }
                recheck => {
                    let previous = latest_seed_record(&rec, recheck.as_ref());
                    if !skills_explicit {
                        rebase_terminal_skill_options(&mut options, Some(previous), false)?;
                    }
                    seed_new(
                        deps,
                        cfg,
                        session_id,
                        options,
                        Some(previous),
                        message_preview,
                        lineage,
                    )
                    .await
                }
            }
        }
        previous => {
            if !skills_explicit {
                rebase_terminal_skill_options(&mut options, previous.as_ref(), false)?;
            }
            seed_new(
                deps,
                cfg,
                session_id,
                options,
                previous.as_ref(),
                message_preview,
                lineage,
            )
            .await
        }
    }
}

fn rebase_terminal_skill_options(
    options: &mut TurnOptions,
    previous: Option<&TurnRecord>,
    skills_explicit: bool,
) -> Result<(), HarnessError> {
    let Some(previous) = previous else {
        return Ok(());
    };
    if skills_explicit {
        let invalid_legacy_change = || {
            HarnessError::InvalidRequest(
                "cannot change `skills` on a legacy session; start a new session to use the names-only skill index"
                    .into(),
            )
        };
        let filter = options
            .skill_context
            .as_ref()
            .ok_or_else(&invalid_legacy_change)?
            .filter
            .clone();
        let mut context = previous
            .options
            .skill_context
            .clone()
            .ok_or_else(&invalid_legacy_change)?;
        context.filter = filter;
        options.skill_context = Some(context);
        options.skills_prompt = previous.options.skills_prompt.clone();
        return Ok(());
    }
    options.skill_context = previous.options.skill_context.clone();
    options.skills_prompt = previous.options.skills_prompt.clone();
    Ok(())
}

async fn append_explicit_after_terminal_rebase<F, Fut>(
    options: &mut TurnOptions,
    previous: Option<&TurnRecord>,
    append: F,
) -> Result<String, HarnessError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<String, HarnessError>>,
{
    validate_active_skill_request(
        previous.is_some_and(|record| !record.status.is_terminal()),
        true,
    )?;
    rebase_terminal_skill_options(options, previous, true)?;
    append().await
}

/// Seed a fresh turn record and enqueue its first step. Exposed to the turn
/// loop's finalize-drain reseed path (`turn_loop::reseed_after_finalize_drain`)
/// so a notification that parked during a turn's final step gets a turn to
/// react to it, instead of being drained to the transcript and stranded.
pub(crate) async fn seed_new(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    mut options: TurnOptions,
    prior: Option<&TurnRecord>,
    message_preview: Option<String>,
    lineage: &TurnLineage,
) -> Result<StartOutcome, HarnessError> {
    if let Some(prior) = prior {
        inherit_prior_filesystem_root(&mut options, &prior.options);
    }
    let turn_id = ids::new_turn_id();
    let now = AgentMessage::now_ms();
    let functions_generation = prior.and_then(|record| record.functions_generation);
    let function_contract_ledger = prior
        .map(|record| record.function_contract_ledger.clone())
        .unwrap_or_default();
    let skill_ack = prior.and_then(|record| record.skill_ack.clone());
    let skills_started = prior.is_some_and(|record| record.skills_started);
    let record = TurnRecord {
        turn_id: turn_id.clone(),
        session_id: session_id.to_string(),
        status: TurnStatus::Running,
        step: 0,
        turn_count: 0,
        depth: lineage.depth,
        message_preview,
        abort: false,
        watermark_entry_id: None,
        stream_request_id: None,
        options,
        calls: Default::default(),
        parent: lineage.parent.clone(),
        display_parent_session_id: lineage.display_parent_session_id.clone(),
        functions_generation,
        function_contract_ledger,
        skill_ack,
        skills_started,
        context_snapshot: None,
        result: None,
        result_error: None,
        validation_retries: 0,
        transient_resumes: 0,
        created_at: now,
        updated_at: now,
    };
    crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
    turn_loop::enqueue_step(
        &deps.iii,
        session_id,
        &turn_id,
        0,
        record.message_preview.as_deref(),
        lineage.depth,
    )
    .await?;
    Ok(StartOutcome {
        session_id: session_id.to_string(),
        turn_id,
        merged: false,
        queued: false,
        deduplicated: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use iii_helpers::observability::opentelemetry::trace::{
        TraceContextExt, Tracer, TracerProvider,
    };
    use iii_helpers::observability::opentelemetry::Context;
    use opentelemetry_sdk::trace::{InMemorySpanExporter, SdkTracerProvider, SimpleSpanProcessor};
    use std::sync::Arc;

    fn failed_send_session_attribute(result: Result<(), HarnessError>) -> Option<String> {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_span_processor(SimpleSpanProcessor::new(exporter.clone()))
            .build();
        let tracer = provider.tracer("harness-send-test");
        let span = tracer.start("execute harness::send");
        let guard = Context::new().with_span(span).attach();

        let _ = tag_failed_send_with_session("session-filter-test", result);

        drop(guard);
        exporter
            .get_finished_spans()
            .expect("exporter")
            .into_iter()
            .next()
            .and_then(|span| {
                span.attributes
                    .into_iter()
                    .find(|attribute| attribute.key.as_str() == "iii.session.id")
                    .map(|attribute| attribute.value.as_str().to_string())
            })
    }

    fn send_span_message_tag(message: &AgentMessage) -> Option<String> {
        let exporter = InMemorySpanExporter::default();
        let provider = SdkTracerProvider::builder()
            .with_span_processor(SimpleSpanProcessor::new(exporter.clone()))
            .build();
        let tracer = provider.tracer("harness-send-test");
        let span = tracer.start("execute harness::send");
        let guard = Context::new().with_span(span).attach();

        tag_send_span_with_message(message);

        drop(guard);
        exporter
            .get_finished_spans()
            .expect("exporter")
            .into_iter()
            .next()
            .and_then(|span| {
                span.attributes
                    .into_iter()
                    .find(|attribute| attribute.key.as_str() == "iii.tag.message")
                    .map(|attribute| attribute.value.as_str().to_string())
            })
    }

    #[test]
    fn send_root_span_carries_the_message_preview_label() {
        let user = normalize_message(MessageInput::Text(
            "help me implement the traces v2 new tags please".into(),
        ))
        .unwrap();
        assert_eq!(
            send_span_message_tag(&user).as_deref(),
            Some("help me implement the traces v"),
        );

        // Nothing to label: a blank message leaves the span untouched.
        let blank = normalize_message(MessageInput::Text("   ".into())).unwrap();
        assert_eq!(send_span_message_tag(&blank), None);
    }

    #[test]
    fn failed_send_is_attributed_to_its_session_without_tagging_successes() {
        assert_eq!(failed_send_session_attribute(Ok(())), None);
        assert_eq!(
            failed_send_session_attribute(Err(HarnessError::Dependency(
                "enqueue harness::turn failed".into(),
            ))),
            Some("session-filter-test".into()),
        );
    }

    #[tokio::test]
    async fn omitted_delivery_cannot_overwrite_an_explicit_filter_writer() {
        let locks = crate::locks::SessionLocks::new();
        let explicit_guard = delivery_guard(&locks, "s", false)
            .await
            .expect("external delivery acquires the session lock");
        let mut durable = (false, Some(vec!["old".to_string()]));
        let stale_omitted_filter = durable.1.clone();
        let mut omitted_guard = Box::pin(delivery_guard(&locks, "s", false));

        let blocked = tokio::select! {
            biased;
            _ = &mut omitted_guard => false,
            _ = tokio::task::yield_now() => true,
        };
        assert!(blocked, "omitted writer must wait for the explicit writer");

        durable = (true, Some(vec!["requested".to_string()]));
        drop(explicit_guard);
        let _omitted_guard = omitted_guard
            .await
            .expect("omitted delivery uses the same session lock");
        if !durable.0 {
            durable = (true, stale_omitted_filter);
        }

        assert_eq!(durable.1, Some(vec!["requested".to_string()]));
    }

    #[tokio::test]
    async fn delivery_skips_reacquiring_a_session_lock_its_caller_owns() {
        let locks = crate::locks::SessionLocks::new();
        let _held = locks.guard("s").await;

        let guard = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            delivery_guard(&locks, "s", true),
        )
        .await
        .expect("owned-lock delivery must not deadlock");

        assert!(guard.is_none());
    }

    #[tokio::test]
    async fn queued_terminal_recheck_waits_for_the_explicit_filter_writer() {
        let locks = crate::locks::SessionLocks::new();
        let explicit_guard = delivery_guard(&locks, "s", false)
            .await
            .expect("explicit writer acquires the session lock");
        let durable_filter = Arc::new(tokio::sync::Mutex::new(Some(vec!["old".to_string()])));
        let queued_filter = durable_filter.clone();
        let mut queued_recheck = Box::pin(with_delivery_guard(
            &locks,
            "s",
            false,
            move || async move { queued_filter.lock().await.clone() },
        ));

        let blocked = tokio::select! {
            biased;
            _ = &mut queued_recheck => false,
            _ = tokio::task::yield_now() => true,
        };
        assert!(blocked, "queued terminal recheck must wait");

        *durable_filter.lock().await = Some(vec!["requested".to_string()]);
        drop(explicit_guard);

        assert_eq!(queued_recheck.await, Some(vec!["requested".to_string()]));
    }

    #[tokio::test]
    async fn live_queue_root_refresh_cannot_restore_a_stale_skill_filter() {
        let mut stale_recheck = options_with(None, None);
        stale_recheck.skill_context = Some(crate::types::turn::SkillContext {
            filter: Some(vec!["old".into()]),
            baseline: Some("frozen".into()),
        });
        stale_recheck.set_filesystem_root("/old");
        let mut incoming = stale_recheck.clone();
        incoming.set_filesystem_root("/new");
        assert!(filesystem_root_refresh_needed(&stale_recheck, &incoming));

        let locks = crate::locks::SessionLocks::new();
        let explicit_guard = delivery_guard(&locks, "s", false)
            .await
            .expect("explicit writer acquires the session lock");
        let durable = Arc::new(tokio::sync::Mutex::new(stale_recheck));
        let queued_durable = durable.clone();
        let mut queued_refresh = Box::pin(with_delivery_guard(&locks, "s", false, move || {
            let incoming = incoming.clone();
            async move {
                let mut current = queued_durable.lock().await;
                current.refresh_filesystem_root_from(&incoming);
                current.clone()
            }
        }));

        let blocked = tokio::select! {
            biased;
            _ = &mut queued_refresh => false,
            _ = tokio::task::yield_now() => true,
        };
        assert!(
            blocked,
            "live queue refresh must wait for the explicit writer"
        );
        durable.lock().await.skill_context.as_mut().unwrap().filter =
            Some(vec!["requested".into()]);
        drop(explicit_guard);

        let refreshed = queued_refresh.await;
        assert_eq!(refreshed.filesystem_root(), Some("/new"));
        assert_eq!(
            refreshed.skill_context.unwrap().filter,
            Some(vec!["requested".into()])
        );
    }

    #[tokio::test]
    async fn trusted_same_session_send_inherits_the_callers_lock() {
        let locks = crate::locks::SessionLocks::new();
        let _held = locks.guard("s_caller").await;
        let owns_target = caller_holds_target_session_lock("s_caller", Some("s_caller"), true);
        assert!(owns_target);
        assert!(tokio::time::timeout(
            std::time::Duration::from_millis(50),
            delivery_guard(&locks, "s_caller", owns_target),
        )
        .await
        .expect("same-session delivery must not deadlock")
        .is_none());
    }

    #[tokio::test]
    async fn trusted_other_session_send_acquires_the_target_lock() {
        let locks = crate::locks::SessionLocks::new();
        let _held = locks.guard("s_caller").await;
        let owns_target = caller_holds_target_session_lock("s_caller", Some("s_other"), true);
        assert!(!owns_target);
        assert!(delivery_guard(&locks, "s_other", owns_target)
            .await
            .is_some());
        assert!(!caller_holds_target_session_lock(
            "s_caller",
            Some("s_caller"),
            false
        ));
    }

    #[test]
    fn string_message_becomes_user_text() {
        let m = normalize_message(MessageInput::Text("hi".into())).unwrap();
        assert!(matches!(m, AgentMessage::User(_)));
    }

    #[test]
    fn assistant_message_is_rejected() {
        let assistant = AgentMessage::Assistant(crate::types::message::empty_assistant("p", "m"));
        let err = normalize_message(MessageInput::Message(Box::new(assistant))).unwrap_err();
        assert_eq!(err.code(), "harness/invalid_message_role");
    }

    #[test]
    fn message_preview_takes_first_30_chars_single_line() {
        let m = normalize_message(MessageInput::Text(
            "help me implement the traces v2 new tags please".into(),
        ))
        .unwrap();
        assert_eq!(
            message_preview(&m).as_deref(),
            Some("help me implement the traces v"),
        );

        let multiline =
            normalize_message(MessageInput::Text("fix\nthe   login\n\nbug".into())).unwrap();
        assert_eq!(
            message_preview(&multiline).as_deref(),
            Some("fix the login bug")
        );

        // Char-boundary safe on multi-byte text.
        let emoji = normalize_message(MessageInput::Text("🦀".repeat(40))).unwrap();
        assert_eq!(message_preview(&emoji).unwrap().chars().count(), 30);

        let empty = normalize_message(MessageInput::Text("   ".into())).unwrap();
        assert_eq!(message_preview(&empty), None);
    }

    #[test]
    fn unqueue_matches_the_client_visible_entry_id_not_the_row_id() {
        // The subtlety `unqueue` encodes: the queue is keyed by an internal
        // `q_*` row id, but clients only see `entry_id` (via harness::status).
        // Removal must match on entry_id and resolve to the row id to delete.
        fn row(id: &str, entry: &str) -> crate::state::QueuedMessage {
            crate::state::QueuedMessage {
                id: id.into(),
                session_id: "s_1".into(),
                message: AgentMessage::user_text("hi"),
                entry_id: entry.into(),
                origin: None,
                queued_at: 0,
            }
        }
        let rows = [row("q_aaa", "e_idem_msg-1"), row("q_bbb", "e_idem_msg-2")];
        let hit = rows.iter().find(|r| r.entry_id == "e_idem_msg-2");
        assert_eq!(hit.map(|r| r.id.as_str()), Some("q_bbb"));
        assert!(rows.iter().all(|r| r.entry_id != "e_idem_msg-3"));
    }

    #[test]
    fn edit_queued_preserves_position_fields_and_only_swaps_message() {
        // Editing in place keeps id / entry_id / queued_at / origin (position
        // is `(queued_at, id)`) and replaces only the message — so re-writing
        // the same `session_id:id` state key leaves it where it was in order.
        let mut row = crate::state::QueuedMessage {
            id: "q_x".into(),
            session_id: "s_1".into(),
            message: AgentMessage::user_text("old text"),
            entry_id: "e_idem_msg-1".into(),
            origin: Some(serde_json::json!({ "reaction": true })),
            queued_at: 4242,
        };
        let before = (
            row.id.clone(),
            row.entry_id.clone(),
            row.queued_at,
            row.origin.clone(),
        );
        row.message = normalize_message(MessageInput::Text("new text".into())).unwrap();
        assert_eq!(
            (row.id, row.entry_id, row.queued_at, row.origin),
            before,
            "only the message changes"
        );
    }

    #[test]
    fn build_options_applies_embedded_prompt_when_system_prompt_omitted() {
        let cfg = WorkerConfig::default();
        let req = SendRequest {
            session_id: None,
            message: MessageInput::Text("hi".into()),
            model: Some("claude-sonnet-4".into()),
            provider: Some("anthropic".into()),
            idempotency_key: None,
            session: None,
            options: Some(SendOptions {
                mode: Some(Mode::Agent),
                ..Default::default()
            }),
        };
        let opts = build_options(
            &cfg,
            &req,
            "claude-sonnet-4".into(),
            req.provider.clone(),
            None,
            crate::prompt::DEFAULT,
        );
        let prompt = opts.system_prompt.expect("built-in prompt");
        assert!(prompt.contains("operating in agent mode"));
        assert!(prompt.contains("# System rules"));
    }

    #[test]
    fn build_options_honors_non_empty_system_prompt_override() {
        let cfg = WorkerConfig::default();
        let req = SendRequest {
            session_id: None,
            message: MessageInput::Text("hi".into()),
            model: Some("m".into()),
            provider: Some("anthropic".into()),
            idempotency_key: None,
            session: None,
            options: Some(SendOptions {
                system_prompt: Some("custom".into()),
                system_prompt_strategy: Some(SystemPromptStrategy::Override),
                mode: Some(Mode::Ask),
                ..Default::default()
            }),
        };
        let opts = build_options(
            &cfg,
            &req,
            "claude-sonnet-4".into(),
            req.provider.clone(),
            None,
            crate::prompt::DEFAULT,
        );
        assert_eq!(opts.system_prompt.as_deref(), Some("custom"));
    }

    fn options_with(mode: Option<Mode>, functions: Option<FunctionPolicy>) -> TurnOptions {
        TurnOptions {
            model: "m".into(),
            provider: None,
            system_prompt: None,
            skills_prompt: None,
            skill_context: None,
            mode,
            max_turns: 16,
            max_output_tokens: None,
            max_total_tokens: None,
            max_cost_usd: None,
            budget_root_session_id: None,
            thinking_level: None,
            provider_options: None,
            output: OutputContract::Text,
            functions,
            metadata: None,
            agent: None,
            max_validation_retries: 2,
            max_transient_resumes: 1,
        }
    }

    fn terminal_record_with_skill_state(generation: u64, started: bool) -> TurnRecord {
        TurnRecord {
            turn_id: "t_1".into(),
            session_id: "s_1".into(),
            status: TurnStatus::Completed,
            step: 1,
            turn_count: 1,
            depth: 0,
            message_preview: None,
            abort: false,
            watermark_entry_id: None,
            stream_request_id: None,
            options: options_with(None, None),
            calls: Default::default(),
            parent: None,
            display_parent_session_id: None,
            functions_generation: Some(generation),
            function_contract_ledger: Default::default(),
            skill_ack: Some(crate::types::turn::SkillAck {
                generation,
                fingerprint: Some(format!("sha256:{generation}")),
            }),
            skills_started: started,
            context_snapshot: None,
            result: None,
            result_error: None,
            validation_retries: 0,
            transient_resumes: 0,
            created_at: 1,
            updated_at: 1,
        }
    }

    #[test]
    fn terminal_recheck_is_the_seed_source_after_an_active_turn_finishes() {
        let stale = terminal_record_with_skill_state(1, false);
        let final_record = terminal_record_with_skill_state(2, true);

        let selected = latest_seed_record(&stale, Some(&final_record));

        assert_eq!(selected.functions_generation, Some(2));
        assert_eq!(selected.skill_ack.as_ref().unwrap().generation, 2);
        assert!(selected.skills_started);
        assert!(std::ptr::eq(selected, &final_record));
    }

    #[test]
    fn omitted_skills_rebase_from_terminal_recheck_before_seeding() {
        let mut initial = terminal_record_with_skill_state(1, false);
        initial.status = TurnStatus::Running;
        initial.options.skill_context = Some(crate::types::turn::SkillContext {
            filter: Some(vec!["old".into()]),
            baseline: Some("old baseline".into()),
        });
        let mut terminal = terminal_record_with_skill_state(2, true);
        terminal.options.skill_context = Some(crate::types::turn::SkillContext {
            filter: Some(vec!["new".into()]),
            baseline: Some("new baseline".into()),
        });
        let mut prepared = initial.options.clone();

        let authoritative = latest_seed_record(&initial, Some(&terminal));
        rebase_terminal_skill_options(&mut prepared, Some(authoritative), false).unwrap();

        assert_eq!(prepared.skill_context, terminal.options.skill_context);
        assert_eq!(prepared.skills_prompt, terminal.options.skills_prompt);
    }

    #[test]
    fn omitted_skills_rebase_from_initial_terminal_record_before_seeding() {
        let mut terminal = terminal_record_with_skill_state(2, true);
        terminal.options.skill_context = None;
        terminal.options.skills_prompt = Some("authoritative legacy body".into());
        let mut prepared = options_with(None, None);
        prepared.skill_context = Some(crate::types::turn::SkillContext {
            filter: Some(vec!["stale".into()]),
            baseline: Some("stale baseline".into()),
        });
        prepared.skills_prompt = Some("stale legacy body".into());

        rebase_terminal_skill_options(&mut prepared, Some(&terminal), false).unwrap();

        assert_eq!(prepared.skill_context, None);
        assert_eq!(
            prepared.skills_prompt.as_deref(),
            Some("authoritative legacy body")
        );
    }

    #[test]
    fn explicit_reset_rebases_the_authoritative_baseline_without_losing_its_filter() {
        let mut initial = terminal_record_with_skill_state(1, false);
        initial.status = TurnStatus::Running;
        initial.options.skill_context = Some(crate::types::turn::SkillContext {
            filter: Some(vec!["prior".into()]),
            baseline: None,
        });
        let mut terminal = terminal_record_with_skill_state(2, true);
        terminal.options.skill_context = Some(crate::types::turn::SkillContext {
            filter: Some(vec!["prior".into()]),
            baseline: Some("baseline frozen by the completed turn".into()),
        });
        let mut prepared = initial.options.clone();
        prepared.skill_context.as_mut().unwrap().filter = None;

        let authoritative = latest_seed_record(&initial, Some(&terminal));
        rebase_terminal_skill_options(&mut prepared, Some(authoritative), true).unwrap();

        assert_eq!(
            prepared.skill_context,
            Some(crate::types::turn::SkillContext {
                filter: None,
                baseline: Some("baseline frozen by the completed turn".into()),
            })
        );
        assert_eq!(prepared.skills_prompt, terminal.options.skills_prompt);
        assert_eq!(authoritative.skill_ack, terminal.skill_ack);
        assert!(authoritative.skills_started);
    }

    #[test]
    fn fresh_skill_options_keep_the_prepared_context() {
        let mut prepared = options_with(None, None);
        prepared.skill_context = Some(crate::types::turn::SkillContext {
            filter: Some(vec!["fresh".into()]),
            baseline: Some("fresh baseline".into()),
        });
        let expected = prepared.clone();

        rebase_terminal_skill_options(&mut prepared, None, true).unwrap();
        assert_eq!(prepared.skill_context, expected.skill_context);
        assert_eq!(prepared.skills_prompt, expected.skills_prompt);
    }

    #[test]
    fn terminal_rebase_rejects_an_explicit_change_to_authoritative_legacy_context() {
        let mut terminal = terminal_record_with_skill_state(2, true);
        terminal.options.skill_context = None;
        terminal.options.skills_prompt = Some("legacy body".into());
        let mut prepared = options_with(None, None);
        prepared.skill_context = Some(crate::types::turn::SkillContext {
            filter: None,
            baseline: None,
        });

        let error = rebase_terminal_skill_options(&mut prepared, Some(&terminal), true)
            .expect_err("the latest terminal format controls legacy rejection");

        assert_eq!(error.code(), "harness/invalid_request");
        assert!(error.to_string().contains("legacy session"));
    }

    #[tokio::test]
    async fn authoritative_legacy_rejection_does_not_append_input() {
        let mut terminal = terminal_record_with_skill_state(2, true);
        terminal.options.skill_context = None;
        terminal.options.skills_prompt = Some("legacy body".into());
        let mut prepared = options_with(None, None);
        prepared.skill_context = Some(crate::types::turn::SkillContext {
            filter: None,
            baseline: None,
        });
        let appended = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let append_observer = appended.clone();
        let locks = crate::locks::SessionLocks::new();

        let error = with_delivery_guard(&locks, "s_1", false, || async {
            append_explicit_after_terminal_rebase(&mut prepared, Some(&terminal), || async move {
                append_observer.store(true, std::sync::atomic::Ordering::SeqCst);
                Ok("e_user".to_string())
            })
            .await
        })
        .await
        .expect_err("legacy validation must reject before append");

        assert_eq!(error.code(), "harness/invalid_request");
        assert!(!appended.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[test]
    fn ask_mode_steer_inherits_the_prior_policy_clamped() {
        let cfg = WorkerConfig::default();
        let broad = FunctionPolicy {
            allow: vec!["*".into()],
            deny: vec![],
            expose: Default::default(),
        };

        // An ask-mode steer keeps the run armed and is capped at the wildcard
        // default policy.
        let mut options = options_with(Some(Mode::Ask), None);
        inherit_prior_functions(&cfg, &mut options, Some(&broad));
        let compiled = policy::CompiledPolicy::from(options.functions.as_ref());
        assert!(compiled.allows("state::get"));
        assert!(compiled.allows("state::set"));

        // Outside ask mode the prior policy is inherited whole.
        let mut options = options_with(Some(Mode::Agent), None);
        inherit_prior_functions(&cfg, &mut options, Some(&broad));
        assert!(policy::CompiledPolicy::from(options.functions.as_ref()).allows("state::set"));

        // An explicit policy on the send still beats inheritance.
        let strip = FunctionPolicy {
            allow: vec![],
            deny: vec![],
            expose: Default::default(),
        };
        let mut options = options_with(Some(Mode::Ask), Some(strip));
        inherit_prior_functions(&cfg, &mut options, Some(&broad));
        assert!(!policy::CompiledPolicy::from(options.functions.as_ref()).allows("state::get"));
    }

    #[test]
    fn send_without_prompt_fields_inherits_prior_resolved_prompt() {
        // Omitting both prompt fields is the inherit condition…
        assert!(prompt_fields_omitted(None));
        assert!(prompt_fields_omitted(Some(&SendOptions::default())));

        // …and inheritance copies the prior turn's RESOLVED prompt verbatim,
        // replacing the built-in prompt build_options resolved.
        let mut options = bare_options();
        let mut prev = options_with(None, None);
        prev.system_prompt = Some("frozen custom prompt".into());
        prev.skills_prompt = Some("frozen skill prompt".into());
        inherit_prior_system_prompt(&mut options, &prev);
        assert_eq!(
            options.system_prompt.as_deref(),
            Some("frozen custom prompt")
        );
        assert_eq!(
            options.skills_prompt.as_deref(),
            Some("frozen skill prompt")
        );

        // A prior `disabled` turn's None inherits too — disabled stays disabled.
        let mut options = bare_options();
        assert!(options.system_prompt.is_some());
        inherit_prior_system_prompt(&mut options, &options_with(None, None));
        assert_eq!(options.system_prompt, None);
    }

    #[test]
    fn explicit_prompt_fields_block_inheritance() {
        // A named prompt on a later send wins over the prior turn's.
        let with_prompt = SendOptions {
            system_prompt: Some("p".into()),
            ..Default::default()
        };
        assert!(!prompt_fields_omitted(Some(&with_prompt)));

        // A bare strategy is the reset-to-default escape hatch: it blocks
        // inheritance, so build_options' fresh resolve (built-in prompt for
        // `enrich`, none for `disabled`) stands.
        let bare_strategy = SendOptions {
            system_prompt_strategy: Some(SystemPromptStrategy::Enrich),
            ..Default::default()
        };
        assert!(!prompt_fields_omitted(Some(&bare_strategy)));
    }

    #[test]
    fn skill_context_is_new_for_fresh_sessions_inherited_when_omitted_and_rejects_legacy_changes() {
        let view = crate::skills::EffectiveView::Removed { generation: 1 };
        let fresh = select_skill_context(None, None, &view).unwrap().unwrap();
        assert_eq!(fresh.filter, None);

        let mut previous = options_with(None, None);
        previous.skill_context = Some(crate::types::turn::SkillContext {
            filter: Some(vec!["one".into()]),
            baseline: Some("frozen".into()),
        });
        assert_eq!(
            select_skill_context(Some(&previous), None, &view).unwrap(),
            previous.skill_context
        );
        assert_eq!(
            select_skill_context(Some(&previous), Some(&[]), &view)
                .unwrap()
                .unwrap()
                .filter,
            None
        );

        previous.skill_context = None;
        previous.skills_prompt = Some("legacy body attribution".into());
        assert_eq!(
            select_skill_context(Some(&previous), None, &view).unwrap(),
            None
        );
        let error =
            select_skill_context(Some(&previous), Some(&["one".into()]), &view).unwrap_err();
        assert_eq!(error.code(), "harness/invalid_request");
        assert!(error.to_string().contains("start a new session"));
    }

    #[test]
    fn send_skills_option_is_ids_only_on_the_wire() {
        let value = serde_json::to_value(SendOptions {
            skills: Some(vec!["review".into(), "release".into()]),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(value["skills"], serde_json::json!(["review", "release"]));
    }

    #[test]
    fn active_sessions_reject_any_explicit_skill_request_but_allow_omission() {
        let error = validate_active_skill_request(true, true).unwrap_err();
        assert_eq!(error.code(), "harness/invalid_request");
        assert!(error.to_string().contains("turn is active"));

        assert!(validate_active_skill_request(true, false).is_ok());
        assert!(validate_active_skill_request(false, true).is_ok());
    }

    #[test]
    fn lost_response_retry_returns_its_idempotent_outcome_while_the_turn_is_active() {
        let existing = IdemRecord {
            session_id: "s_original".into(),
            turn_id: "t_original".into(),
            entry_id: "e_original".into(),
            ts: 1,
        };

        let outcome = resolve_send_gate(Some(existing), true, true)
            .unwrap()
            .expect("idempotency mapping resolves before active validation");

        assert_eq!(outcome.session_id, "s_original");
        assert_eq!(outcome.turn_id, "t_original");
        assert!(outcome.deduplicated);
        assert!(!outcome.merged);
        assert!(!outcome.queued);
    }

    #[test]
    fn post_append_race_merges_only_the_explicit_filter_into_the_active_context() {
        let mut active = options_with(None, None);
        active.skill_context = Some(crate::types::turn::SkillContext {
            filter: Some(vec!["old".into()]),
            baseline: Some("active frozen baseline".into()),
        });
        let mut requested = options_with(None, None);
        requested.skill_context = Some(crate::types::turn::SkillContext {
            filter: None,
            baseline: Some("stale request baseline".into()),
        });

        assert!(merge_explicit_skill_filter(&mut active, &requested, true).unwrap());
        assert_eq!(
            active.skill_context,
            Some(crate::types::turn::SkillContext {
                filter: None,
                baseline: Some("active frozen baseline".into())
            })
        );
    }

    #[test]
    fn build_options_clamps_functions_to_the_default_policy_in_ask_mode() {
        let cfg = WorkerConfig::default();
        let req = SendRequest {
            session_id: None,
            message: MessageInput::Text("hi".into()),
            model: Some("m".into()),
            provider: Some("anthropic".into()),
            idempotency_key: None,
            session: None,
            options: Some(SendOptions {
                mode: Some(Mode::Ask),
                functions: Some(FunctionPolicy {
                    allow: vec!["*".into()],
                    deny: vec![],
                    expose: Default::default(),
                }),
                ..Default::default()
            }),
        };
        let opts = build_options(
            &cfg,
            &req,
            "m".into(),
            req.provider.clone(),
            None,
            crate::prompt::DEFAULT,
        );
        let compiled = policy::CompiledPolicy::from(opts.functions.as_ref());
        // The wildcard default preserves the requested policy…
        assert!(compiled.allows("state::get"));
        assert!(compiled.allows("engine::functions::list"));
        assert!(compiled.allows("state::set"));
        assert!(compiled.allows("harness::spawn"));
        assert!(compiled.allows("engine::register_trigger"));
        assert!(compiled.allows("shell::run"));
    }

    #[test]
    fn build_options_leaves_functions_unclamped_outside_ask_mode() {
        let cfg = WorkerConfig::default();
        for mode in [Some(Mode::Agent), None] {
            let req = SendRequest {
                session_id: None,
                message: MessageInput::Text("hi".into()),
                model: Some("m".into()),
                provider: None,
                idempotency_key: None,
                session: None,
                options: Some(SendOptions {
                    mode,
                    functions: Some(FunctionPolicy {
                        allow: vec!["*".into()],
                        deny: vec![],
                        expose: Default::default(),
                    }),
                    ..Default::default()
                }),
            };
            let opts = build_options(&cfg, &req, "m".into(), None, None, crate::prompt::DEFAULT);
            let compiled = policy::CompiledPolicy::from(opts.functions.as_ref());
            assert!(
                compiled.allows("state::set"),
                "mode {mode:?} must not clamp"
            );
        }
    }

    #[test]
    fn build_options_enrich_appends_to_builtin_prompt() {
        let cfg = WorkerConfig::default();
        let req = SendRequest {
            session_id: None,
            message: MessageInput::Text("hi".into()),
            model: Some("m".into()),
            provider: Some("anthropic".into()),
            idempotency_key: None,
            session: None,
            options: Some(SendOptions {
                system_prompt: Some("Speak only in haiku.".into()),
                system_prompt_strategy: Some(SystemPromptStrategy::Enrich),
                ..Default::default()
            }),
        };
        let opts = build_options(
            &cfg,
            &req,
            "claude-sonnet-4".into(),
            req.provider.clone(),
            None,
            crate::prompt::DEFAULT,
        );
        let prompt = opts.system_prompt.expect("enriched prompt");
        assert!(prompt.starts_with(crate::prompt::DEFAULT));
        assert!(prompt.ends_with("Speak only in haiku."));
    }

    fn bare_options() -> TurnOptions {
        build_options(
            &WorkerConfig::default(),
            &SendRequest {
                session_id: None,
                message: MessageInput::Text("hi".into()),
                model: Some("m".into()),
                provider: None,
                idempotency_key: None,
                session: None,
                options: None,
            },
            "m".into(),
            None,
            None,
            crate::prompt::DEFAULT,
        )
    }

    #[test]
    fn default_root_applies_only_to_new_sessions() {
        let mut opts = bare_options();
        apply_default_filesystem_root(&mut opts, true, Some("/work/project"));
        assert_eq!(opts.filesystem_root(), Some("/work/project"));

        // An existing session's scope-less turn stays unscoped.
        let mut opts = bare_options();
        apply_default_filesystem_root(&mut opts, false, Some("/work/project"));
        assert_eq!(opts.filesystem_root(), None);
    }

    #[test]
    fn explicit_root_wins_over_default() {
        let mut opts = bare_options();
        opts.metadata = Some(serde_json::json!({ "fs_scope": { "root": "/picked" } }));
        apply_default_filesystem_root(&mut opts, true, Some("/work/project"));
        assert_eq!(opts.filesystem_root(), Some("/picked"));
    }

    #[test]
    fn default_root_merges_into_existing_metadata() {
        let mut opts = bare_options();
        opts.metadata = Some(serde_json::json!({ "session_id": "s_1" }));
        apply_default_filesystem_root(&mut opts, true, Some("/work/project"));
        assert_eq!(opts.filesystem_root(), Some("/work/project"));
        assert_eq!(
            opts.metadata.as_ref().unwrap().get("session_id"),
            Some(&serde_json::json!("s_1"))
        );
    }

    #[test]
    fn no_default_leaves_options_untouched() {
        let mut opts = bare_options();
        apply_default_filesystem_root(&mut opts, true, None);
        assert_eq!(opts.filesystem_root(), None);
        assert!(opts.metadata.is_none());
    }

    #[test]
    fn omitted_fs_scope_inherits_prior_root() {
        let mut prev = bare_options();
        prev.set_filesystem_root("/work/project");

        let mut opts = bare_options();
        inherit_prior_filesystem_root(&mut opts, &prev);
        assert_eq!(opts.filesystem_root(), Some("/work/project"));
    }

    #[test]
    fn inheritance_preserves_unrelated_metadata_keys() {
        let mut prev = bare_options();
        prev.set_filesystem_root("/work/project");

        let mut opts = bare_options();
        opts.metadata = Some(serde_json::json!({ "session_id": "s_1" }));
        inherit_prior_filesystem_root(&mut opts, &prev);
        assert_eq!(opts.filesystem_root(), Some("/work/project"));
        assert_eq!(
            opts.metadata.as_ref().unwrap().get("session_id"),
            Some(&serde_json::json!("s_1"))
        );
    }

    #[test]
    fn explicit_empty_fs_scope_clears_instead_of_inheriting() {
        let mut prev = bare_options();
        prev.set_filesystem_root("/work/project");

        let mut opts = bare_options();
        opts.metadata = Some(serde_json::json!({ "fs_scope": {} }));
        inherit_prior_filesystem_root(&mut opts, &prev);
        assert_eq!(opts.filesystem_root(), None);
    }

    #[test]
    fn explicit_root_wins_over_inheritance() {
        let mut prev = bare_options();
        prev.set_filesystem_root("/work/project");

        let mut opts = bare_options();
        opts.metadata = Some(serde_json::json!({ "fs_scope": { "root": "/picked" } }));
        inherit_prior_filesystem_root(&mut opts, &prev);
        assert_eq!(opts.filesystem_root(), Some("/picked"));
    }

    #[test]
    fn resolved_default_filesystem_root_states() {
        // Absent → the boot cwd (the local stack's launch folder).
        let cfg = WorkerConfig::default();
        let cwd = std::fs::canonicalize(std::env::current_dir().unwrap()).unwrap();
        assert_eq!(
            cfg.resolved_default_filesystem_root(),
            Some(cwd.to_string_lossy().into_owned())
        );
        // "off" → disabled.
        let cfg = WorkerConfig {
            default_filesystem_root: Some("off".into()),
            ..Default::default()
        };
        assert_eq!(cfg.resolved_default_filesystem_root(), None);
        // Explicit path → that path.
        let cfg = WorkerConfig {
            default_filesystem_root: Some("/srv/projects".into()),
            ..Default::default()
        };
        assert_eq!(
            cfg.resolved_default_filesystem_root(),
            Some("/srv/projects".to_string())
        );
    }

    fn resolved_agent(model: Option<&str>) -> crate::agents::ResolvedAgent {
        crate::agents::ResolvedAgent {
            identity: crate::types::turn::AgentIdentity {
                id: "tech-leader".into(),
                name: Some("Tech Leader".into()),
                icon: None,
                color: None,
            },
            prompt: "You are Tech Leader.\n\nDelegate everything.".into(),
            skills: Some(vec!["review".into()]),
            model: model.map(str::to_string),
            reasoning_effort: None,
            name: "Tech Leader".into(),
            icon: None,
            color: None,
        }
    }

    fn agent_send_request(options: SendOptions) -> SendRequest {
        SendRequest {
            session_id: None,
            message: MessageInput::Text("hi".into()),
            model: None,
            provider: None,
            idempotency_key: None,
            session: None,
            options: Some(options),
        }
    }

    #[test]
    fn agent_send_id_refusal_matrix() {
        let named = |options: &SendOptions| {
            agent_send_id(Some(options), false).map(|id| id.map(String::from))
        };
        // Plain agent send resolves.
        assert_eq!(
            named(&SendOptions {
                agent: Some("tech-leader".into()),
                ..Default::default()
            })
            .unwrap(),
            Some("tech-leader".to_string())
        );
        // No agent named → no resolution, whatever else is set.
        assert_eq!(agent_send_id(None, true).unwrap(), None);
        // Existing session → refused.
        let err = agent_send_id(
            Some(&SendOptions {
                agent: Some("tech-leader".into()),
                ..Default::default()
            }),
            true,
        )
        .unwrap_err();
        assert_eq!(err.code(), "harness/invalid_request");
        assert!(err.to_string().contains("starting a session"));
        // Combined with either explicit prompt field → refused.
        for options in [
            SendOptions {
                agent: Some("tech-leader".into()),
                system_prompt: Some("be terse".into()),
                ..Default::default()
            },
            SendOptions {
                agent: Some("tech-leader".into()),
                system_prompt_strategy: Some(SystemPromptStrategy::Enrich),
                ..Default::default()
            },
        ] {
            let err = named(&options).unwrap_err();
            assert!(err.to_string().contains("drop `system_prompt`"), "{err}");
        }
    }

    /// The profile prompt IS the identity: no built-in prompt underneath,
    /// and the mode paragraph is the only layer the harness adds in front.
    #[test]
    fn build_options_applies_agent_prompt_as_the_identity() {
        let cfg = WorkerConfig::default();
        let agent = resolved_agent(None);

        let req = agent_send_request(SendOptions {
            agent: Some("tech-leader".into()),
            ..Default::default()
        });
        let opts = build_options(
            &cfg,
            &req,
            "m".into(),
            None,
            Some(&agent),
            crate::prompt::DEFAULT,
        );
        assert_eq!(
            opts.system_prompt.as_deref(),
            Some(agent.prompt.as_str()),
            "override, not enrich"
        );
        assert_eq!(opts.agent, Some(agent.identity.clone()));

        let req = agent_send_request(SendOptions {
            agent: Some("tech-leader".into()),
            mode: Some(Mode::Agent),
            ..Default::default()
        });
        let opts = build_options(
            &cfg,
            &req,
            "m".into(),
            None,
            Some(&agent),
            crate::prompt::DEFAULT,
        );
        let prompt = opts.system_prompt.expect("agent prompt");
        assert!(prompt.starts_with("You are operating in agent mode"));
        assert!(prompt.ends_with(&format!("\n\n{}", agent.prompt)));
        assert!(
            !prompt.contains("# System rules"),
            "the built-in identity never rides under a profile"
        );
    }

    #[test]
    fn build_options_applies_the_profile_reasoning_effort_exactly() {
        let cfg = WorkerConfig::default();
        let req = agent_send_request(SendOptions {
            agent: Some("tech-leader".into()),
            thinking_level: Some(ThinkingLevel::Low),
            ..Default::default()
        });
        let mut agent = resolved_agent(Some("openai-codex::codex/gpt-5.6-sol"));
        agent.reasoning_effort = Some("high".into());
        let opts = build_options(
            &cfg,
            &req,
            "codex/gpt-5.6-sol".into(),
            Some("openai-codex".into()),
            Some(&agent),
            crate::prompt::DEFAULT,
        );
        assert_eq!(opts.thinking_level, Some(ThinkingLevel::High));
        assert_eq!(
            opts.provider_options.unwrap()["openai-codex"],
            serde_json::json!({ "reasoning_effort": "high" })
        );
    }

    #[test]
    fn agent_session_metadata_preserves_existing_keys() {
        let mut agent = resolved_agent(Some("openai-codex::codex/gpt-5.6-sol"));
        agent.reasoning_effort = Some("high".into());
        let metadata = session_metadata_with_agent(
            Some(serde_json::json!({
                "surface": "console",
                "fs_scope": { "root": "/workspace" },
            })),
            Some(&agent),
        )
        .unwrap();

        assert_eq!(metadata["surface"], "console");
        assert_eq!(metadata["fs_scope"]["root"], "/workspace");
        assert_eq!(metadata["agent_profile"]["id"], "tech-leader");
        assert_eq!(
            metadata["agent_profile"]["model"],
            "openai-codex::codex/gpt-5.6-sol"
        );
        assert_eq!(metadata["agent_profile"]["reasoning_effort"], "high");
    }

    #[test]
    fn agent_send_defaults_functions_to_the_configured_baseline() {
        let cfg = WorkerConfig::default();
        let agent = resolved_agent(None);
        let req = agent_send_request(SendOptions {
            agent: Some("tech-leader".into()),
            ..Default::default()
        });
        // Absent policy + agent → the configured default applies.
        let mut opts = build_options(
            &cfg,
            &req,
            "m".into(),
            None,
            Some(&agent),
            crate::prompt::DEFAULT,
        );
        inherit_prior_functions(
            &cfg,
            &mut opts,
            None.or_else(|| Some(&agent).and(cfg.default_functions.as_ref())),
        );
        let compiled = policy::CompiledPolicy::from(opts.functions.as_ref());
        assert!(compiled.allows("harness::spawn"));
        assert!(compiled.allows("state::set"));
        // Explicit policy wins over the agent default.
        let req = agent_send_request(SendOptions {
            agent: Some("tech-leader".into()),
            functions: Some(FunctionPolicy {
                allow: vec!["state::get".into()],
                deny: vec![],
                expose: Default::default(),
            }),
            ..Default::default()
        });
        let mut opts = build_options(
            &cfg,
            &req,
            "m".into(),
            None,
            Some(&agent),
            crate::prompt::DEFAULT,
        );
        inherit_prior_functions(
            &cfg,
            &mut opts,
            None.or_else(|| Some(&agent).and(cfg.default_functions.as_ref())),
        );
        let compiled = policy::CompiledPolicy::from(opts.functions.as_ref());
        assert!(compiled.allows("state::get"));
        assert!(!compiled.allows("harness::spawn"));
        // Ask mode still clamps: the shipped wildcard baseline is identity, so
        // the agent default survives — same as an explicit `allow:["*"]` ask
        // send. A narrowed operator baseline stays authoritative.
        let narrow_cfg = WorkerConfig {
            default_functions: Some(FunctionPolicy {
                allow: vec!["state::get".into()],
                deny: vec![],
                expose: Default::default(),
            }),
            ..Default::default()
        };
        let req = agent_send_request(SendOptions {
            agent: Some("tech-leader".into()),
            mode: Some(Mode::Ask),
            ..Default::default()
        });
        let mut opts = build_options(
            &narrow_cfg,
            &req,
            "m".into(),
            None,
            Some(&agent),
            crate::prompt::DEFAULT,
        );
        inherit_prior_functions(
            &narrow_cfg,
            &mut opts,
            None.or_else(|| Some(&agent).and(narrow_cfg.default_functions.as_ref())),
        );
        let compiled = policy::CompiledPolicy::from(opts.functions.as_ref());
        assert!(compiled.allows("state::get"));
        assert!(!compiled.allows("harness::spawn"), "ask cap holds");
    }

    #[test]
    fn agent_identity_inherits_with_the_prompt_and_sheds_with_it() {
        let cfg = WorkerConfig::default();
        let agent = resolved_agent(None);
        let req = agent_send_request(SendOptions {
            agent: Some("tech-leader".into()),
            ..Default::default()
        });
        let prev = build_options(
            &cfg,
            &req,
            "m".into(),
            None,
            Some(&agent),
            crate::prompt::DEFAULT,
        );

        // A bare steer inherits prompt AND identity.
        let mut next = bare_options();
        inherit_prior_system_prompt(&mut next, &prev);
        assert_eq!(next.system_prompt, prev.system_prompt);
        assert_eq!(next.agent, prev.agent);

        // An explicit prompt field resolves fresh — no inherit call — and the
        // freshly built options carry no identity.
        let explicit = build_options(
            &cfg,
            &agent_send_request(SendOptions {
                system_prompt: Some("be terse".into()),
                ..Default::default()
            }),
            "m".into(),
            None,
            None,
            crate::prompt::DEFAULT,
        );
        assert_eq!(explicit.agent, None);
    }
}

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
use crate::prompt::{self, Mode, SystemPromptStrategy};
use crate::turn_loop;
use crate::types::message::{AgentMessage, UserMessage, UserRoleTag};
use crate::types::model::ThinkingLevel;
use crate::types::output::OutputContract;
use crate::types::turn::{FunctionPolicy, IdemRecord, TurnOptions, TurnRecord, TurnStatus};

/// `message` is either a plain string (sugar for a user text message) or a
/// full `AgentMessage`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum MessageInput {
    Text(String),
    Message(Box<AgentMessage>),
}

/// Per-send options frozen onto the turn record (harness.md § `harness::send`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct SendOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
    /// How `system_prompt` combines with the built-in prompt: `override`
    /// replaces it; `enrich` (default) appends to it.
    #[serde(default)]
    pub system_prompt_strategy: SystemPromptStrategy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<u32>,
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
    /// effective policy is capped at the operator's read-only baseline; a
    /// steer folded into an already-running turn keeps that turn's frozen
    /// policy until it finalises.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions: Option<FunctionPolicy>,
    /// Tracing passthrough.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
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
    /// Required to start a NEW session. Steering or waking an EXISTING
    /// session may omit it — the session's last turn's model (and provider,
    /// unless overridden) is inherited, the same rule the notification
    /// inject path uses.
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
    let out = start(deps, req).await?;
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
    let cfg = deps.cfg().await;
    let session = deps.session().await;

    // Steering an EXISTING session may omit `model`: inherit the last turn's
    // model (and provider, unless overridden) instead of failing on a raw
    // missing-field error — a user nudging a live run should not have to
    // re-name the model every time. A NEW session has nothing to inherit.
    let prev = match req.session_id.as_deref() {
        Some(sid) => crate::state::get_turn(&deps.iii, sid, cfg.session_timeout_ms).await?,
        None => None,
    };
    let (model, provider) = match (req.model.clone(), &prev) {
        (Some(m), _) => (m, req.provider.clone()),
        (None, Some(prev)) => (
            prev.options.model.clone(),
            req.provider
                .clone()
                .or_else(|| prev.options.provider.clone()),
        ),
        (None, None) if req.session_id.is_some() => {
            return Err(HarnessError::InvalidRequest(
                "harness::send without `model` inherits from the session's prior \
                 turn, but this session has none — name a `model`"
                    .into(),
            ))
        }
        (None, None) => {
            return Err(HarnessError::InvalidRequest(
                "harness::send creating a NEW session requires `model` (steering an \
                 existing session may omit it)"
                    .into(),
            ))
        }
    };

    // Freeze the per-send options before moving the message out of `req`.
    // The provider identity prompt is fetched once here and frozen with them.
    let identity = deps
        .router()
        .await
        .system_prompt_get(provider.as_deref())
        .await;
    let mut options = build_options(&cfg, &req, model, provider, identity.as_deref());
    inherit_prior_functions(
        &cfg,
        &mut options,
        prev.as_ref().and_then(|p| p.options.functions.as_ref()),
    );

    // Normalise the incoming message and validate its role.
    let message = normalize_message(req.message)?;

    // Idempotency: a repeated key returns the original mapping unchanged.
    if let Some(key) = &req.idempotency_key {
        if let Some(existing) =
            crate::state::get_idem(&deps.iii, key, cfg.session_timeout_ms).await?
        {
            return Ok(StartOutcome {
                session_id: existing.session_id,
                turn_id: existing.turn_id,
                merged: false,
                queued: false,
                deduplicated: true,
            });
        }
    }

    // Resolve the session (ensure if id given, else create).
    let (title, metadata) = req
        .session
        .as_ref()
        .map(|s| (s.title.clone(), s.metadata.clone()))
        .unwrap_or((None, None));
    let session_id = match &req.session_id {
        Some(id) => {
            session
                .ensure(id, title.as_deref(), metadata.as_ref())
                .await?;
            id.clone()
        }
        None => session.create(title.as_deref(), metadata.as_ref()).await?,
    };

    // Entry id: idempotent when a dedupe key is set.
    let entry_id = req
        .idempotency_key
        .as_ref()
        .map(|k| ids::idem_user_entry_id(k));

    // Queue path: a `Running` step may be streaming — park the message as a
    // durable queue row the loop drains after the stream ends, instead of
    // appending mid-transcript.
    let (outcome, entry) = match try_enqueue(
        deps,
        &cfg,
        &session_id,
        &message,
        entry_id.as_deref(),
        None,
        &options,
    )
    .await?
    {
        Some((outcome, row_entry)) => (outcome, row_entry),
        None => {
            // Append path (no turn / terminal / parked): persist the message,
            // then CAS-seed the turn (or take the merge path).
            let appended_entry = session
                .append(&session_id, &message, entry_id.as_deref(), None, None)
                .await?;
            let preview = message_preview(&message);
            let outcome = seed_or_merge(deps, &cfg, &session_id, options, preview).await?;
            (outcome, appended_entry)
        }
    };

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
    let session = deps.session().await;

    let options = crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms)
        .await?
        .map(|rec| rec.options)
        .ok_or_else(|| {
            HarnessError::InvalidRequest(format!(
                "cannot deliver notification to session `{session_id}`: it has no prior turn to \
                 inherit model/options from"
            ))
        })?;

    if let Some((outcome, _)) =
        try_enqueue(deps, &cfg, session_id, &message, entry_id, origin, &options).await?
    {
        return Ok(outcome);
    }
    let preview = message_preview(&message);
    session
        .append(session_id, &message, entry_id, None, origin)
        .await?;
    seed_or_merge(deps, &cfg, session_id, options, preview).await
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
/// still-live turn drains the row at its next step; a turn that went terminal
/// in the window gets a fresh turn seeded, whose step-0 drain delivers the
/// row. A row landing after the loop's last queue check is appended by the
/// finalize drain — queued messages are never silently dropped.
async fn try_enqueue(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    message: &AgentMessage,
    entry_id: Option<&str>,
    origin: Option<&Value>,
    options: &TurnOptions,
) -> Result<Option<(StartOutcome, String)>, HarnessError> {
    let existing = crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms).await?;
    let prior_generation = existing.as_ref().and_then(|r| r.functions_generation);
    match existing {
        Some(rec) if rec.status == TurnStatus::Running => {}
        _ => return Ok(None),
    }

    let id = ids::new_queued_id();
    let entry_id = entry_id
        .map(str::to_string)
        .unwrap_or_else(|| ids::queued_entry_id(&id));
    let row = crate::state::QueuedMessage {
        id,
        session_id: session_id.to_string(),
        message: message.clone(),
        entry_id: entry_id.clone(),
        origin: origin.cloned(),
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
        Some(mut r) if !r.status.is_terminal() => {
            if r.options.refresh_filesystem_root_from(options) {
                r.updated_at = AgentMessage::now_ms();
                crate::state::put_turn(&deps.iii, &r, cfg.session_timeout_ms).await?;
            }
            StartOutcome {
                session_id: session_id.to_string(),
                turn_id: r.turn_id,
                merged: true,
                queued: true,
                deduplicated: false,
            }
        }
        _ => {
            let mut seeded = seed_new(
                deps,
                cfg,
                session_id,
                options.clone(),
                prior_generation,
                message_preview(message),
            )
            .await?;
            seeded.queued = true;
            seeded
        }
    };
    Ok(Some((outcome, entry_id)))
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
    identity: Option<&str>,
) -> TurnOptions {
    let opts = req.options.clone().unwrap_or_default();
    let functions = clamp_for_mode(cfg, opts.mode, opts.functions);
    TurnOptions {
        model,
        provider,
        system_prompt: prompt::resolve_system_prompt(
            opts.system_prompt,
            opts.system_prompt_strategy,
            opts.mode,
            identity,
        ),
        mode: opts.mode,
        max_turns: opts.max_turns.unwrap_or(cfg.default_max_turns),
        thinking_level: opts.thinking_level,
        provider_options: opts.provider_options,
        output: opts.output.unwrap_or_default(),
        functions,
        metadata: opts.metadata,
        max_validation_retries: cfg.max_validation_retries,
        max_transient_resumes: cfg.max_transient_resumes,
    }
}

/// The single chokepoint for the "ask is structurally read-only" invariant:
/// every turn-seeding path (a fresh send, an inherited steer, a spawned child)
/// routes its resolved dispatch policy through here, so ask mode is capped at
/// the operator's read-only baseline no matter how the policy was assembled.
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
) -> Result<StartOutcome, HarnessError> {
    let existing = crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms).await?;
    // Carry the session's last-acknowledged registry generation onto a new turn
    // so run_step can notice a registry change that landed between turns.
    let prior_generation = existing.as_ref().and_then(|r| r.functions_generation);
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
                    if r.options.refresh_filesystem_root_from(&options) {
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
                _ => {
                    seed_new(
                        deps,
                        cfg,
                        session_id,
                        options,
                        prior_generation,
                        message_preview,
                    )
                    .await
                }
            }
        }
        _ => {
            seed_new(
                deps,
                cfg,
                session_id,
                options,
                prior_generation,
                message_preview,
            )
            .await
        }
    }
}

/// Seed a fresh turn record and enqueue its first step. Exposed to the turn
/// loop's finalize-drain reseed path (`turn_loop::reseed_after_finalize_drain`)
/// so a notification that parked during a turn's final step gets a turn to
/// react to it, instead of being drained to the transcript and stranded.
pub(crate) async fn seed_new(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    options: TurnOptions,
    functions_generation: Option<u64>,
    message_preview: Option<String>,
) -> Result<StartOutcome, HarnessError> {
    let turn_id = ids::new_turn_id();
    let now = AgentMessage::now_ms();
    let record = TurnRecord {
        turn_id: turn_id.clone(),
        session_id: session_id.to_string(),
        status: TurnStatus::Running,
        step: 0,
        turn_count: 0,
        depth: 0,
        message_preview,
        abort: false,
        watermark_entry_id: None,
        stream_request_id: None,
        options,
        calls: Default::default(),
        parent: None,
        display_parent_session_id: None,
        spawned_by_subscription_id: None,
        reactive_depth: None,
        functions_generation,
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
        0,
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
    fn build_options_applies_builtin_prompt_when_system_prompt_omitted() {
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
        // Router-served identity used when present…
        let opts = build_options(
            &cfg,
            &req,
            "claude-sonnet-4".into(),
            req.provider.clone(),
            Some("You are an iii agent worker. VOICE."),
        );
        let prompt = opts.system_prompt.expect("built-in prompt");
        assert!(prompt.contains("operating in agent mode"));
        assert!(prompt.ends_with("You are an iii agent worker. VOICE."));
        // …embedded default when the router serves none.
        let opts = build_options(&cfg, &req, "m".into(), req.provider.clone(), None);
        let prompt = opts.system_prompt.expect("built-in prompt");
        assert!(prompt.contains("operating in agent mode"));
        assert!(prompt.contains("# The steps for every action"));
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
                system_prompt_strategy: SystemPromptStrategy::Override,
                mode: Some(Mode::Ask),
                ..Default::default()
            }),
        };
        let opts = build_options(
            &cfg,
            &req,
            "claude-sonnet-4".into(),
            req.provider.clone(),
            Some("You are an iii agent worker. VOICE."),
        );
        assert_eq!(opts.system_prompt.as_deref(), Some("custom"));
    }

    fn options_with(mode: Option<Mode>, functions: Option<FunctionPolicy>) -> TurnOptions {
        TurnOptions {
            model: "m".into(),
            provider: None,
            system_prompt: None,
            mode,
            max_turns: 16,
            thinking_level: None,
            provider_options: None,
            output: OutputContract::Text,
            functions,
            metadata: None,
            max_validation_retries: 2,
            max_transient_resumes: 1,
        }
    }

    #[test]
    fn ask_mode_steer_inherits_the_prior_policy_clamped() {
        let cfg = WorkerConfig::default();
        let broad = FunctionPolicy {
            allow: vec!["*".into()],
            deny: vec![],
            expose: Default::default(),
        };

        // An ask-mode steer keeps the run armed but capped at the baseline.
        let mut options = options_with(Some(Mode::Ask), None);
        inherit_prior_functions(&cfg, &mut options, Some(&broad));
        let compiled = policy::CompiledPolicy::from(options.functions.as_ref());
        assert!(compiled.allows("state::get"));
        assert!(!compiled.allows("state::set"));

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
    fn build_options_clamps_functions_to_the_read_only_baseline_in_ask_mode() {
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
        let opts = build_options(&cfg, &req, "m".into(), req.provider.clone(), None);
        let compiled = policy::CompiledPolicy::from(opts.functions.as_ref());
        // Baseline reads survive the clamp…
        assert!(compiled.allows("state::get"));
        assert!(compiled.allows("engine::functions::list"));
        // …every write/orchestration surface is out, wildcard request or not.
        for denied in [
            "state::set",
            "harness::spawn",
            "engine::register_trigger",
            "shell::run",
        ] {
            assert!(
                !compiled.allows(denied),
                "{denied} must be denied in ask mode"
            );
        }
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
            let opts = build_options(&cfg, &req, "m".into(), None, None);
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
                system_prompt_strategy: SystemPromptStrategy::Enrich,
                ..Default::default()
            }),
        };
        let opts = build_options(
            &cfg,
            &req,
            "claude-sonnet-4".into(),
            req.provider.clone(),
            Some("You are an iii agent worker. VOICE."),
        );
        let prompt = opts.system_prompt.expect("enriched prompt");
        assert!(prompt.starts_with("You are an iii agent worker. VOICE."));
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
}

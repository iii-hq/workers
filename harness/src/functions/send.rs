//! `harness::send` — the entry point: ensure the session, persist the incoming
//! user message, and CAS-seed the first turn step (or merge into a running
//! turn). Returns fast (harness.md § `harness::send`).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::WorkerConfig;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::ids;
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
    /// The turn's deliverable; default `{ type: "text" }`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<OutputContract>,
    /// The fail-closed dispatch policy; omit to deny every call.
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
    pub model: String,
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
    /// True when `idempotency_key` matched an earlier send.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deduplicated: Option<bool>,
}

/// The shared result of starting (or merging) a turn.
pub struct StartOutcome {
    pub session_id: String,
    pub turn_id: String,
    pub merged: bool,
    pub deduplicated: bool,
}

pub async fn handle(deps: &Deps, req: SendRequest) -> Result<SendResponse, HarnessError> {
    let out = start(deps, req).await?;
    Ok(SendResponse {
        session_id: out.session_id,
        turn_id: out.turn_id,
        accepted: true,
        merged: out.merged.then_some(true),
        deduplicated: out.deduplicated.then_some(true),
    })
}

/// Ensure the session, persist the message, and seed/merge the turn.
pub async fn start(deps: &Deps, req: SendRequest) -> Result<StartOutcome, HarnessError> {
    let cfg = deps.cfg().await;
    let session = deps.session().await;

    // Freeze the per-send options before moving the message out of `req`.
    let options = build_options(&cfg, &req);

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

    // Persist the user message (idempotent entry id when a dedupe key is set).
    let entry_id = req
        .idempotency_key
        .as_ref()
        .map(|k| ids::idem_user_entry_id(k));
    let appended_entry = session
        .append(&session_id, &message, entry_id.as_deref(), None, None)
        .await?;

    // CAS-seed the turn (or take the merge path).
    let outcome = seed_or_merge(deps, &cfg, &session_id, options).await?;

    // Record the idempotency mapping (TTL-bound by contract).
    if let Some(key) = &req.idempotency_key {
        let rec = IdemRecord {
            session_id: session_id.clone(),
            turn_id: outcome.turn_id.clone(),
            entry_id: appended_entry,
            ts: AgentMessage::now_ms(),
        };
        let _ = crate::state::put_idem(&deps.iii, key, &rec, cfg.session_timeout_ms).await;
    }

    Ok(outcome)
}

/// Inject a message into an EXISTING session and wake/steer a turn, reusing the
/// session's last turn options (model / provider / dispatch policy / prompt) so
/// a woken turn keeps the agent's capabilities. Used by ephemeral subscriptions
/// to deliver a notification without polling — the opposite of `harness::spawn`:
/// the agent is never parked; the event arrives as a message.
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

    // Append into the existing session (no `ensure` — a deleted session must
    // not be resurrected; its subscriptions are cleaned on session::deleted).
    session
        .append(session_id, &message, entry_id, None, None)
        .await?;
    seed_or_merge(deps, &cfg, session_id, options).await
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

fn build_options(cfg: &WorkerConfig, req: &SendRequest) -> TurnOptions {
    let opts = req.options.clone().unwrap_or_default();
    TurnOptions {
        model: req.model.clone(),
        provider: req.provider.clone(),
        system_prompt: prompt::resolve_system_prompt(
            opts.system_prompt,
            opts.system_prompt_strategy,
            opts.mode,
            req.provider.as_deref(),
        ),
        mode: opts.mode,
        max_turns: opts.max_turns.unwrap_or(cfg.default_max_turns),
        thinking_level: opts.thinking_level,
        output: opts.output.unwrap_or_default(),
        functions: opts.functions,
        metadata: opts.metadata,
        max_validation_retries: cfg.max_validation_retries,
    }
}

/// The turn CAS + merge double-check (harness.md § Concurrency & steering).
async fn seed_or_merge(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    options: TurnOptions,
) -> Result<StartOutcome, HarnessError> {
    let existing = crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms).await?;
    match existing {
        Some(rec) if !rec.status.is_terminal() => {
            // Merge path: the message is already appended; the running loop's
            // steering check folds it in. Double-check the record didn't go
            // terminal in the append window.
            let recheck =
                crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms).await?;
            match recheck {
                Some(r) if !r.status.is_terminal() => Ok(StartOutcome {
                    session_id: session_id.to_string(),
                    turn_id: r.turn_id,
                    merged: true,
                    deduplicated: false,
                }),
                _ => seed_new(deps, cfg, session_id, options).await,
            }
        }
        _ => seed_new(deps, cfg, session_id, options).await,
    }
}

async fn seed_new(
    deps: &Deps,
    cfg: &WorkerConfig,
    session_id: &str,
    options: TurnOptions,
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
        abort: false,
        watermark_entry_id: None,
        stream_request_id: None,
        options,
        calls: Default::default(),
        parent: None,
        result: None,
        result_error: None,
        validation_retries: 0,
        created_at: now,
        updated_at: now,
    };
    crate::state::put_turn(&deps.iii, &record, cfg.session_timeout_ms).await?;
    turn_loop::enqueue_step(&deps.iii, session_id, &turn_id, 0).await?;
    Ok(StartOutcome {
        session_id: session_id.to_string(),
        turn_id,
        merged: false,
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
    fn build_options_applies_builtin_prompt_when_system_prompt_omitted() {
        let cfg = WorkerConfig::default();
        let req = SendRequest {
            session_id: None,
            message: MessageInput::Text("hi".into()),
            model: "claude-sonnet-4".into(),
            provider: Some("anthropic".into()),
            idempotency_key: None,
            session: None,
            options: Some(SendOptions {
                mode: Some(Mode::Agent),
                ..Default::default()
            }),
        };
        let opts = build_options(&cfg, &req);
        let prompt = opts.system_prompt.expect("built-in prompt");
        assert!(prompt.contains("operating in agent mode"));
        assert!(prompt.contains("IMPORTANT: NEVER invent function ids"));
    }

    #[test]
    fn build_options_honors_non_empty_system_prompt_override() {
        let cfg = WorkerConfig::default();
        let req = SendRequest {
            session_id: None,
            message: MessageInput::Text("hi".into()),
            model: "m".into(),
            provider: Some("anthropic".into()),
            idempotency_key: None,
            session: None,
            options: Some(SendOptions {
                system_prompt: Some("custom".into()),
                system_prompt_strategy: SystemPromptStrategy::Override,
                mode: Some(Mode::Plan),
                ..Default::default()
            }),
        };
        let opts = build_options(&cfg, &req);
        assert_eq!(opts.system_prompt.as_deref(), Some("custom"));
    }

    #[test]
    fn build_options_enrich_appends_to_builtin_prompt() {
        let cfg = WorkerConfig::default();
        let req = SendRequest {
            session_id: None,
            message: MessageInput::Text("hi".into()),
            model: "m".into(),
            provider: Some("anthropic".into()),
            idempotency_key: None,
            session: None,
            options: Some(SendOptions {
                system_prompt: Some("Speak only in haiku.".into()),
                system_prompt_strategy: SystemPromptStrategy::Enrich,
                ..Default::default()
            }),
        };
        let opts = build_options(&cfg, &req);
        let prompt = opts.system_prompt.expect("enriched prompt");
        assert!(prompt.contains("IMPORTANT: NEVER invent function ids"));
        assert!(prompt.ends_with("Speak only in haiku."));
    }
}

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
    // The provider identity prompt is fetched once here and frozen with them.
    let identity = deps
        .router()
        .await
        .system_prompt_get(req.provider.as_deref())
        .await;
    let options = build_options(&cfg, &req, identity.as_deref());

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
    let preview = message_preview(&message);
    let outcome = seed_or_merge(deps, &cfg, &session_id, options, preview).await?;

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

    let preview = message_preview(&message);
    session
        .append(session_id, &message, entry_id, None, origin)
        .await?;
    seed_or_merge(deps, &cfg, session_id, options, preview).await
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

fn build_options(cfg: &WorkerConfig, req: &SendRequest, identity: Option<&str>) -> TurnOptions {
    let opts = req.options.clone().unwrap_or_default();
    TurnOptions {
        model: req.model.clone(),
        provider: req.provider.clone(),
        system_prompt: prompt::resolve_system_prompt(
            opts.system_prompt,
            opts.system_prompt_strategy,
            opts.mode,
            identity,
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

async fn seed_new(
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
        // Router-served identity used when present…
        let opts = build_options(&cfg, &req, Some("You are an iii agent worker. VOICE."));
        let prompt = opts.system_prompt.expect("built-in prompt");
        assert!(prompt.contains("operating in agent mode"));
        assert!(prompt.ends_with("You are an iii agent worker. VOICE."));
        // …embedded default when the router serves none.
        let opts = build_options(&cfg, &req, None);
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
        let opts = build_options(&cfg, &req, Some("You are an iii agent worker. VOICE."));
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
        let opts = build_options(&cfg, &req, Some("You are an iii agent worker. VOICE."));
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
                model: "m".into(),
                provider: None,
                idempotency_key: None,
                session: None,
                options: None,
            },
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

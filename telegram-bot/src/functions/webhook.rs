use std::sync::Arc;

use iii_sdk::{IIIError, III};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

use crate::clients::{approval, harness, router, telegram};
use crate::config::{ModelRef, SteeringMode, UpdatesAdapter, WorkerConfig};
use crate::deps::Deps;
use crate::functions::bindings::message_added::BindingAck;
use crate::kv;
use crate::preferences::{self, parse_thinking_level, parse_verbosity, thinking_level_wire};
use crate::telemetry;
use crate::types::{
    ChatFsm, HttpTriggerRequest, PendingApprovalRecord, ResolveDecision, TelegramUpdate,
};

#[derive(Debug, Serialize, JsonSchema)]
pub struct WebhookResponse {
    pub ok: bool,
}

pub fn register(iii: &Arc<III>, deps: &Arc<Deps>) {
    super::register(
        iii,
        deps,
        super::WEBHOOK_ID,
        "Receive Telegram updates; route commands, messages, and callbacks.",
        |d, req| async move { handle(&d, req).await },
    );
}

pub async fn handle(deps: &Deps, req: HttpTriggerRequest) -> Result<WebhookResponse, IIIError> {
    let cfg = deps.cfg().await;
    if let UpdatesAdapter::Webhook(webhook) = &cfg.updates {
        // Validate the secret token only when one is configured. A secret-less
        // webhook is accepted (the operator opted out of validation) so ingress
        // works without it, but it is strongly recommended — without it anyone
        // who learns the URL can inject forged updates.
        match webhook.secret.as_deref().filter(|s| !s.is_empty()) {
            Some(secret) => {
                if !header_matches(&req.headers, secret) {
                    return Err(IIIError::Handler("invalid webhook secret".into()));
                }
            }
            None => {
                tracing::warn!(
                    "webhook update accepted without secret validation; set updates.webhook.config.secret"
                );
            }
        }
    } else {
        return Err(IIIError::Handler(
            "webhook HTTP ingress is disabled; updates adapter is not webhook".into(),
        ));
    }

    let update: TelegramUpdate = serde_json::from_value(req.body)
        .map_err(|e| IIIError::Handler(format!("invalid telegram update: {e}")))?;

    process_update_with_tracing(deps, update).await?;
    Ok(WebhookResponse { ok: true })
}

/// Resolve harness session id for trace baggage from a Telegram update.
async fn baggage_session_for_update(deps: &Deps, update: &TelegramUpdate) -> String {
    let chat_id = update.message.as_ref().map(|m| m.chat.id).or_else(|| {
        update
            .callback_query
            .as_ref()
            .and_then(|cb| cb.message.as_ref())
            .map(|m| m.chat.id)
    });
    if let Some(chat_id) = chat_id {
        if let Some(sid) = kv::chat_session(deps, chat_id).await {
            return sid;
        }
        return telemetry::pending_session_id(chat_id);
    }
    "telegram-update".into()
}

pub async fn process_update_with_tracing(
    deps: &Deps,
    update: TelegramUpdate,
) -> Result<(), IIIError> {
    let session_id = baggage_session_for_update(deps, &update).await;
    let message_id = telemetry::telegram_message_id(update.update_id);
    telemetry::with_baggage(&session_id, &message_id, || async {
        process_update(deps, update).await
    })
    .await
}

pub async fn process_update(deps: &Deps, update: TelegramUpdate) -> Result<(), IIIError> {
    if let Some(cb) = update.callback_query {
        return handle_callback(deps, cb).await;
    }

    if let Some(message) = update.message {
        let update_id = update.update_id;
        return handle_message(deps, update_id, message).await;
    }

    Ok(())
}

async fn handle_message(
    deps: &Deps,
    update_id: i64,
    message: crate::types::TelegramMessage,
) -> Result<(), IIIError> {
    let chat_id = message.chat.id;
    let cfg = deps.cfg().await;

    if let Some(text) = message.text.as_deref() {
        if text.starts_with('/') {
            return handle_command(deps, chat_id, text, &cfg).await;
        }
    }

    let user_text = extract_user_content(&message);
    if user_text.is_empty() {
        return Ok(());
    }

    let session_id = kv::chat_session(deps, chat_id).await;
    if let Some(ref sid) = session_id {
        catch_up_approvals(deps, sid, &cfg).await;
    }

    if kv::get_fsm(deps, chat_id).await == ChatFsm::AwaitingModel {
        telegram::send_message(
            deps,
            chat_id,
            "Pick a model with /start or /model first.",
            None,
            None,
        )
        .await?;
        return Ok(());
    }

    let Some(model) = kv::chat_model(deps, chat_id).await else {
        telegram::send_message(deps, chat_id, "No model selected. Send /start.", None, None)
            .await?;
        return Ok(());
    };

    if cfg.steering_mode == SteeringMode::Fifo {
        if let Some(ref sid) = session_id {
            match harness::status_active(&deps.iii, sid, cfg.timeout_ms).await {
                Ok(true) => {
                    deps.runtime
                        .fifo_queues
                        .entry(chat_id)
                        .or_default()
                        .push(user_text);
                    return Ok(());
                }
                Ok(false) => {}
                Err(e) => {
                    tracing::warn!(error = %e, session_id = %sid, "harness::status failed; queueing message");
                    deps.runtime
                        .fifo_queues
                        .entry(chat_id)
                        .or_default()
                        .push(user_text);
                    return Ok(());
                }
            }
        }
    }

    send_user_message(
        deps,
        chat_id,
        session_id.as_deref(),
        &user_text,
        Some(update_id),
        &cfg,
        &model,
    )
    .await?;
    Ok(())
}

fn extract_user_content(message: &crate::types::TelegramMessage) -> String {
    if let Some(text) = &message.text {
        if !text.is_empty() {
            return text.clone();
        }
    }
    if let Some(caption) = &message.caption {
        if !caption.is_empty() {
            return caption.clone();
        }
    }
    if message.photo.is_some() {
        return "[User sent a photo]".into();
    }
    if message.document.is_some() {
        return "[User sent a document]".into();
    }
    if message.voice.is_some() {
        return "[User sent a voice message]".into();
    }
    String::new()
}

async fn handle_command(
    deps: &Deps,
    chat_id: i64,
    text: &str,
    cfg: &WorkerConfig,
) -> Result<(), IIIError> {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let cmd = parts.first().copied().unwrap_or(text);
    let arg = parts.get(1).copied();

    match cmd {
        "/start" => start_flow(deps, chat_id, arg, cfg).await,
        "/stop" => {
            if let Some(session_id) = kv::chat_session(deps, chat_id).await {
                harness::stop(&deps.iii, &session_id, cfg.timeout_ms).await?;
            }
            telegram::send_message(deps, chat_id, "Stopped.", None, None).await?;
            Ok(())
        }
        "/model" => show_model_picker(deps, chat_id, cfg).await,
        "/help" => help_message(deps, chat_id).await,
        "/thinking" => thinking_command(deps, chat_id, arg, cfg).await,
        "/verbosity" => verbosity_command(deps, chat_id, arg, cfg).await,
        "/settings" => settings_message(deps, chat_id, cfg).await,
        _ => {
            telegram::send_message(deps, chat_id, "Unknown command. Try /help.", None, None)
                .await?;
            Ok(())
        }
    }
}

async fn help_message(deps: &Deps, chat_id: i64) -> Result<(), IIIError> {
    let text = "/start — start a new session and pick a model\n\
/stop — stop the current turn\n\
/model — change model\n\
/thinking — set reasoning depth (off|minimal|low|medium|high|xhigh)\n\
/verbosity — set transcript detail (none|minimal|high|debug)\n\
/settings — show current preferences\n\
/help — this message";
    telegram::send_message(deps, chat_id, text, None, None).await?;
    Ok(())
}

async fn settings_message(deps: &Deps, chat_id: i64, cfg: &WorkerConfig) -> Result<(), IIIError> {
    let verbosity = preferences::effective_verbosity(deps, chat_id, cfg).await;
    let thinking = preferences::effective_thinking_level(deps, chat_id, cfg)
        .await
        .map(|l| thinking_level_wire(l).to_string())
        .unwrap_or_else(|| "default".into());
    let text = format!(
        "verbosity: {verbosity:?}\nthinking: {thinking}\nstreaming: {:?}",
        cfg.streaming.transport
    );
    telegram::send_message(deps, chat_id, &text, None, None).await?;
    Ok(())
}

async fn thinking_command(
    deps: &Deps,
    chat_id: i64,
    arg: Option<&str>,
    cfg: &WorkerConfig,
) -> Result<(), IIIError> {
    let Some(arg) = arg else {
        let current = preferences::effective_thinking_level(deps, chat_id, cfg)
            .await
            .map(|l| thinking_level_wire(l).to_string())
            .unwrap_or_else(|| "default (off)".into());
        telegram::send_message(
            deps,
            chat_id,
            &format!(
                "Thinking level: {current}. Usage: /thinking off|minimal|low|medium|high|xhigh"
            ),
            None,
            None,
        )
        .await?;
        return Ok(());
    };

    let level = parse_thinking_level(arg);
    if level.is_none() && arg.to_lowercase() != "off" && arg.to_lowercase() != "none" {
        telegram::send_message(
            deps,
            chat_id,
            "Invalid level. Use: off, minimal, low, medium, high, xhigh",
            None,
            None,
        )
        .await?;
        return Ok(());
    }

    kv::set_chat_thinking_level(deps, chat_id, level).await?;
    let label = level.map(thinking_level_wire).unwrap_or("off");
    telegram::send_message(
        deps,
        chat_id,
        &format!("Thinking level set to {label}."),
        None,
        None,
    )
    .await?;
    Ok(())
}

async fn verbosity_command(
    deps: &Deps,
    chat_id: i64,
    arg: Option<&str>,
    cfg: &WorkerConfig,
) -> Result<(), IIIError> {
    let Some(arg) = arg else {
        let current = preferences::effective_verbosity(deps, chat_id, cfg).await;
        telegram::send_message(
            deps,
            chat_id,
            &format!("Verbosity: {current:?}. Usage: /verbosity none|minimal|high|debug"),
            None,
            None,
        )
        .await?;
        return Ok(());
    };

    let Some(v) = parse_verbosity(arg) else {
        telegram::send_message(
            deps,
            chat_id,
            "Invalid verbosity. Use: none, minimal, high, debug",
            None,
            None,
        )
        .await?;
        return Ok(());
    };

    kv::set_chat_verbosity(deps, chat_id, v).await?;
    telegram::send_message(
        deps,
        chat_id,
        &format!("Verbosity set to {v:?}."),
        None,
        None,
    )
    .await?;
    Ok(())
}

async fn start_flow(
    deps: &Deps,
    chat_id: i64,
    payload: Option<&str>,
    cfg: &WorkerConfig,
) -> Result<(), IIIError> {
    if let Some(p) = payload {
        if !p.is_empty() {
            tracing::info!(chat_id, payload = p, "deep link /start payload");
        }
    }

    reset_chat_for_start(deps, chat_id, cfg).await?;

    if let Some(model) = cfg.default_model.clone() {
        bind_model(deps, chat_id, &model).await?;
        telegram::send_message(
            deps,
            chat_id,
            &format!("Ready. Model: {} ({})", model.id, model.provider),
            None,
            None,
        )
        .await?;
        return Ok(());
    }
    show_model_picker(deps, chat_id, cfg).await
}

async fn reset_chat_for_start(
    deps: &Deps,
    chat_id: i64,
    cfg: &WorkerConfig,
) -> Result<(), IIIError> {
    let old_session = kv::chat_session(deps, chat_id).await;
    if let Some(ref sid) = old_session {
        let _ = harness::stop(&deps.iii, sid, cfg.timeout_ms).await;
    }
    deps.runtime.reset_for_chat(chat_id, old_session.as_deref());
    kv::clear_chat_session(deps, chat_id).await?;
    kv::clear_chat_model(deps, chat_id).await?;
    Ok(())
}

async fn show_model_picker(deps: &Deps, chat_id: i64, cfg: &WorkerConfig) -> Result<(), IIIError> {
    if cfg.default_model.is_some() {
        telegram::send_message(
            deps,
            chat_id,
            "Model is fixed by configuration.",
            None,
            None,
        )
        .await?;
        return Ok(());
    }

    let models = router::list_models(&deps.iii, cfg.timeout_ms).await?;
    if models.is_empty() {
        telegram::send_message(deps, chat_id, "No models available.", None, None).await?;
        return Ok(());
    }

    let mut rows = Vec::new();
    for model in models.iter().take(12) {
        let mut label = model
            .display_name
            .clone()
            .unwrap_or_else(|| model.id.clone());
        if model.supports_thinking == Some(true) {
            label.push_str(" 🧠");
        }
        rows.push(vec![(
            format!("{label} ({})", model.provider),
            format!("m:{}:{}", model.provider, model.id),
        )]);
    }

    kv::set_fsm(deps, chat_id, ChatFsm::AwaitingModel).await?;
    telegram::send_message(
        deps,
        chat_id,
        "Choose a model:",
        Some(telegram::inline_keyboard(rows)),
        None,
    )
    .await?;
    Ok(())
}

async fn handle_callback(
    deps: &Deps,
    cb: crate::types::TelegramCallbackQuery,
) -> Result<(), IIIError> {
    let cfg = deps.cfg().await;
    let data = cb.data.unwrap_or_default();
    let chat_id = cb
        .message
        .as_ref()
        .map(|m| m.chat.id)
        .or_else(|| cb.from.as_ref().map(|u| u.id))
        .unwrap_or(0);

    if data.starts_with("m:") {
        let parts: Vec<&str> = data.splitn(3, ':').collect();
        if parts.len() == 3 {
            let model = ModelRef {
                provider: parts[1].into(),
                id: parts[2].into(),
            };
            bind_model(deps, chat_id, &model).await?;
            telegram::answer_callback_query(deps, &cb.id, Some("Model selected")).await?;
            telegram::send_message(
                deps,
                chat_id,
                &format!("Model set to {} ({})", model.id, model.provider),
                None,
                None,
            )
            .await?;
        }
        return Ok(());
    }

    if let Some(token) = data.strip_prefix("a:") {
        resolve_callback(deps, token, ResolveDecision::Allow, false, &cfg).await?;
        telegram::answer_callback_query(deps, &cb.id, Some("Approved")).await?;
        return Ok(());
    }
    if let Some(token) = data.strip_prefix("d:") {
        resolve_callback(deps, token, ResolveDecision::Deny, false, &cfg).await?;
        telegram::answer_callback_query(deps, &cb.id, Some("Rejected")).await?;
        return Ok(());
    }
    if let Some(token) = data.strip_prefix("w:") {
        resolve_callback(deps, token, ResolveDecision::Allow, true, &cfg).await?;
        telegram::answer_callback_query(deps, &cb.id, Some("Approved always")).await?;
        return Ok(());
    }

    telegram::answer_callback_query(deps, &cb.id, None).await?;
    Ok(())
}

async fn resolve_callback(
    deps: &Deps,
    token: &str,
    decision: ResolveDecision,
    approve_always: bool,
    cfg: &WorkerConfig,
) -> Result<(), IIIError> {
    let Some(data) = kv::load_approval_callback(deps, token).await else {
        return Ok(());
    };
    if approve_always {
        approval::approve_always(
            &deps.iii,
            &data.session_id,
            &data.function_id,
            cfg.timeout_ms,
        )
        .await?;
    }
    let reason = if decision == ResolveDecision::Deny {
        Some("rejected via telegram".into())
    } else {
        None
    };
    approval::resolve(
        &deps.iii,
        approval::ResolveRequest {
            session_id: data.session_id,
            function_call_id: data.function_call_id,
            decision,
            reason,
        },
        cfg.timeout_ms,
    )
    .await?;
    kv::delete_approval_callback(deps, token).await;
    Ok(())
}

pub async fn send_user_message(
    deps: &Deps,
    chat_id: i64,
    session_id: Option<&str>,
    text: &str,
    idempotency_key: Option<i64>,
    cfg: &WorkerConfig,
    model: &ModelRef,
) -> Result<(), IIIError> {
    let thinking_level = preferences::effective_thinking_level(deps, chat_id, cfg)
        .await
        .map(|l| thinking_level_wire(l).to_string());

    let message_id = idempotency_key
        .map(telemetry::telegram_message_id)
        .unwrap_or_else(|| format!("tg-fifo-{chat_id}"));

    let session_id_for_trace = session_id
        .map(String::from)
        .unwrap_or_else(|| telemetry::pending_session_id(chat_id));

    let metadata = telemetry::tracing_metadata(&session_id_for_trace, &message_id);

    let options = harness::SendOptions {
        system_prompt: cfg.system_prompt.clone(),
        mode: None,
        thinking_level,
        functions: if cfg.functions_allow.is_empty() {
            None
        } else {
            Some(harness::FunctionsPolicy {
                allow: cfg.functions_allow.clone(),
            })
        },
        metadata: Some(metadata),
    };

    let resp = telemetry::with_baggage(&session_id_for_trace, &message_id, || async {
        harness::send(
            &deps.iii,
            harness::HarnessSendRequest {
                session_id: session_id.map(String::from),
                message: text.to_string(),
                model: model.id.clone(),
                provider: model.provider.clone(),
                idempotency_key: idempotency_key.map(|id| id.to_string()),
                session: Some(harness::SessionSeed {
                    title: Some(format!("Telegram {chat_id}")),
                    metadata: Some(harness::metadata_for_chat(chat_id, model)),
                }),
                options: Some(options),
            },
            cfg.timeout_ms,
        )
        .await
    })
    .await?;

    deps.runtime
        .active_turns
        .insert(resp.session_id.clone(), resp.turn_id.clone());

    if session_id.is_none() {
        kv::set_chat_session(deps, chat_id, &resp.session_id).await?;
    }

    Ok(())
}

async fn bind_model(deps: &Deps, chat_id: i64, model: &ModelRef) -> Result<(), IIIError> {
    kv::set_chat_model(deps, chat_id, model).await?;
    kv::set_fsm(deps, chat_id, ChatFsm::Idle).await?;
    Ok(())
}

async fn catch_up_approvals(deps: &Deps, session_id: &str, cfg: &WorkerConfig) {
    let Ok(pending) = approval::list_pending(&deps.iii, session_id, cfg.timeout_ms).await else {
        return;
    };
    for record in pending {
        if kv::approval_message_id(deps, &record.session_id, &record.function_call_id)
            .await
            .is_some()
        {
            continue;
        }
        let _ = dispatch_pending_created(deps, record).await;
    }
}

async fn dispatch_pending_created(
    deps: &Deps,
    record: PendingApprovalRecord,
) -> Result<BindingAck, IIIError> {
    super::bindings::pending_created::handle_internal(deps, record).await
}

fn header_matches(headers: &Option<serde_json::Map<String, Value>>, secret: &str) -> bool {
    let Some(map) = headers else {
        return false;
    };
    const TARGET: &str = "x-telegram-bot-api-secret-token";
    map.iter()
        .any(|(key, value)| key.eq_ignore_ascii_case(TARGET) && value.as_str() == Some(secret))
}

#[cfg(test)]
mod tests {
    #[test]
    fn parses_model_callback() {
        let data = "m:anthropic:claude-sonnet-4";
        let parts: Vec<&str> = data.splitn(3, ':').collect();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[1], "anthropic");
    }
}

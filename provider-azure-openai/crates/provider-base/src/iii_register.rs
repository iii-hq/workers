//! Shared `register_with_iii` plumbing for provider crates.
//!
//! Each provider crate exposes
//! `pub async fn register_with_iii(iii: &iii_sdk::III) -> anyhow::Result<()>`
//! which publishes `provider::<name>::complete` on the iii bus. The handler
//! decodes a JSON payload of the shape
//! `{ model, system_prompt, messages, tools }`, resolves the provider's
//! credential via `auth::get_token`, builds the per-provider `Config` from
//! the supplied builder closure, calls into the crate's own
//! `pub async fn stream(...)`, drains the resulting
//! [`ReceiverStream<AssistantMessageEvent>`] into a final
//! [`AssistantMessage`], and returns the message as JSON.
//!
//! `iii-sdk` 0.11 returns a single `Value` from a registered function — there
//! is no per-call streaming response surface — so the contract is to collect
//! the event sequence into the terminal `AssistantMessage`. Callers that
//! want incremental events subscribe to `agent::events/<sid>` separately.

use std::future::Future;
use std::sync::Arc;

use auth_credentials::Credential;
use harness_types::{
    AgentMessage, AgentTool, AssistantMessage, AssistantMessageEvent, ContentBlock, ErrorKind,
    StopReason, TextContent,
};
use iii_sdk::{FunctionRef, IIIError, RegisterFunctionMessage, III};
use serde_json::Value;
use tokio_stream::wrappers::ReceiverStream;
use tokio_stream::StreamExt;

async fn collect_final(
    stream: &mut ReceiverStream<AssistantMessageEvent>,
    provider: &str,
    model: &str,
) -> AssistantMessage {
    let mut last: Option<AssistantMessage> = None;
    while let Some(ev) = stream.next().await {
        match ev {
            AssistantMessageEvent::Done { message } => return message,
            AssistantMessageEvent::Error { error } => return error,
            AssistantMessageEvent::Start { partial }
            | AssistantMessageEvent::TextStart { partial }
            | AssistantMessageEvent::TextDelta { partial, .. }
            | AssistantMessageEvent::TextEnd { partial }
            | AssistantMessageEvent::ToolcallStart { partial }
            | AssistantMessageEvent::ToolcallDelta { partial, .. }
            | AssistantMessageEvent::ToolcallEnd { partial }
            | AssistantMessageEvent::ThinkingStart { partial }
            | AssistantMessageEvent::ThinkingDelta { partial, .. }
            | AssistantMessageEvent::ThinkingEnd { partial } => {
                last = Some(partial);
            }
            _ => {}
        }
    }
    last.unwrap_or_else(|| AssistantMessage {
        content: vec![ContentBlock::Text(TextContent {
            text: "stream closed without final".into(),
        })],
        stop_reason: StopReason::Error,
        error_message: Some("stream closed without final".into()),
        error_kind: Some(ErrorKind::Transient),
        usage: None,
        model: model.into(),
        provider: provider.into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    })
}

/// Register `provider::<name>::complete` as an iii function backed by
/// `stream_fn`. The handler issues an `auth::get_token` iii trigger to
/// resolve credentials before constructing the per-provider config —
/// providers no longer read env vars directly.
///
/// `build_config` is the per-provider `Config::with_credential(model,
/// &Credential)` adapter, called once per invocation. It receives the
/// [`Credential`] enum verbatim so providers that branch on type
/// (Anthropic: `x-api-key` for API-key, `Authorization: Bearer` for OAuth)
/// can choose their header convention.
///
/// When `auth::get_token` returns `null` the handler emits a synthetic
/// error `AssistantMessage` describing the missing credential and never
/// invokes the builder.
pub fn register_provider_complete<C, B, BErr, F, Fut>(
    iii: &III,
    provider_name: &str,
    build_config: B,
    stream_fn: F,
) -> FunctionRef
where
    C: Send + Sync + 'static,
    B: Fn(&str, &Credential) -> Result<C, BErr> + Copy + Send + Sync + 'static,
    BErr: std::fmt::Display + Send + Sync + 'static,
    F: Fn(Arc<C>, String, Vec<AgentMessage>, Vec<AgentTool>) -> Fut + Copy + Send + Sync + 'static,
    Fut: Future<Output = ReceiverStream<AssistantMessageEvent>> + Send + 'static,
{
    register_provider_with_id(
        iii,
        provider_name,
        format!("provider::{provider_name}::complete"),
        build_config,
        stream_fn,
    )
}

fn register_provider_with_id<C, B, BErr, F, Fut>(
    iii: &III,
    provider_name: &str,
    function_id: String,
    build_config: B,
    stream_fn: F,
) -> FunctionRef
where
    C: Send + Sync + 'static,
    B: Fn(&str, &Credential) -> Result<C, BErr> + Copy + Send + Sync + 'static,
    BErr: std::fmt::Display + Send + Sync + 'static,
    F: Fn(Arc<C>, String, Vec<AgentMessage>, Vec<AgentTool>) -> Fut + Copy + Send + Sync + 'static,
    Fut: Future<Output = ReceiverStream<AssistantMessageEvent>> + Send + 'static,
{
    let description = format!(
        "Stream a response from the {provider_name} provider (P5: complete contract; resolves credentials via auth::get_token)"
    );
    let provider_label = provider_name.to_string();
    let iii_clone = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(function_id).with_description(description),
        move |payload: Value| {
            let provider_label = provider_label.clone();
            let iii = iii_clone.clone();
            async move {
                let model = payload
                    .get("model")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing required field: model".into()))?
                    .to_string();
                let system_prompt = payload
                    .get("system_prompt")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let messages: Vec<AgentMessage> = payload
                    .get("messages")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|e| IIIError::Handler(format!("invalid messages: {e}")))?
                    .unwrap_or_default();
                let tools: Vec<AgentTool> = payload
                    .get("tools")
                    .cloned()
                    .map(serde_json::from_value)
                    .transpose()
                    .map_err(|e| IIIError::Handler(format!("invalid tools: {e}")))?
                    .unwrap_or_default();

                let cred = match crate::auth::fetch_credential(&iii, &provider_label).await {
                    Ok(Some(c)) => c,
                    Ok(None) => {
                        let msg = error_no_credential(&provider_label, &model);
                        return serde_json::to_value(msg)
                            .map_err(|e| IIIError::Handler(e.to_string()));
                    }
                    Err(e) => {
                        let msg = error_auth_lookup(&provider_label, &model, &e.to_string());
                        return serde_json::to_value(msg)
                            .map_err(|e| IIIError::Handler(e.to_string()));
                    }
                };

                let cfg = match build_config(&model, &cred) {
                    Ok(c) => c,
                    Err(e) => {
                        let msg = error_config_build(&provider_label, &model, &e.to_string());
                        return serde_json::to_value(msg)
                            .map_err(|e| IIIError::Handler(e.to_string()));
                    }
                };

                let mut s = stream_fn(Arc::new(cfg), system_prompt, messages, tools).await;
                let final_msg = collect_final(&mut s, &provider_label, &model).await;
                serde_json::to_value(final_msg).map_err(|e| IIIError::Handler(e.to_string()))
            }
        },
    ))
}

fn error_no_credential(provider: &str, model: &str) -> AssistantMessage {
    let text = format!(
        "no credential configured for provider '{provider}': set it with auth::set_token or export the corresponding env var"
    );
    // `harness_types::ErrorKind` has no dedicated `Auth` variant. `AuthExpired`
    // is the closest semantic match for "no credential present" — clients
    // already use it as a re-login signal.
    synthetic_error(provider, model, text, ErrorKind::AuthExpired)
}

fn error_auth_lookup(provider: &str, model: &str, detail: &str) -> AssistantMessage {
    let text = format!("auth::get_token failed for '{provider}': {detail}");
    // Bus-call failures are typically retryable (timeout, transient lookup).
    synthetic_error(provider, model, text, ErrorKind::Transient)
}

fn error_config_build(provider: &str, model: &str, detail: &str) -> AssistantMessage {
    let text = format!("provider '{provider}' config build failed: {detail}");
    // Config-build errors generally indicate misconfiguration and won't
    // succeed on retry.
    synthetic_error(provider, model, text, ErrorKind::Permanent)
}

fn synthetic_error(provider: &str, model: &str, text: String, kind: ErrorKind) -> AssistantMessage {
    AssistantMessage {
        content: vec![ContentBlock::Text(TextContent { text: text.clone() })],
        stop_reason: StopReason::Error,
        error_message: Some(text),
        error_kind: Some(kind),
        usage: None,
        model: model.into(),
        provider: provider.into(),
        timestamp: chrono::Utc::now().timestamp_millis(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    #[allow(dead_code)]
    struct DummyConfig {
        model: String,
    }

    #[allow(dead_code)]
    async fn dummy_stream(
        cfg: Arc<DummyConfig>,
        _system_prompt: String,
        _messages: Vec<AgentMessage>,
        _tools: Vec<AgentTool>,
    ) -> ReceiverStream<AssistantMessageEvent> {
        let (tx, rx) = mpsc::channel(4);
        tokio::spawn(async move {
            let final_msg = AssistantMessage {
                content: vec![ContentBlock::Text(TextContent { text: "ok".into() })],
                stop_reason: StopReason::End,
                error_message: None,
                error_kind: None,
                usage: None,
                model: cfg.model.clone(),
                provider: "dummy".into(),
                timestamp: 0,
            };
            let _ = tx
                .send(AssistantMessageEvent::Done { message: final_msg })
                .await;
        });
        ReceiverStream::new(rx)
    }

    #[allow(dead_code)]
    fn dummy_with_credential(
        model: &str,
        _cred: &auth_credentials::Credential,
    ) -> Result<DummyConfig, std::env::VarError> {
        Ok(DummyConfig {
            model: model.to_string(),
        })
    }

    /// Compile-time guard that `register_provider_complete` accepts a
    /// `(with_credential, stream)` pair shaped like every real provider
    /// crate. End-to-end behaviour against a live engine is covered in
    /// `workers/replay-test/tests/p5_provider_contract.rs`.
    #[allow(dead_code)]
    fn _bounds_witness_complete(iii: &III) {
        let _ = || {
            register_provider_complete::<DummyConfig, _, _, _, _>(
                iii,
                "dummy",
                dummy_with_credential,
                dummy_stream,
            )
        };
    }
}

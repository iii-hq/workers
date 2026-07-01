//! The `provider::openai-codex::stream` iii function: resolve a fresh OAuth
//! token from the vault, build a Responses request, and relay
//! AssistantMessageEvent frames into the router-owned channel. Login + refresh
//! live in the oauth-openai-codex worker / auth-credentials vault — this
//! provider only *triggers* a refresh when the token is near expiry.
use crate::config::build_config;
use crate::reasoning::{is_reasoning_model, reasoning_effort_for};
use crate::request::{build_body, build_headers, BodyArgs};
use crate::sse::synthetic_error_event;
use crate::upstream::{spawn_upstream, UpstreamArgs};
use crate::{auth, router_client, state, PROVIDER_ID};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::channels::open_sink;
use llm_router::chat::relay::FrameSink;
use llm_router::types::events::{AssistantMessageEvent, ErrorKind};
use llm_router::types::router::{
    CredentialSource, ProviderResolveResponse, ProviderStreamInput, ProviderStreamOutput,
};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

/// Heartbeat cadence while the upstream is silent (spec: at least every 30s).
pub const PING_INTERVAL: Duration = Duration::from_secs(30);

/// Refresh proactively when the token expires within this margin.
const EXPIRY_MARGIN_SECS: i64 = 60;

pub fn make_stream(
    iii: IIIClient,
    http: reqwest::Client,
) -> impl Fn(ProviderStreamInput) -> BoxFuture<'static, Result<ProviderStreamOutput, Error>>
       + Send
       + Sync
       + 'static {
    move |input: ProviderStreamInput| {
        let (iii, http) = (iii.clone(), http.clone());
        Box::pin(async move {
            let sink = open_sink(&iii, &input.writer_ref).await?;
            run_stream_call(&iii, http, input, sink.as_ref()).await;
            sink.close();
            Ok(ProviderStreamOutput { ok: true })
        })
    }
}

fn send_event(sink: &dyn FrameSink, ev: &AssistantMessageEvent) -> Result<(), ()> {
    let frame = serde_json::to_string(ev).expect("serializable event");
    sink.send(&frame).map_err(|_| ())
}

fn default_resolve() -> ProviderResolveResponse {
    ProviderResolveResponse {
        configured: false,
        source: CredentialSource::None,
        credential: None,
        api_url: None,
        max_tokens: None,
    }
}

/// Token expiry (seconds) from the credential's `expires_at`, else its JWT.
fn credential_expires_at(cred: &Value) -> Option<i64> {
    cred.get("expires_at").and_then(Value::as_i64).or_else(|| {
        cred.get("access_token")
            .and_then(Value::as_str)
            .and_then(auth::expires_at_from_access_token)
    })
}

fn near_expiry(cred: &Value) -> bool {
    match credential_expires_at(cred) {
        Some(exp) => exp <= crate::now_ms() / 1000 + EXPIRY_MARGIN_SECS,
        None => false, // unknown expiry: let the vault decide, don't force
    }
}

/// Fetch a usable credential.
///
/// 1. The `auth-credentials` vault is the authority when present; a near-expiry
///    token triggers the vault-owned refresh (the provider never calls the
///    OAuth endpoints itself).
/// 2. DEV FALLBACK: when no vault credential is available, read
///    `~/.codex/auth.json` directly (local-dev only — no vault to own refresh,
///    so the `codex` CLI keeps that file fresh). Gated on the vault being
///    absent/empty, so it never overrides a real vault credential.
async fn fetch_fresh_credential(iii: &IIIClient) -> Option<Value> {
    if let Some(cred) = router_client::get_token(iii, PROVIDER_ID)
        .await
        .ok()
        .flatten()
    {
        if near_expiry(&cred) {
            let _ = router_client::refresh(iii, PROVIDER_ID).await; // vault-owned
            return router_client::get_token(iii, PROVIDER_ID)
                .await
                .ok()
                .flatten()
                .or(Some(cred));
        }
        return Some(cred);
    }
    if let Some(cred) = auth::read_codex_home_credential() {
        eprintln!(
            "[provider-openai-codex] no auth-credentials vault — using local ~/.codex/auth.json (dev fallback)"
        );
        return Some(cred);
    }
    None
}

async fn run_stream_call(
    iii: &IIIClient,
    http: reqwest::Client,
    input: ProviderStreamInput,
    sink: &dyn FrameSink,
) {
    let model = input.model.clone(); // router id (e.g. codex/gpt-5.5)
    let mut warnings = Vec::new();

    let token = state::load_token(iii).await;
    // Effective settings from the router; tolerate a missing router (defaults).
    let resolved = router_client::resolve(iii, token.as_deref())
        .await
        .unwrap_or_else(|_| default_resolve());

    let credential = fetch_fresh_credential(iii).await;
    let cfg = match build_config(
        &model,
        input.max_output_tokens,
        &resolved,
        credential.as_ref(),
    ) {
        Ok(c) => c,
        Err(e) => {
            let _ = send_event(
                sink,
                &synthetic_error_event(&e.to_string(), &model, ErrorKind::Permanent),
            );
            return;
        }
    };

    // model_meta is a hint; the catalog is authoritative, curated as last resort.
    let model_meta = match input.model_meta {
        Some(m) => Some(m),
        None => router_client::models_get(iii, &model).await,
    };
    let reasoning_effort = if is_reasoning_model(
        &cfg.model,
        model_meta.as_ref().and_then(|m| m.supports_thinking),
    ) {
        let effort = reasoning_effort_for(input.thinking_level, &cfg.model);
        if input.thinking_level.is_some() && effort.is_none() {
            warnings.push(format!(
                "thinking_level ignored: {} does not accept reasoning effort",
                cfg.model
            ));
        }
        effort
    } else {
        if input.thinking_level.is_some() {
            warnings.push(format!(
                "thinking_level ignored: {} is not a reasoning model",
                cfg.model
            ));
        }
        None
    };

    let body = build_body(&BodyArgs {
        model: cfg.model.clone(),
        max_tokens: cfg.max_tokens,
        system_prompt: input.system_prompt.unwrap_or_default(),
        messages: input.messages,
        tools: input.tools.unwrap_or_default(),
        reasoning_effort,
    });
    let headers = build_headers(&cfg);

    let rx = spawn_upstream(
        http,
        UpstreamArgs {
            api_url: cfg.api_url.clone(),
            model,
            body,
            headers,
            warnings,
        },
    );
    pump(rx, sink, PING_INTERVAL).await;
}

/// Forward upstream events to the sink; ping through silence; stop on the
/// terminal event or a failed write (caller gone → dropping `rx` aborts the
/// upstream task and its in-flight HTTP request).
pub async fn pump(
    mut rx: mpsc::Receiver<AssistantMessageEvent>,
    sink: &dyn FrameSink,
    ping_interval: Duration,
) {
    loop {
        match tokio::time::timeout(ping_interval, rx.recv()).await {
            Ok(Some(ev)) => {
                let terminal = ev.is_terminal();
                if send_event(sink, &ev).is_err() {
                    return;
                }
                if terminal {
                    return;
                }
            }
            Ok(None) => return,
            Err(_elapsed) => {
                if send_event(sink, &AssistantMessageEvent::Ping).is_err() {
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sse::empty_assistant;
    use llm_router::chat::relay::RelayRead;
    use llm_router::testkit::fake_channels::FakeChannel;
    use serde_json::Value;

    fn done_event() -> AssistantMessageEvent {
        AssistantMessageEvent::Done {
            message: empty_assistant("codex/gpt-5.5"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn forwards_events_and_stops_at_terminal() {
        let ch = FakeChannel::new();
        let (tx, rx) = mpsc::channel(8);
        tx.send(AssistantMessageEvent::Start {
            partial: empty_assistant("m"),
        })
        .await
        .unwrap();
        tx.send(done_event()).await.unwrap();
        tx.send(AssistantMessageEvent::Ping).await.unwrap();
        drop(tx);

        pump(rx, &ch.writer, Duration::from_secs(30)).await;
        ch.writer.close();

        let mut frames = Vec::new();
        let mut reader = ch.reader;
        while let llm_router::chat::relay::ReadEvent::Msg(m) =
            reader.next(Duration::from_millis(100)).await
        {
            frames.push(m);
        }
        assert_eq!(frames.len(), 2);
        let last: Value = serde_json::from_str(&frames[1]).unwrap();
        assert_eq!(last["type"], "done");
    }

    #[test]
    fn near_expiry_uses_margin() {
        let past = serde_json::json!({ "access_token": "a", "expires_at": 1 });
        assert!(near_expiry(&past));
        let far =
            serde_json::json!({ "access_token": "a", "expires_at": crate::now_ms() / 1000 + 3600 });
        assert!(!near_expiry(&far));
        let unknown = serde_json::json!({ "access_token": "no-exp" });
        assert!(!near_expiry(&unknown));
    }
}

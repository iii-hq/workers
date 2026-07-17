//! The `provider::openai-codex::stream` iii function: resolve a fresh OAuth
//! token from the vault, build a Responses request, and relay
//! AssistantMessageEvent frames into the router-owned channel. Login + refresh
//! live in the oauth-openai-codex worker / auth-credentials vault — this
//! provider only *triggers* a refresh when the token is near expiry.
use crate::config::build_config;
use crate::reasoning::{is_reasoning_model, native_reasoning_effort, reasoning_effort_for};
use crate::request::{build_body, build_headers, BodyArgs};
use crate::sse::synthetic_error_event;
use crate::upstream::{spawn_upstream, UpstreamArgs};
use crate::{auth, router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::channels::open_sink;
use llm_router::chat::relay::FrameSink;
use llm_router::provider_scaffold::pump::{pump, send_event, PING_INTERVAL};
use llm_router::types::events::ErrorKind;
use llm_router::types::router::{
    CredentialSource, ProviderResolveResponse, ProviderStreamInput, ProviderStreamOutput,
};

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

fn default_resolve() -> ProviderResolveResponse {
    ProviderResolveResponse {
        configured: false,
        source: CredentialSource::None,
        credential: None,
        api_url: None,
        max_tokens: None,
    }
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

    let credential = auth::fetch_fresh_credential(iii).await;
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

    // model_meta is a hint; the dynamically reconciled catalog is authoritative.
    let model_meta = match input.model_meta {
        Some(m) => Some(m),
        None => router_client::models_get(iii, &model).await,
    };
    let native_effort =
        match native_reasoning_effort(input.provider_options.as_ref(), model_meta.as_ref()) {
            Ok(effort) => effort,
            Err(message) => {
                let _ = send_event(
                    sink,
                    &synthetic_error_event(&message, &model, ErrorKind::Permanent),
                );
                return;
            }
        };
    let reasoning_effort = if native_effort.is_some() {
        native_effort
    } else if is_reasoning_model(
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
        effort.map(str::to_string)
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

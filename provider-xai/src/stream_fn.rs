//! The `provider::xai::stream` iii function (spec § Provider stream
//! contract): write AssistantMessageEvent frames as JSON text messages into
//! the router-owned channel, terminal done/error last, then close.
use crate::config::config_from_resolve;
use crate::configuration::ConfigCell;
use crate::errors::classify_bus_error;
use crate::reasoning::{is_reasoning_model, reasoning_effort_for};
use crate::request::{build_body, build_headers, BodyArgs};
use crate::responses::{
    build_responses_body, responses_url, spawn_responses_upstream, ResponsesUpstreamArgs,
};
use crate::sse::synthetic_error_event;
use crate::upstream::{spawn_upstream, UpstreamArgs};
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::channels::open_sink;
use llm_router::chat::relay::FrameSink;
use llm_router::provider_scaffold::aborts::{AbortGuard, StreamAborts};
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::provider_scaffold::pump::{pump, pump_abortable, send_event, PING_INTERVAL};
use llm_router::types::events::{AssistantMessageEvent, ErrorKind};
use llm_router::types::router::{ProviderStreamInput, ProviderStreamOutput};
use tokio::sync::{mpsc, watch};

pub fn make_stream(
    iii: IIIClient,
    http: reqwest::Client,
    cell: ConfigCell,
    cache: ScaffoldCache,
    aborts: StreamAborts,
) -> impl Fn(ProviderStreamInput) -> BoxFuture<'static, Result<ProviderStreamOutput, Error>>
       + Send
       + Sync
       + 'static {
    move |input: ProviderStreamInput| {
        let (iii, http, cell, cache, aborts) = (
            iii.clone(),
            http.clone(),
            cell.clone(),
            cache.clone(),
            aborts.clone(),
        );
        Box::pin(async move {
            // Register BEFORE the first await: an abort landing while the sink
            // opens must latch, not hit an unknown id. The RAII guard
            // deregisters on every exit — early returns and an executor
            // cancelling this future mid-await alike.
            let abort_reg = input
                .resolution_key
                .as_ref()
                .map(|rid| aborts.register(rid));
            let sink = open_sink(&iii, &input.writer_ref).await?;
            let wc = { cell.read().await.clone() };
            run_stream_call(
                &iii,
                http,
                &cache,
                abort_reg.as_ref(),
                &wc,
                input,
                sink.as_ref(),
            )
            .await;
            sink.close();
            // ProviderStreamOutput (spec § stream contract)
            Ok(ProviderStreamOutput { ok: true })
        })
    }
}

/// Pump the upstream, abortable via `provider::xai::abort` while it's live.
/// Takes a receiver cloned from the call's `AbortGuard`, whose Drop in
/// `make_stream` deregisters the entry. Returns the pump's terminal
/// `ErrorKind`.
async fn pump_stream(
    rx: mpsc::Receiver<AssistantMessageEvent>,
    sink: &dyn FrameSink,
    abort_rx: Option<watch::Receiver<bool>>,
) -> Option<ErrorKind> {
    match abort_rx {
        Some(abort_rx) => pump_abortable(rx, sink, PING_INTERVAL, abort_rx).await,
        None => pump(rx, sink, PING_INTERVAL).await,
    }
}

async fn run_stream_call(
    iii: &IIIClient,
    http: reqwest::Client,
    cache: &ScaffoldCache,
    abort_reg: Option<&AbortGuard>,
    wc: &crate::config::WorkerConfig,
    input: ProviderStreamInput,
    sink: &dyn FrameSink,
) {
    let model = input.model.clone();
    let mut warnings = Vec::new();

    // Token + resolve are cached (ScaffoldCache): zero engine round trips
    // on the hot path within the TTL. An auth-classified resolve failure
    // drops the cache so the next attempt re-resolves fresh — retrying
    // stays the router's job.
    let token = cache.load_token(iii, state::STATE_SCOPE).await;
    let resolved = match cache
        .resolve(
            iii,
            crate::PROVIDER_ID,
            token.as_deref(),
            Some(crate::register::CREDENTIAL_ENV_VAR),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            let kind = classify_bus_error(&e);
            if kind == ErrorKind::AuthExpired {
                cache.invalidate();
            }
            let _ = send_event(
                sink,
                &synthetic_error_event(
                    &format!("router::provider::resolve failed: {e}"),
                    &model,
                    kind,
                ),
            );
            return;
        }
    };
    let cfg = match config_from_resolve(&model, input.max_output_tokens, &resolved) {
        Ok(c) => c,
        Err(e) => {
            let _ = send_event(
                sink,
                &synthetic_error_event(&e.to_string(), &model, ErrorKind::Permanent),
            );
            return;
        }
    };

    // Agent Tools path (opt-in via the console config): when enabled, route to
    // the /v1/responses API with server-side tools (x_search / web_search) so
    // the model can pull live X / web data. Otherwise fall through to the plain
    // Chat Completions path below.
    if wc.tools_enabled && !wc.tool_sources.is_empty() {
        // Report-and-continue: the Responses path carries the conversation +
        // server-side tools, but not the chat-path per-request controls, so
        // name each dropped one instead of silently ignoring it.
        if input.tools.as_ref().is_some_and(|t| !t.is_empty()) {
            warnings.push(
                "client function tools are not sent on the xAI Agent Tools path; \
                 disable provider-xai tools to use function calling"
                    .to_string(),
            );
        }
        if input.response_format.is_some() {
            warnings.push("response_format is not applied on the xAI Agent Tools path".to_string());
        }
        if input.thinking_level.is_some() {
            warnings.push("thinking_level is not applied on the xAI Agent Tools path".to_string());
        }
        let system_prompt = input.system_prompt.clone().unwrap_or_default();
        let tool_types: Vec<String> = wc
            .tool_sources
            .iter()
            .map(|t| t.as_type().to_string())
            .collect();
        let body = build_responses_body(
            &cfg.model,
            &system_prompt,
            &input.messages,
            &tool_types,
            cfg.max_tokens,
        );
        let headers = build_headers(&cfg);
        // Aborted while we were setting up — never start the upstream request.
        if abort_reg.is_some_and(|g| g.is_fired()) {
            return;
        }
        let rx = spawn_responses_upstream(
            http,
            ResponsesUpstreamArgs {
                api_url: responses_url(&cfg.api_url),
                model,
                body,
                headers,
                warnings,
            },
        );
        // An upstream auth terminal means the cached credential was rotated
        // out from under us: drop the cache so the next attempt re-resolves.
        if pump_stream(rx, sink, abort_reg.map(|g| g.watch())).await == Some(ErrorKind::AuthExpired)
        {
            cache.invalidate();
        }
        return;
    }

    // model_meta is a hint, never source of truth (spec): absent → the
    // catalog is authoritative → curated snapshot as a last resort.
    let model_meta = match input.model_meta {
        Some(m) => Some(m),
        None => router_client::models_get(iii, &model).await,
    };
    let reasoning_effort = if is_reasoning_model(
        &model,
        model_meta.as_ref().and_then(|m| m.supports_thinking),
    ) {
        let effort = reasoning_effort_for(input.thinking_level, &model);
        if input.thinking_level.is_some() && effort.is_none() {
            // Report-and-continue: the family takes no effort param
            // (o1/chat-tuned) — the request still succeeds at the default.
            warnings.push(format!(
                "thinking_level ignored: {model} does not accept reasoning_effort"
            ));
        }
        effort
    } else {
        if input.thinking_level.is_some() {
            warnings.push(format!(
                "thinking_level ignored: {model} is not a reasoning model"
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
        response_format: input.response_format,
    });
    let headers = build_headers(&cfg);

    // Aborted while we were setting up — never start the upstream request.
    if abort_reg.is_some_and(|g| g.is_fired()) {
        return;
    }
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
    // An upstream auth terminal means the cached credential was rotated
    // out from under us: drop the cache so the next attempt re-resolves.
    if pump_stream(rx, sink, abort_reg.map(|g| g.watch())).await == Some(ErrorKind::AuthExpired) {
        cache.invalidate();
    }
}

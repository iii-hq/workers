//! Resolve the provider session, build a Responses request, and relay frames.
use crate::config::build_config;
use crate::errors::classify_bus_error;
use crate::reasoning::{is_reasoning_model, native_reasoning_effort, reasoning_effort_for};
use crate::request::{build_body, build_headers, resolve_cache_routing, BodyArgs};
use crate::session::AuthManager;
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
use llm_router::types::events::ErrorKind;
use llm_router::types::router::{
    CredentialSource, ProviderResolveResponse, ProviderStreamInput, ProviderStreamOutput,
};
use std::sync::Arc;

pub fn make_stream(
    iii: IIIClient,
    http: reqwest::Client,
    cache: ScaffoldCache,
    aborts: StreamAborts,
    auth: Arc<AuthManager>,
) -> impl Fn(ProviderStreamInput) -> BoxFuture<'static, Result<ProviderStreamOutput, Error>>
       + Send
       + Sync
       + 'static {
    move |input: ProviderStreamInput| {
        let (iii, http, cache, aborts) = (iii.clone(), http.clone(), cache.clone(), aborts.clone());
        let auth = auth.clone();
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
            run_stream_call(
                &iii,
                http,
                &cache,
                abort_reg.as_ref(),
                input,
                sink.as_ref(),
                auth,
            )
            .await;
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
    cache: &ScaffoldCache,
    abort_reg: Option<&AbortGuard>,
    input: ProviderStreamInput,
    sink: &dyn FrameSink,
    auth: Arc<AuthManager>,
) {
    let model = input.model.clone(); // router id (e.g. codex/gpt-5.5)
    let mut warnings = Vec::new();

    // Token + resolve are cached (ScaffoldCache): zero engine round trips on
    // the hot path within the TTL. Session resolution stays fresh and serializes
    // token rotation independently of the router registration cache.
    let token = cache.load_token(iii, state::STATE_SCOPE).await;
    // Effective settings from the router; tolerate a missing router (defaults).
    // An auth-classified failure drops the cache so the next attempt
    // re-resolves fresh — retrying stays the router's job.
    let resolved = match cache
        .resolve(iii, crate::PROVIDER_ID, token.as_deref(), None)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            if classify_bus_error(&e) == ErrorKind::AuthExpired {
                cache.invalidate();
            }
            default_resolve()
        }
    };

    let credential = match auth.resolve(None).await {
        Ok(credential) => credential,
        Err(e) => {
            let _ = send_event(
                sink,
                &synthetic_error_event(&e.message, &model, e.error_kind()),
            );
            return;
        }
    };
    let cfg = match build_config(
        &model,
        input.max_output_tokens,
        &resolved,
        credential.as_ref().map(|c| &c.value),
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
    let supports_thinking = is_reasoning_model(
        &cfg.model,
        model_meta.as_ref().and_then(|m| m.supports_thinking),
    );
    let reasoning_effort = if native_effort.is_some() {
        native_effort
    } else if supports_thinking {
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

    let system_prompt = input.system_prompt.unwrap_or_default();
    let (affinity_headers, prompt_cache_key) = resolve_cache_routing(
        input.provider_options.as_ref(),
        input.cache_intent.as_ref(),
        input.session_id.as_deref(),
        input.resolution_key.as_deref(),
    );
    tracing::debug!(
        affinity_configured = !affinity_headers.is_empty(),
        prompt_cache_key_configured = prompt_cache_key.is_some(),
        "codex cache affinity key"
    );
    let mut headers = build_headers(&cfg);
    headers.extend(affinity_headers);
    let body = build_body(&BodyArgs {
        model: cfg.model.clone(),
        max_tokens: cfg.max_tokens,
        system_prompt,
        messages: input.messages,
        tools: input.tools.unwrap_or_default(),
        supports_thinking,
        reasoning_effort,
        prompt_cache_key,
    });

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
        Some(auth),
    );
    let kind = match abort_reg {
        Some(g) => pump_abortable(rx, sink, PING_INTERVAL, g.watch()).await,
        None => pump(rx, sink, PING_INTERVAL).await,
    };
    // Invalidate router settings after an authentication terminal as well.
    if kind == Some(ErrorKind::AuthExpired) {
        cache.invalidate();
    }
}

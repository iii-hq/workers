//! The `provider::anthropic::stream` iii function (spec § Provider stream
//! contract): write AssistantMessageEvent frames as JSON text messages into
//! the router-owned channel, terminal done/error last, then close.
use crate::config::config_from_resolve;
use crate::errors::classify_bus_error;
use crate::request::{build_body, build_headers, BodyArgs};
use crate::sse::synthetic_error_event;
use crate::thinking::build_thinking_config;
use crate::upstream::{spawn_upstream, UpstreamArgs};
use crate::wire::cache::cache_enabled;
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
use llm_router::types::router::{ProviderStreamInput, ProviderStreamOutput};

pub fn make_stream(
    iii: IIIClient,
    http: reqwest::Client,
    cache: ScaffoldCache,
    aborts: StreamAborts,
) -> impl Fn(ProviderStreamInput) -> BoxFuture<'static, Result<ProviderStreamOutput, Error>>
       + Send
       + Sync
       + 'static {
    move |input: ProviderStreamInput| {
        let (iii, http, cache, aborts) = (iii.clone(), http.clone(), cache.clone(), aborts.clone());
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
            run_stream_call(&iii, http, &cache, abort_reg.as_ref(), input, sink.as_ref()).await;
            sink.close();
            // ProviderStreamOutput (spec § stream contract)
            Ok(ProviderStreamOutput { ok: true })
        })
    }
}

async fn run_stream_call(
    iii: &IIIClient,
    http: reqwest::Client,
    cache: &ScaffoldCache,
    abort_reg: Option<&AbortGuard>,
    input: ProviderStreamInput,
    sink: &dyn FrameSink,
) {
    let model = input.model.clone();

    let mut warnings = Vec::new();
    if input.response_format.is_some() {
        // Report-and-continue (spec § stream contract): no native structured
        // output on the Messages API; the router only gates *known* models.
        warnings.push(
            "response_format ignored: anthropic has no native structured-output mode".to_string(),
        );
    }

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

    // model_meta is a hint, never source of truth (spec): absent → the
    // catalog is authoritative. Adaptive thinking needs no budget data, so
    // a missing record costs nothing on the request path.
    let model_meta = match input.model_meta {
        Some(m) => Some(m),
        None => router_client::models_get(iii, &model).await,
    };
    let thinking_build = build_thinking_config(input.thinking_level, model_meta.as_ref());
    warnings.extend(thinking_build.warnings);

    // Defense in depth: never POST an empty messages array — Anthropic rejects
    // it with a 400 ("messages: at least one message is required"). Surface a
    // clear provider error frame instead of a cryptic upstream failure. The
    // harness/context-manager guards make this unreachable in practice.
    if input.messages.is_empty() {
        let _ = send_event(
            sink,
            &synthetic_error_event(
                "refusing to call anthropic with an empty messages array \
                 (messages: at least one message is required)",
                &model,
                ErrorKind::Permanent,
            ),
        );
        return;
    }

    let body = build_body(
        &BodyArgs {
            model: cfg.model.clone(),
            max_tokens: cfg.max_tokens,
            system_prompt: input.system_prompt.unwrap_or_default(),
            messages: input.messages,
            tools: input.tools.unwrap_or_default(),
            thinking: thinking_build.config,
            effort: thinking_build.effort,
            cache_enabled: cache_enabled(),
        },
        &mut warnings,
    );
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
    let kind = match abort_reg {
        Some(g) => pump_abortable(rx, sink, PING_INTERVAL, g.watch()).await,
        None => pump(rx, sink, PING_INTERVAL).await,
    };
    // An upstream auth terminal means the cached credential was rotated
    // out from under us: drop the cache so the next attempt re-resolves.
    if kind == Some(ErrorKind::AuthExpired) {
        cache.invalidate();
    }
}

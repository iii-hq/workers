//! The `provider::sarvam::stream` iii function (spec § Provider stream
//! contract): write AssistantMessageEvent frames as JSON text messages into
//! the router-owned channel, terminal done/error last, then close.
use crate::config::config_from_resolve;
use crate::errors::classify_bus_error;
use crate::reasoning::{is_reasoning_model, reasoning_effort_for};
use crate::request::{build_body, build_headers, BodyArgs};
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
            let abort_reg = input
                .resolution_key
                .as_ref()
                .map(|rid| aborts.register(rid));
            let sink = open_sink(&iii, &input.writer_ref).await?;
            run_stream_call(&iii, http, &cache, abort_reg.as_ref(), input, sink.as_ref()).await;
            sink.close();
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

    let model_meta = match input.model_meta {
        Some(m) => Some(m),
        None => router_client::models_get(iii, &model).await,
    };
    if input
        .response_format
        .as_ref()
        .is_some_and(|rf| rf.schema.is_some())
    {
        warnings.push(
            "response_format schema unsupported: Sarvam runs json_object mode without schema validation"
                .to_string(),
        );
    }

    let reasoning = is_reasoning_model(
        &model,
        model_meta.as_ref().and_then(|m| m.supports_thinking),
    );
    if input.thinking_level.is_some() && !reasoning {
        warnings.push(format!(
            "thinking_level ignored: {model} is not a reasoning model"
        ));
    }
    let reasoning_effort = if reasoning {
        reasoning_effort_for(input.thinking_level, &model)
    } else {
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
    if kind == Some(ErrorKind::AuthExpired) {
        cache.invalidate();
    }
}

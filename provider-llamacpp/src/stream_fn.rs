//! The `provider::llamacpp::stream` iii function (spec § Provider stream
//! contract): write AssistantMessageEvent frames as JSON text messages into
//! the router-owned channel, terminal done/error last, then close.
use crate::config::config_from_resolve;
use crate::errors::classify_bus_error;
use crate::request::{build_body, build_headers, BodyArgs};
use crate::sse::synthetic_error_event;
use crate::upstream::{spawn_upstream, UpstreamArgs};
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::channels::open_sink;
use llm_router::chat::relay::FrameSink;
use llm_router::provider_scaffold::pump::{pump, send_event, PING_INTERVAL};
use llm_router::types::events::ErrorKind;
use llm_router::types::router::{ProviderStreamInput, ProviderStreamOutput};

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
            // ProviderStreamOutput (spec § stream contract)
            Ok(ProviderStreamOutput { ok: true })
        })
    }
}

async fn run_stream_call(
    iii: &IIIClient,
    http: reqwest::Client,
    input: ProviderStreamInput,
    sink: &dyn FrameSink,
) {
    let model = input.model.clone();
    let mut warnings = Vec::new();

    let token = state::load_token(iii).await;
    let resolved = match router_client::resolve(iii, token.as_deref()).await {
        Ok(r) => r,
        Err(e) => {
            let _ = send_event(
                sink,
                &synthetic_error_event(
                    &format!("router::provider::resolve failed: {e}"),
                    &model,
                    classify_bus_error(&e),
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

    // Any requested level means "think"; absent means "off" (the enum has no
    // Off variant). Mapped onto the `enable_thinking` chat-template kwarg in
    // build_body — llama.cpp's only per-request reasoning lever.
    let enable_thinking = input.thinking_level.is_some();

    // Best-effort: the kwarg only bites if the model's chat template gates its
    // reasoning channel on it; otherwise reasoning stays whatever the server's
    // --reasoning-format / template dictates, so warn rather than promise.
    if enable_thinking {
        warnings.push(
            "thinking_level applied best-effort via chat_template_kwargs.enable_thinking; \
             effective only if the model's chat template gates reasoning on that key \
             (otherwise controlled by the server's --reasoning-format)"
                .to_string(),
        );
    }

    let body = build_body(&BodyArgs {
        model: cfg.model.clone(),
        max_tokens: cfg.max_tokens,
        system_prompt: input.system_prompt.unwrap_or_default(),
        messages: input.messages,
        tools: input.tools.unwrap_or_default(),
        response_format: input.response_format,
        enable_thinking,
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

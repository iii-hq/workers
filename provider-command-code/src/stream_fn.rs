use crate::config::config_from_resolve;
use crate::errors::classify_bus_error;
use crate::request::{build_request, RequestArgs};
use crate::sse::synthetic_error;
use crate::upstream::{spawn_upstream, UpstreamArgs};
use crate::{PROVIDER_ID, STATE_SCOPE};
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
            let abort = input
                .resolution_key
                .as_ref()
                .map(|request_id| aborts.register(request_id));
            let sink = open_sink(&iii, &input.writer_ref).await?;
            run_stream(&iii, http, &cache, abort.as_ref(), input, sink.as_ref()).await;
            sink.close();
            Ok(ProviderStreamOutput { ok: true })
        })
    }
}

async fn run_stream(
    iii: &IIIClient,
    http: reqwest::Client,
    cache: &ScaffoldCache,
    abort: Option<&AbortGuard>,
    input: ProviderStreamInput,
    sink: &dyn FrameSink,
) {
    let model = input.model.clone();
    if input.messages.is_empty() {
        let _ = send_event(
            sink,
            &synthetic_error(
                &model,
                "refusing to call Command Code with an empty messages array",
                ErrorKind::Permanent,
            ),
        );
        return;
    }

    let token = cache.load_token(iii, STATE_SCOPE).await;
    let resolved = match cache
        .resolve(
            iii,
            PROVIDER_ID,
            token.as_deref(),
            Some(crate::register::CREDENTIAL_ENV_VAR),
        )
        .await
    {
        Ok(resolved) => resolved,
        Err(error) => {
            let kind = classify_bus_error(&error);
            let _ = send_event(
                sink,
                &synthetic_error(
                    &model,
                    format!("router::provider::resolve failed: {error}"),
                    kind,
                ),
            );
            return;
        }
    };
    let config = match config_from_resolve(&model, input.max_output_tokens, &resolved) {
        Ok(config) => config,
        Err(error) => {
            let _ = send_event(
                sink,
                &synthetic_error(&model, error.to_string(), ErrorKind::Permanent),
            );
            return;
        }
    };

    let mut warnings = Vec::new();
    if input.thinking_level.is_some() {
        warnings.push(
            "thinking_level ignored: the Command Code catalog does not report a portable reasoning-control capability"
                .to_string(),
        );
    }
    let request = build_request(
        &config,
        &RequestArgs {
            system_prompt: input.system_prompt.unwrap_or_default(),
            messages: input.messages,
            tools: input.tools.unwrap_or_default(),
            response_format: input.response_format,
        },
        &mut warnings,
    );
    if abort.is_some_and(AbortGuard::is_fired) {
        return;
    }
    let receiver = spawn_upstream(
        http,
        UpstreamArgs {
            url: request.url,
            model,
            dialect: request.dialect,
            body: request.body,
            headers: request.headers,
            warnings,
        },
    );
    let kind = match abort {
        Some(abort) => pump_abortable(receiver, sink, PING_INTERVAL, abort.watch()).await,
        None => pump(receiver, sink, PING_INTERVAL).await,
    };
    if kind == Some(ErrorKind::AuthExpired) {
        cache.invalidate();
    }
}

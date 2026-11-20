//! The `provider::gemini::stream` iii function (spec § Provider stream
//! contract): write AssistantMessageEvent frames as JSON text messages into
//! the router-owned channel, terminal done/error last, then close.
use crate::config::{config_from_resolve, upstream_timeout};
use crate::errors::classify_bus_error;
use crate::reasoning::thinking_level_for;
use crate::request::{BodyArgs, build_body, build_headers};
use crate::sse::synthetic_error_event;
use crate::upstream::{HEARTBEAT, UpstreamArgs, spawn_upstream};
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::IIIClient;
use iii_sdk::errors::Error;
use llm_router::channels::open_sink;
use llm_router::chat::relay::FrameSink;
use llm_router::provider_scaffold::aborts::{AbortGuard, StreamAborts};
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::provider_scaffold::pump::{PING_INTERVAL, pump, pump_abortable, send_event};
use llm_router::types::events::ErrorKind;
use llm_router::types::model::{Model, ThinkingLevel};
use llm_router::types::router::{ProviderStreamInput, ProviderStreamOutput};

/// Whether to send `thinking_level` and summaries: the row decides, the id
/// pattern is the last resort.
fn thinks(meta: Option<&Model>, model: &str) -> bool {
    meta.and_then(|m| m.supports_thinking)
        .unwrap_or_else(|| model.starts_with("gemini-"))
}

/// The wire `thinking_level` and whether to ask for summaries. A model that
/// does not think gets neither: unknown thinking config can be a 400.
fn thinking_wire(
    meta: Option<&Model>,
    model: &str,
    level: Option<ThinkingLevel>,
    warnings: &mut Vec<String>,
) -> (Option<&'static str>, bool) {
    if thinks(meta, model) {
        let (level, warning) = thinking_level_for(level, model);
        warnings.extend(warning);
        return (level, true);
    }
    if level.is_some() {
        warnings.push(format!(
            "thinking_level ignored: {model} is not a reasoning model"
        ));
    }
    (None, false)
}

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
    // catalog is authoritative → id-pattern fallback as a last resort.
    let model_meta = match input.model_meta {
        Some(m) => Some(m),
        None => router_client::models_get(iii, &model).await,
    };
    let (thinking_level, thinks) = thinking_wire(
        model_meta.as_ref(),
        &model,
        input.thinking_level,
        &mut warnings,
    );

    let body = match build_body(
        &BodyArgs {
            model: cfg.model.clone(),
            max_tokens: cfg.max_tokens,
            system_prompt: input.system_prompt.unwrap_or_default(),
            messages: input.messages,
            tools: input.tools.unwrap_or_default(),
            thinking_level,
            // Summaries show the thoughts in the console and keep bytes
            // flowing while the model thinks (no keepalives are sent).
            thinking_summaries: thinks,
            response_format: input.response_format,
        },
        &mut warnings,
    ) {
        Ok(body) => body,
        Err(e) => {
            let _ = send_event(
                sink,
                &synthetic_error_event(&e.to_string(), &model, ErrorKind::Permanent),
            );
            return;
        }
    };
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
            first_token_timeout: upstream_timeout(),
            heartbeat: HEARTBEAT,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::curated::enrich;

    #[test]
    fn the_row_gates_thinking_and_the_id_is_the_fallback() {
        let mut row = enrich("gemini-3.8-flash");
        assert!(thinks(Some(&row), "gemini-3.8-flash"));
        row.supports_thinking = Some(false);
        assert!(!thinks(Some(&row), "gemini-3.8-flash"));
        assert!(thinks(None, "gemini-3.8-flash"));
        assert!(!thinks(None, "gemma-4-31b-it"));
    }

    #[test]
    fn a_model_that_does_not_think_gets_no_thinking_config() {
        let mut row = enrich("gemini-3.8-flash");
        row.supports_thinking = Some(false);
        let mut warnings = Vec::new();
        let wire = thinking_wire(
            Some(&row),
            "gemini-3.8-flash",
            Some(ThinkingLevel::High),
            &mut warnings,
        );
        assert_eq!(wire, (None, false));
        assert_eq!(
            warnings,
            ["thinking_level ignored: gemini-3.8-flash is not a reasoning model"]
        );
        let mut warnings = Vec::new();
        let wire = thinking_wire(
            None,
            "gemini-3.8-flash",
            Some(ThinkingLevel::High),
            &mut warnings,
        );
        assert_eq!(wire, (Some("high"), true));
    }
}

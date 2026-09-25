//! The `provider::anthropic::stream` iii function (spec § Provider stream
//! contract): write AssistantMessageEvent frames as JSON text messages into
//! the router-owned channel, terminal done/error last, then close.
use crate::config::config_from_resolve;
use crate::errors::classify_bus_error;
use crate::request::{build_body, build_headers, BodyArgs};
use crate::sse::synthetic_error_event;
use crate::thinking::{build_thinking_config, prefix_mismatch};
use crate::upstream::{spawn_upstream, UpstreamArgs};
use crate::wire::cache::{cache_enabled, cache_ttl};
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
use serde_json::{json, Value};

const CAPTURE_DIR_ENV: &str = "PROVIDER_ANTHROPIC_CAPTURE_DIR";

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
            system_sections: input.system_sections,
            messages: input.messages,
            tools: input.tools.unwrap_or_default(),
            thinking: thinking_build.config,
            effort: thinking_build.effort,
            prefix_mismatch: prefix_mismatch(),
            cache_enabled: cache_enabled(),
            cache_ttl: cache_ttl(),
        },
        &mut warnings,
    );
    let headers = build_headers(&cfg, &body);

    // Aborted while we were setting up — never start the upstream request.
    if abort_reg.is_some_and(|g| g.is_fired()) {
        return;
    }
    capture(
        std::env::var(CAPTURE_DIR_ENV).ok(),
        input.session_id.as_deref(),
        &headers,
        &body,
    );
    let rx = spawn_upstream(
        http,
        UpstreamArgs {
            api_url: cfg.api_url.clone(),
            session_id: input.session_id,
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

/// `PROVIDER_ANTHROPIC_CAPTURE_DIR`: append each sent request, as
/// `{conversation_id, headers: {anthropic-beta}, request}`, to
/// `<dir>/<session id>.jsonl` for offline prefix diffing. Auth headers are
/// never written; requests without a session (the summarizer) are skipped.
// ponytail: blocking std::fs append on the request path; debug-only knob.
fn capture(
    dir: Option<String>,
    session_id: Option<&str>,
    headers: &[(&'static str, String)],
    body: &Value,
) {
    let (Some(dir), Some(sid)) = (dir, session_id) else {
        return;
    };
    // Caller-supplied: no separator survives, so the file stays inside `dir`.
    let file: String = sid
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || "-_.".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let beta: serde_json::Map<String, Value> = headers
        .iter()
        .filter(|(k, _)| *k == "anthropic-beta")
        .map(|(k, v)| (k.to_string(), json!(v)))
        .collect();
    let line = format!(
        "{}\n",
        json!({ "conversation_id": sid, "headers": beta, "request": body })
    );
    let path = std::path::Path::new(&dir).join(format!("{file}.jsonl"));
    let res = std::fs::create_dir_all(&dir)
        .and_then(|_| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
        })
        .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
    if let Err(e) = res {
        tracing::warn!(path = %path.display(), "request capture failed: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::THINKING_BINDING_BETA;

    #[test]
    fn capture_appends_wrapper_lines_without_credentials() {
        let dir = std::env::temp_dir().join(format!("pa-capture-{}", uuid::Uuid::new_v4()));
        let dir_s = dir.to_string_lossy().into_owned();
        let headers = [
            ("x-api-key", "sk-secret".to_string()),
            ("anthropic-beta", THINKING_BINDING_BETA.to_string()),
        ];
        let body = json!({ "model": "m", "messages": [{ "role": "user", "content": "hi" }] });
        for _ in 0..2 {
            capture(Some(dir_s.clone()), Some("s/../x"), &headers, &body);
        }
        // no session: nothing written (the summarizer path)
        capture(Some(dir_s.clone()), None, &headers, &body);

        let names: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["s_.._x.jsonl"]);
        let text = std::fs::read_to_string(dir.join("s_.._x.jsonl")).unwrap();
        assert!(!text.contains("sk-secret"), "{text}");
        let lines: Vec<Value> = text
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect();
        assert_eq!(lines.len(), 2);
        for line in &lines {
            assert_eq!(line["conversation_id"], "s/../x");
            assert_eq!(
                line["headers"],
                json!({ "anthropic-beta": THINKING_BINDING_BETA })
            );
            assert_eq!(line["request"], body);
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

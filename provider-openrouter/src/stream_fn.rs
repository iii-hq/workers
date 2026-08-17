//! The `provider::openrouter::stream` iii function (spec § Provider stream
//! contract): write AssistantMessageEvent frames as JSON text messages into
//! the router-owned channel, terminal done/error last, then close.
use crate::catalog::upstream_id;
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
use llm_router::provider_scaffold::aborts::{AbortGuard, StreamAborts};
use llm_router::provider_scaffold::pump::pump_abortable;
use llm_router::types::events::{AssistantMessageEvent, ErrorKind};
use llm_router::types::model::{Model, ThinkingLevel};
use llm_router::types::router::{ProviderStreamInput, ProviderStreamOutput};
use std::time::Duration;
use tokio::sync::mpsc;

/// Heartbeat cadence while the upstream is silent (spec: at least every 30s).
pub const PING_INTERVAL: Duration = Duration::from_secs(30);

pub fn make_stream(
    iii: IIIClient,
    http: reqwest::Client,
    aborts: StreamAborts,
) -> impl Fn(ProviderStreamInput) -> BoxFuture<'static, Result<ProviderStreamOutput, Error>>
       + Send
       + Sync
       + 'static {
    move |input: ProviderStreamInput| {
        let (iii, http, aborts) = (iii.clone(), http.clone(), aborts.clone());
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
            run_stream_call(&iii, http, abort_reg.as_ref(), input, sink.as_ref()).await;
            sink.close();
            // ProviderStreamOutput (spec § stream contract)
            Ok(ProviderStreamOutput { ok: true })
        })
    }
}

/// Resolve a requested thinking level against the model's advertised efforts
/// (OpenRouter's `reasoning.supported_efforts`, carried on the catalog
/// record). Preference ladders step DOWN, never up, so a request can only get
/// less reasoning than asked, not more. No advertised list → send the first
/// candidate as-is (OpenRouter drops parameters a model does not support).
pub fn resolve_reasoning_effort(level: ThinkingLevel, meta: Option<&Model>) -> Option<String> {
    let candidates: &[&str] = match level {
        ThinkingLevel::Minimal => &["minimal", "low"],
        ThinkingLevel::Low => &["low", "minimal"],
        ThinkingLevel::Medium => &["medium", "low"],
        ThinkingLevel::High => &["high", "medium"],
        ThinkingLevel::Xhigh => &["xhigh", "high", "medium"],
    };
    let advertised = meta.and_then(|m| m.reasoning_efforts.as_ref());
    match advertised {
        Some(efforts) => candidates
            .iter()
            .find(|c| efforts.iter().any(|e| e.effort == **c))
            .map(|c| c.to_string()),
        None => Some(candidates[0].to_string()),
    }
}

fn send_event(sink: &dyn FrameSink, ev: &AssistantMessageEvent) -> Result<(), ()> {
    let frame = serde_json::to_string(ev).expect("serializable event");
    sink.send(&frame).map_err(|_| ())
}

async fn run_stream_call(
    iii: &IIIClient,
    http: reqwest::Client,
    abort_reg: Option<&AbortGuard>,
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
        Err(_) => {
            let _ = send_event(
                sink,
                &synthetic_error_event(
                    "provider openrouter not configured (no api_key in the llm-router entry and OPENROUTER_API_KEY unset)",
                    &model,
                    ErrorKind::Permanent,
                ),
            );
            return;
        }
    };

    // The unified reasoning parameter: requested level resolved against the
    // model's advertised efforts. A model that advertises no reasoning at all
    // keeps its default behavior; the level is reported as ignored.
    let meta = input.model_meta.as_ref();
    let reasoning_effort = match input.thinking_level {
        Some(level) => {
            if meta.is_some_and(|m| m.supports_thinking == Some(false)) {
                warnings.push(format!(
                    "thinking_level ignored: {} does not support the reasoning parameter",
                    upstream_id(&model)
                ));
                None
            } else {
                let effort = resolve_reasoning_effort(level, meta);
                if effort.is_none() {
                    warnings.push(
                        "thinking_level ignored: no advertised reasoning effort matches the request"
                            .to_string(),
                    );
                }
                effort
            }
        }
        None => None,
    };

    // Strict json_schema only on models that declare structured_outputs; a
    // schema requested for any other model degrades to json_object and the
    // caller is told (report-and-continue, spec § stream contract).
    let allow_json_schema = meta.is_none_or(|m| m.supports_structured_output != Some(false));
    if !allow_json_schema
        && input
            .response_format
            .as_ref()
            .and_then(|rf| rf.schema.as_ref())
            .is_some()
    {
        warnings.push(format!(
            "response_format json_schema degraded to json_object: {} does not declare structured_outputs",
            upstream_id(&model)
        ));
    }

    let body = build_body(&BodyArgs {
        // the wire wants OpenRouter's own id, not the prefixed catalog id
        model: upstream_id(&cfg.model).to_string(),
        max_tokens: cfg.max_tokens,
        system_prompt: input.system_prompt.unwrap_or_default(),
        messages: input.messages,
        tools: input.tools.unwrap_or_default(),
        response_format: input.response_format,
        reasoning_effort,
        allow_json_schema,
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
    // openrouter discards the terminal error_kind (no cached credential to drop).
    match abort_reg {
        Some(g) => {
            pump_abortable(rx, sink, PING_INTERVAL, g.watch()).await;
        }
        None => pump(rx, sink, PING_INTERVAL).await,
    }
}

/// Forward upstream events to the sink; ping through silence; stop on the
/// terminal event or on a failed write (caller gone → dropping `rx` aborts
/// the upstream task and its in-flight HTTP request).
/// Verbatim copy of provider-anthropic's pump — shared extraction into
/// llm-router is a listed follow-up.
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
            // Upstream task ended without a terminal (panic/abort): the
            // router synthesizes the terminal frame — never two terminals.
            Ok(None) => return,
            // Silent stretch: heartbeat (also probes for a gone caller).
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
    use llm_router::types::model::ReasoningEffort;
    use serde_json::Value;

    fn done_event() -> AssistantMessageEvent {
        AssistantMessageEvent::Done {
            message: empty_assistant("gpt-test"),
        }
    }

    fn meta_with_efforts(efforts: &[&str]) -> Model {
        Model {
            id: "openrouter/vendor/m".into(),
            provider: "openrouter".into(),
            display_name: None,
            context_window: 8192,
            max_output_tokens: 4096,
            input_limit: None,
            supports_thinking: Some(true),
            supports_xhigh: None,
            reasoning_efforts: Some(
                efforts
                    .iter()
                    .map(|e| ReasoningEffort {
                        effort: e.to_string(),
                        description: None,
                    })
                    .collect(),
            ),
            supports_tools: Some(true),
            supports_vision: None,
            supports_cache: None,
            supports_structured_output: None,
            thinking_budgets: None,
            pricing: None,
        }
    }

    #[test]
    fn effort_resolution_steps_down_through_advertised_ladder() {
        let meta = meta_with_efforts(&["high", "medium", "low"]);
        assert_eq!(
            resolve_reasoning_effort(ThinkingLevel::Xhigh, Some(&meta)).as_deref(),
            Some("high"),
            "xhigh unavailable → high"
        );
        assert_eq!(
            resolve_reasoning_effort(ThinkingLevel::High, Some(&meta)).as_deref(),
            Some("high")
        );
        assert_eq!(
            resolve_reasoning_effort(ThinkingLevel::Minimal, Some(&meta)).as_deref(),
            Some("low"),
            "minimal unavailable → low"
        );
        let none_match = meta_with_efforts(&["exotic"]);
        assert_eq!(
            resolve_reasoning_effort(ThinkingLevel::High, Some(&none_match)),
            None
        );
    }

    #[test]
    fn effort_resolution_without_metadata_sends_the_request_as_is() {
        assert_eq!(
            resolve_reasoning_effort(ThinkingLevel::Xhigh, None).as_deref(),
            Some("xhigh")
        );
        assert_eq!(
            resolve_reasoning_effort(ThinkingLevel::Medium, None).as_deref(),
            Some("medium")
        );
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
        // a frame after the terminal must never be forwarded
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

    #[tokio::test(flavor = "multi_thread")]
    async fn pings_through_silence() {
        let ch = FakeChannel::new();
        let (tx, rx) = mpsc::channel::<AssistantMessageEvent>(8);
        // hold tx open, send nothing for > 2 ping intervals, then terminate
        let pump_task = {
            let writer = ch.writer.clone();
            tokio::spawn(async move { pump(rx, &writer, Duration::from_millis(50)).await })
        };
        tokio::time::sleep(Duration::from_millis(140)).await;
        tx.send(done_event()).await.unwrap();
        drop(tx);
        pump_task.await.unwrap();
        ch.writer.close();

        let mut frames = Vec::new();
        let mut reader = ch.reader;
        while let llm_router::chat::relay::ReadEvent::Msg(m) =
            reader.next(Duration::from_millis(100)).await
        {
            frames.push(m);
        }
        let pings = frames
            .iter()
            .filter(|f| serde_json::from_str::<Value>(f).unwrap()["type"] == "ping")
            .count();
        assert!(
            pings >= 2,
            "want >=2 pings through 140ms of silence, got {pings}"
        );
        assert_eq!(
            serde_json::from_str::<Value>(frames.last().unwrap()).unwrap()["type"],
            "done"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn reader_close_stops_the_pump_and_drops_the_receiver() {
        let ch = FakeChannel::new();
        ch.reader.close(); // caller gone before anything is written
        let (tx, rx) = mpsc::channel(8);
        tx.send(AssistantMessageEvent::Start {
            partial: empty_assistant("m"),
        })
        .await
        .unwrap();
        pump(rx, &ch.writer, Duration::from_secs(30)).await; // returns immediately
                                                             // the receiver was consumed and dropped by pump → upstream send fails
        assert!(tx.send(done_event()).await.is_err());
    }
}

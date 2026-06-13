//! The `provider::openai::stream` iii function (spec § Provider stream
//! contract): write AssistantMessageEvent frames as JSON text messages into
//! the router-owned channel, terminal done/error last, then close.
use crate::config::config_from_resolve;
use crate::errors::{classify_bus_error, invalid_request};
use crate::reasoning::{is_reasoning_model, reasoning_effort_for};
use crate::request::{build_body, build_headers, BodyArgs};
use crate::sse::synthetic_error_event;
use crate::upstream::{spawn_upstream, UpstreamArgs};
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::{IIIError, III};
use llm_router::channels::open_sink;
use llm_router::chat::relay::FrameSink;
use llm_router::types::events::{AssistantMessageEvent, ErrorKind};
use llm_router::types::router::ProviderStreamInput;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::sync::mpsc;

/// Heartbeat cadence while the upstream is silent (spec: at least every 30s).
pub const PING_INTERVAL: Duration = Duration::from_secs(30);

pub fn make_stream(
    iii: III,
    http: reqwest::Client,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |raw: Value| {
        let (iii, http) = (iii.clone(), http.clone());
        Box::pin(async move {
            let input: ProviderStreamInput = serde_json::from_value(raw)
                .map_err(|e| invalid_request(format!("bad ProviderStreamInput: {e}")))?;
            let sink = open_sink(&iii, &input.writer_ref).await?;
            run_stream_call(&iii, http, input, sink.as_ref()).await;
            sink.close();
            // ProviderStreamOutput (spec § stream contract)
            Ok(json!({ "ok": true }))
        })
    }
}

fn send_event(sink: &dyn FrameSink, ev: &AssistantMessageEvent) -> Result<(), ()> {
    let frame = serde_json::to_string(ev).expect("serializable event");
    sink.send(&frame).map_err(|_| ())
}

async fn run_stream_call(
    iii: &III,
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
        Err(_) => {
            let _ = send_event(
                sink,
                &synthetic_error_event(
                    "provider openai not configured (no api_key in the llm-router entry and OPENAI_API_KEY unset)",
                    &model,
                    ErrorKind::Permanent,
                ),
            );
            return;
        }
    };

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

    fn done_event() -> AssistantMessageEvent {
        AssistantMessageEvent::Done {
            message: empty_assistant("gpt-test"),
        }
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

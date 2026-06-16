//! The `router::complete` iii function: internal iii channel, drain, return
//! the final message. A mid-stream failure returns the synthesized error
//! message, not a throw.
//!
//! Engine-backed coverage: tests/integration.rs.
use std::sync::Arc;
use std::time::Duration;

use crate::types::errors::{RouterCode, RouterError};
use crate::types::events::AssistantMessageEvent;
use crate::types::router::CompleteResponse;
use futures::future::BoxFuture;
use iii_sdk::{IIIError, III};

use super::chat::{ChatCall, ChatPipeline};
use super::relay::ReadEvent;
use crate::channels::create_router_channel;

pub fn make_complete(
    iii: III,
    pipeline: Arc<ChatPipeline>,
) -> impl Fn(ChatCall) -> BoxFuture<'static, Result<CompleteResponse, IIIError>> + Send + Sync + 'static
{
    move |call: ChatCall| {
        let (iii, pipeline) = (iii.clone(), pipeline.clone());
        Box::pin(async move {
            let channel = create_router_channel(&iii).await?;
            let mut reader = channel.reader;
            let mut chat_task = {
                let sink = channel.writer.clone();
                tokio::spawn(async move {
                    let result = pipeline.run(call, sink.clone()).await;
                    sink.close(); // the internal channel must EOF for the drain
                    result
                })
            };

            // Drain the channel and drive the pipeline concurrently. The
            // boundary must not depend on the channel EOF-ing: a pipeline Err
            // that never wrote a frame leaves the channel without an EOF (a
            // zero-write close does not propagate), so a sequential
            // drain-then-await blocks for the full reader budget. Racing the
            // two surfaces any pipeline Err immediately — for every current
            // and future error path.
            let mut terminal: Option<AssistantMessageEvent> = None;
            let collect = |terminal: &mut Option<AssistantMessageEvent>, m: &str| {
                if let Ok(ev) = serde_json::from_str::<AssistantMessageEvent>(m) {
                    if ev.is_terminal() {
                        *terminal = Some(ev);
                    }
                }
            };
            let response = loop {
                tokio::select! {
                    // outer ≥ inner budget, always
                    ev = reader.next(Duration::from_secs(600)) => match ev {
                        ReadEvent::Msg(m) => collect(&mut terminal, &m),
                        _ => break chat_task.await.map_err(|e| IIIError::Handler(e.to_string()))??,
                    },
                    res = &mut chat_task => {
                        let response = res.map_err(|e| IIIError::Handler(e.to_string()))??;
                        // The pipeline is done; remaining frames are in-flight
                        // engine hops, not a live stream — finish the drain on
                        // a short budget instead of the streaming one.
                        while let ReadEvent::Msg(m) = reader.next(Duration::from_secs(5)).await {
                            collect(&mut terminal, &m);
                        }
                        break response;
                    }
                }
            };
            let message = match terminal {
                Some(AssistantMessageEvent::Done { message }) => message,
                Some(AssistantMessageEvent::Error { error }) => error,
                _ => {
                    return Err(RouterError::new(
                        RouterCode::InvalidRequest,
                        "stream produced no terminal frame",
                    )
                    .into())
                }
            };
            let usage = message.usage.clone().or(response.usage);
            Ok(CompleteResponse {
                message,
                usage,
                provider: response.provider,
                model: response.model,
            })
        })
    }
}

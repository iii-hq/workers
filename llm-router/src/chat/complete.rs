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
use serde_json::Value;

use super::chat::{ChatCall, ChatPipeline};
use super::relay::ReadEvent;
use crate::channels::create_router_channel;

pub fn make_complete(
    iii: III,
    pipeline: Arc<ChatPipeline>,
) -> impl Fn(Value) -> BoxFuture<'static, Result<Value, IIIError>> + Send + Sync + 'static {
    move |raw: Value| {
        let (iii, pipeline) = (iii.clone(), pipeline.clone());
        Box::pin(async move {
            let call: ChatCall = serde_json::from_value(raw).map_err(|e| {
                IIIError::from(RouterError::new(RouterCode::InvalidRequest, e.to_string()))
            })?;
            let channel = create_router_channel(&iii).await?;
            let mut reader = channel.reader;
            let chat_task = {
                let sink = channel.writer.clone();
                tokio::spawn(async move {
                    let result = pipeline.run(call, sink.clone()).await;
                    sink.close(); // the internal channel must EOF for the drain
                    result
                })
            };

            let mut terminal: Option<AssistantMessageEvent> = None;
            // outer ≥ inner budget, always
            while let ReadEvent::Msg(m) = reader.next(Duration::from_secs(600)).await {
                if let Ok(ev) = serde_json::from_str::<AssistantMessageEvent>(&m) {
                    if ev.is_terminal() {
                        terminal = Some(ev);
                    }
                }
            }
            let response = chat_task
                .await
                .map_err(|e| IIIError::Handler(e.to_string()))??;
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
            Ok(serde_json::to_value(CompleteResponse {
                message,
                usage,
                provider: response.provider,
                model: response.model,
            })
            .expect("serializable response"))
        })
    }
}

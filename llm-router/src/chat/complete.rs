//! The `router::complete` iii function: internal iii channel, drain, return
//! the final message. A mid-stream failure returns the synthesized error
//! message, not a throw.
use std::sync::Arc;
use std::time::Duration;

use crate::types::errors::{RouterCode, RouterError};
use crate::types::events::AssistantMessageEvent;
use crate::types::router::{ChatResponse, CompleteResponse};
use futures::future::BoxFuture;
use serde_json::Value;

use super::chat::ChatCall;
use super::relay::{ChannelFactory, FrameSink, ReadEvent};
use crate::bus::{handler, BusError, Handler};

pub type RunChat = Arc<
    dyn Fn(ChatCall, Arc<dyn FrameSink>) -> BoxFuture<'static, Result<ChatResponse, BusError>>
        + Send
        + Sync,
>;

pub fn make_complete(run_chat: RunChat, channels: Arc<dyn ChannelFactory>) -> Handler {
    handler(move |raw: Value| {
        let (run_chat, channels) = (run_chat.clone(), channels.clone());
        async move {
            let call: ChatCall = serde_json::from_value(raw).map_err(|e| {
                BusError::from(RouterError::new(RouterCode::InvalidRequest, e.to_string()))
            })?;
            let channel = channels.create().await?;
            let mut reader = channel.reader;
            let chat_task = {
                let sink = channel.writer.clone();
                tokio::spawn(async move { run_chat(call, sink).await })
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
                .map_err(|e| BusError::Transport(e.to_string()))??;
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
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::relay::ChannelFactory;
    use crate::testkit::fake_channels::ChannelHub;
    use crate::types::events::AssistantMessageEvent;
    use serde_json::json;
    use std::sync::Arc;

    #[tokio::test]
    async fn drains_the_internal_channel_and_returns_the_final_message() {
        let hub = ChannelHub::new();
        // a run_chat stand-in that streams a done frame into the supplied sink
        let run = Arc::new(
            |_call: super::super::chat::ChatCall, sink: Arc<dyn crate::chat::relay::FrameSink>| {
                Box::pin(async move {
                    let mut m = crate::chat::synthesize::empty_partial("m", "p", 2);
                    m.content.push(crate::types::content::ContentBlock::Text {
                        text: "Hello".into(),
                    });
                    sink.send(
                        &serde_json::to_string(&AssistantMessageEvent::Done { message: m })
                            .unwrap(),
                    )
                    .unwrap();
                    sink.close();
                    Ok(crate::types::router::ChatResponse {
                        ok: true,
                        provider: "p".into(),
                        model: "m".into(),
                        stop_reason: Some(crate::types::events::StopReason::End),
                        usage: None,
                        error: None,
                    })
                })
                    as futures::future::BoxFuture<
                        'static,
                        Result<crate::types::router::ChatResponse, crate::bus::BusError>,
                    >
            },
        );
        let complete = make_complete(run, hub as Arc<dyn ChannelFactory>);
        let res = complete(json!({ "model": "m", "messages": [] }))
            .await
            .unwrap();
        assert_eq!(res["provider"], "p");
        assert_eq!(res["message"]["content"][0]["text"], "Hello");
        assert_eq!(res["message"]["stop_reason"], "end");
    }
}

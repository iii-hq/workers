use crate::errors::classify;
use crate::request::WireDialect;
use crate::sse::{synthetic_error, DecoderState};
use futures::StreamExt;
use llm_router::provider_scaffold::sse_transport::{
    append_utf8_chunk, drain_sse_blocks, error_chain,
};
use llm_router::types::events::{AssistantMessageEvent, ErrorKind};
use serde_json::Value;
use tokio::sync::mpsc;

pub struct UpstreamArgs {
    pub url: String,
    pub model: String,
    pub dialect: WireDialect,
    pub body: Value,
    pub headers: Vec<(&'static str, String)>,
    pub warnings: Vec<String>,
}

pub fn spawn_upstream(
    client: reqwest::Client,
    args: UpstreamArgs,
) -> mpsc::Receiver<AssistantMessageEvent> {
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        let closed = tx.clone();
        tokio::select! {
            _ = run_upstream(client, args, tx) => {}
            _ = closed.closed() => {}
        }
    });
    rx
}

async fn run_upstream(
    client: reqwest::Client,
    args: UpstreamArgs,
    tx: mpsc::Sender<AssistantMessageEvent>,
) {
    let mut request = client.post(&args.url);
    for (name, value) in &args.headers {
        request = request.header(*name, value);
    }
    let response = match request.json(&args.body).send().await {
        Ok(response) => response,
        Err(error) => {
            let _ = tx
                .send(synthetic_error(
                    &args.model,
                    format!("Command Code fetch failed: {}", error_chain(&error)),
                    ErrorKind::Transient,
                ))
                .await;
            return;
        }
    };
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let message = if body.is_empty() {
            format!("Command Code http {status}")
        } else {
            body.clone()
        };
        let _ = tx
            .send(synthetic_error(
                &args.model,
                message,
                classify(Some(status.as_u16()), &body),
            ))
            .await;
        return;
    }

    let mut state = DecoderState::new(args.dialect, args.warnings);
    if tx
        .send(AssistantMessageEvent::Start {
            partial: state.partial(&args.model),
        })
        .await
        .is_err()
    {
        return;
    }
    let mut stream = response.bytes_stream();
    let mut text = String::new();
    let mut byte_buffer = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(chunk) => chunk,
            Err(error) => {
                let _ = tx
                    .send(state.error_event(
                        &args.model,
                        format!("stream read failed: {error}"),
                        ErrorKind::Transient,
                    ))
                    .await;
                return;
            }
        };
        append_utf8_chunk(&mut byte_buffer, &mut text, &chunk);
        let terminal = drain_sse_blocks(&mut text, &tx, &mut |block| {
            state.handle_block(block, &args.model)
        })
        .await;
        if terminal {
            return;
        }
    }
    if !byte_buffer.is_empty() {
        text.push_str(&String::from_utf8_lossy(&byte_buffer));
    }
    if !text.trim().is_empty() {
        text.push_str("\n\n");
        if drain_sse_blocks(&mut text, &tx, &mut |block| {
            state.handle_block(block, &args.model)
        })
        .await
        {
            return;
        }
    }
    let _ = tx.send(state.eof_event(&args.model)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::content::ContentBlock;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn stub(response: &'static str, path: &str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut request = [0_u8; 65_536];
                let _ = socket.read(&mut request).await;
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        format!("http://{address}/{path}")
    }

    fn args(url: String, dialect: WireDialect, model: &str) -> UpstreamArgs {
        UpstreamArgs {
            url,
            model: model.to_string(),
            dialect,
            body: serde_json::json!({ "stream": true }),
            headers: vec![("authorization", "Bearer cmd-test".to_string())],
            warnings: vec![],
        }
    }

    async fn drain(
        mut receiver: mpsc::Receiver<AssistantMessageEvent>,
    ) -> Vec<AssistantMessageEvent> {
        let mut events = Vec::new();
        while let Some(event) = receiver.recv().await {
            events.push(event);
        }
        events
    }

    const CHAT_HAPPY: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":2,\"prompt_tokens_details\":{\"cached_tokens\":4}}}\n\ndata: [DONE]\n\n";

    const CHAT_READ_ERROR_BEFORE_FINISH: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 4096\r\nconnection: close\r\n\r\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":2}}\n\n";

    const CHAT_READ_ERROR_AFTER_FINISH: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 4096\r\nconnection: close\r\n\r\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":2}}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n";

    const MESSAGES_HAPPY: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\nevent: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12,\"cache_read_input_tokens\":4}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

    fn assert_happy(events: &[AssistantMessageEvent], expected_input: u64) {
        assert!(matches!(
            events.first(),
            Some(AssistantMessageEvent::Start { .. })
        ));
        match events.last() {
            Some(AssistantMessageEvent::Done { message }) => {
                assert!(matches!(
                    message.content.first(),
                    Some(ContentBlock::Text { text }) if text == "Hello"
                ));
                let usage = message.usage.as_ref().expect("reported usage");
                assert_eq!(usage.input, Some(expected_input));
                assert_eq!(usage.output, Some(2));
                assert_eq!(usage.cache_read, Some(4));
                assert_eq!(usage.cost_usd, None);
            }
            other => panic!("want done, got {other:?}"),
        }
        assert_eq!(events.iter().filter(|event| event.is_terminal()).count(), 1);
    }

    fn assert_read_error(events: &[AssistantMessageEvent], native_stop_reason: Option<&str>) {
        match events.last() {
            Some(AssistantMessageEvent::Error { error }) => {
                assert!(matches!(
                    error.content.first(),
                    Some(ContentBlock::Text { text }) if text == "Hello"
                ));
                let usage = error.usage.as_ref().expect("reported usage");
                assert_eq!(usage.input, Some(12));
                assert_eq!(usage.output, Some(2));
                assert_eq!(
                    error.stop_reason,
                    llm_router::types::events::StopReason::Error
                );
                assert_eq!(error.error_kind, Some(ErrorKind::Transient));
                assert!(error
                    .error_message
                    .as_deref()
                    .is_some_and(|message| message.contains("stream read failed")));
                assert_eq!(error.native_stop_reason.as_deref(), native_stop_reason);
            }
            other => panic!("want error, got {other:?}"),
        }
        assert_eq!(events.iter().filter(|event| event.is_terminal()).count(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn chat_completions_wire_is_decoded_without_double_counting_cache() {
        let url = stub(CHAT_HAPPY, "provider/v1/chat/completions").await;
        let events = drain(spawn_upstream(
            reqwest::Client::new(),
            args(url, WireDialect::ChatCompletions, "command-code/gpt-test"),
        ))
        .await;
        assert_happy(&events, 8);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn anthropic_messages_wire_is_decoded_without_inventing_cost() {
        let url = stub(MESSAGES_HAPPY, "provider/v1/messages").await;
        let events = drain(spawn_upstream(
            reqwest::Client::new(),
            args(
                url,
                WireDialect::AnthropicMessages,
                "command-code/claude-test",
            ),
        ))
        .await;
        assert_happy(&events, 12);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn chat_read_failure_before_finish_preserves_partial_content_and_usage() {
        let url = stub(
            CHAT_READ_ERROR_BEFORE_FINISH,
            "provider/v1/chat/completions",
        )
        .await;
        let events = drain(spawn_upstream(
            reqwest::Client::new(),
            args(url, WireDialect::ChatCompletions, "command-code/gpt-test"),
        ))
        .await;
        assert_read_error(&events, None);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn chat_read_failure_after_finish_is_error_with_partial_state() {
        let url = stub(CHAT_READ_ERROR_AFTER_FINISH, "provider/v1/chat/completions").await;
        let events = drain(spawn_upstream(
            reqwest::Client::new(),
            args(url, WireDialect::ChatCompletions, "command-code/gpt-test"),
        ))
        .await;
        assert_read_error(&events, Some("stop"));
    }
}

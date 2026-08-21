use llm_router::provider_scaffold::names::encode_tool_name;
use llm_router::types::content::ContentBlock;
use llm_router::types::messages::{AgentMessage, FunctionResultMessage};
use serde_json::{json, Value};
use std::collections::HashSet;

const ORPHAN_TOOL_PLACEHOLDER: &str =
    "Tool call was interrupted before completing. Continue without its output.";

fn result_text(message: &FunctionResultMessage) -> String {
    let body = message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if message.details.get("status").and_then(Value::as_str) == Some("denied") {
        let envelope = serde_json::to_string(&message.details).unwrap_or_else(|_| "{}".into());
        format!("[PERMISSION_DENIED]\n{envelope}\n\n{body}")
    } else {
        body
    }
}

fn result_images(message: &FunctionResultMessage) -> Vec<(String, String)> {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Image { mime, data } => Some((mime.clone(), data.clone())),
            _ => None,
        })
        .collect()
}

fn chat_image_part(mime: &str, data: &str) -> Value {
    json!({
        "type": "image_url",
        "image_url": { "url": format!("data:{mime};base64,{data}") }
    })
}

fn anthropic_image_block(mime: &str, data: &str) -> Value {
    json!({
        "type": "image",
        "source": { "type": "base64", "media_type": mime, "data": data }
    })
}

fn flush_result_images(out: &mut Vec<Value>, images: Vec<(String, Vec<(String, String)>)>) {
    if images.is_empty() {
        return;
    }
    let mut parts = Vec::new();
    for (call_id, call_images) in images {
        parts.push(json!({
            "type": "text",
            "text": format!("[image result of tool call {call_id}]")
        }));
        for (mime, data) in call_images {
            parts.push(chat_image_part(&mime, &data));
        }
    }
    out.push(json!({ "role": "user", "content": parts }));
}

fn chat_user_content(content: &[ContentBlock]) -> Value {
    let has_images = content
        .iter()
        .any(|block| matches!(block, ContentBlock::Image { .. }));
    let text = content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    if !has_images {
        return Value::String(text);
    }
    let mut parts = Vec::new();
    if !text.is_empty() {
        parts.push(json!({ "type": "text", "text": text }));
    }
    for block in content {
        if let ContentBlock::Image { mime, data } = block {
            parts.push(chat_image_part(mime, data));
        }
    }
    Value::Array(parts)
}

fn chat_calls(assistant: &llm_router::types::messages::AssistantMessage) -> Vec<(String, Value)> {
    assistant
        .content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::FunctionCall {
                id,
                function_id,
                arguments,
            } => Some((
                id.clone(),
                json!({
                    "id": id,
                    "type": "function",
                    "function": {
                        "name": encode_tool_name(function_id),
                        "arguments": arguments.to_string(),
                    }
                }),
            )),
            _ => None,
        })
        .collect()
}

pub fn chat_messages(messages: &[AgentMessage], system_prompt: &str) -> Vec<Value> {
    let ordered = llm_router::types::messages::reorder_displaced_results(messages);
    let mut out = Vec::new();
    if !system_prompt.is_empty() {
        out.push(json!({ "role": "system", "content": system_prompt }));
    }
    let mut index = 0;
    while index < ordered.len() {
        match ordered[index] {
            AgentMessage::User(user) => {
                out.push(json!({
                    "role": "user",
                    "content": chat_user_content(&user.content),
                }));
                index += 1;
            }
            AgentMessage::Custom(_) | AgentMessage::FunctionResult(_) => index += 1,
            AgentMessage::Assistant(assistant) => {
                let text = assistant
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        ContentBlock::Text { text } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let calls = chat_calls(assistant);
                if text.is_empty() && calls.is_empty() {
                    index += 1;
                    continue;
                }
                let mut row = json!({ "role": "assistant" });
                if !text.is_empty() {
                    row["content"] = Value::String(text);
                }
                if calls.is_empty() {
                    out.push(row);
                    index += 1;
                    continue;
                }
                row["tool_calls"] =
                    Value::Array(calls.iter().map(|(_, value)| value.clone()).collect());
                out.push(row);

                let mut cursor = index + 1;
                let mut results = Vec::new();
                while cursor < ordered.len() {
                    match ordered[cursor] {
                        AgentMessage::FunctionResult(result) => {
                            results.push((
                                result.function_call_id.clone(),
                                result_text(result),
                                result_images(result),
                            ));
                            cursor += 1;
                        }
                        AgentMessage::Custom(_) => cursor += 1,
                        _ => break,
                    }
                }
                let mut consumed = vec![false; results.len()];
                let mut images = Vec::new();
                for (call_id, _) in &calls {
                    let match_index = (0..results.len()).rev().find(|result_index| {
                        !consumed[*result_index] && results[*result_index].0 == *call_id
                    });
                    let content = match match_index {
                        Some(result_index) => {
                            consumed[result_index] = true;
                            if !results[result_index].2.is_empty() {
                                images.push((call_id.clone(), results[result_index].2.clone()));
                            }
                            results[result_index].1.clone()
                        }
                        None => ORPHAN_TOOL_PLACEHOLDER.to_string(),
                    };
                    out.push(json!({
                        "role": "tool",
                        "tool_call_id": call_id,
                        "content": content,
                    }));
                }
                flush_result_images(&mut out, images);
                index = cursor;
            }
        }
    }
    out
}

fn anthropic_content(block: &ContentBlock) -> Option<Value> {
    match block {
        ContentBlock::Text { text } => Some(json!({ "type": "text", "text": text })),
        ContentBlock::Image { mime, data } => Some(anthropic_image_block(mime, data)),
        ContentBlock::FunctionCall {
            id,
            function_id,
            arguments,
        } => Some(json!({
            "type": "tool_use",
            "id": id,
            "name": encode_tool_name(function_id),
            "input": if arguments.is_object() { arguments.clone() } else { json!({}) },
        })),
        ContentBlock::Thinking { text, signature } => signature.as_ref().map(
            |signature| json!({ "type": "thinking", "thinking": text, "signature": signature }),
        ),
        ContentBlock::RedactedThinking { data } => {
            Some(json!({ "type": "redacted_thinking", "data": data }))
        }
        ContentBlock::FunctionResult { .. } => None,
    }
}

fn anthropic_result(message: &FunctionResultMessage) -> Value {
    let has_images = message
        .content
        .iter()
        .any(|block| matches!(block, ContentBlock::Image { .. }));
    let content = if has_images {
        let mut blocks = Vec::new();
        let text = result_text(message);
        if !text.is_empty() {
            blocks.push(json!({ "type": "text", "text": text }));
        }
        blocks.extend(message.content.iter().filter_map(|block| match block {
            ContentBlock::Image { mime, data } => Some(anthropic_image_block(mime, data)),
            _ => None,
        }));
        Value::Array(blocks)
    } else {
        Value::String(result_text(message))
    };
    json!({
        "type": "tool_result",
        "tool_use_id": message.function_call_id,
        "content": content,
        "is_error": message.is_error,
    })
}

pub fn anthropic_messages(messages: &[AgentMessage], warnings: &mut Vec<String>) -> Vec<Value> {
    let messages = llm_router::types::messages::reorder_displaced_results(messages);
    let mut out = Vec::new();
    let mut pending = Vec::new();
    let mut resolved: HashSet<String> = messages
        .iter()
        .filter_map(|message| match message {
            AgentMessage::FunctionResult(result) => Some(result.function_call_id.clone()),
            _ => None,
        })
        .collect();
    let emitted: HashSet<String> = messages
        .iter()
        .flat_map(|message| match message {
            AgentMessage::Assistant(assistant) => assistant
                .content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::FunctionCall { id, .. } => Some(id.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
        .collect();

    for message in messages {
        match message {
            AgentMessage::User(user) => {
                let content = pending
                    .drain(..)
                    .chain(user.content.iter().filter_map(anthropic_content))
                    .collect::<Vec<_>>();
                out.push(json!({ "role": "user", "content": content }));
            }
            AgentMessage::Assistant(assistant) => {
                if !pending.is_empty() {
                    out.push(json!({
                        "role": "user",
                        "content": std::mem::take(&mut pending),
                    }));
                }
                let mut content = assistant
                    .content
                    .iter()
                    .filter_map(anthropic_content)
                    .collect::<Vec<_>>();
                while matches!(
                    content
                        .last()
                        .and_then(|block| block.get("type"))
                        .and_then(Value::as_str),
                    Some("thinking") | Some("redacted_thinking")
                ) {
                    content.pop();
                }
                if !content.is_empty() {
                    out.push(json!({ "role": "assistant", "content": content }));
                }
                for block in &assistant.content {
                    if let ContentBlock::FunctionCall { id, .. } = block {
                        if !resolved.contains(id) {
                            pending.push(json!({
                                "type": "tool_result",
                                "tool_use_id": id,
                                "content": ORPHAN_TOOL_PLACEHOLDER,
                                "is_error": true,
                            }));
                            resolved.insert(id.clone());
                        }
                    }
                }
            }
            AgentMessage::FunctionResult(result) => {
                if !emitted.contains(&result.function_call_id) {
                    continue;
                }
                let block = anthropic_result(result);
                match pending.iter().position(|pending: &Value| {
                    pending.get("tool_use_id").and_then(Value::as_str)
                        == Some(result.function_call_id.as_str())
                }) {
                    Some(index) => pending[index] = block,
                    None => pending.push(block),
                }
            }
            AgentMessage::Custom(_) => {}
        }
    }
    if !pending.is_empty() {
        out.push(json!({ "role": "user", "content": pending }));
    }

    let mut dropped_prefill = false;
    while out
        .last()
        .and_then(|row| row.get("role"))
        .and_then(Value::as_str)
        == Some("assistant")
    {
        out.pop();
        dropped_prefill = true;
    }
    if dropped_prefill {
        warnings.push(
            "trailing partial assistant message dropped: Anthropic Messages does not support assistant prefill"
                .to_string(),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::events::StopReason;
    use llm_router::types::messages::{
        AssistantMessage, AssistantRoleTag, FunctionResultMessage, FunctionResultRoleTag,
        UserMessage, UserRoleTag,
    };

    fn assistant_calls(ids: &[&str]) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content: ids
                .iter()
                .map(|id| ContentBlock::FunctionCall {
                    id: (*id).into(),
                    function_id: "shell::exec".into(),
                    arguments: json!({ "cmd": "pwd" }),
                })
                .collect(),
            stop_reason: StopReason::FunctionCall,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            warnings: None,
            usage: None,
            model: "m".into(),
            provider: "p".into(),
            timestamp: 1,
        })
    }

    fn assistant_call() -> AgentMessage {
        assistant_calls(&["call-1"])
    }

    fn result_with_content(id: &str, content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage::FunctionResult(FunctionResultMessage {
            role: FunctionResultRoleTag::FunctionResult,
            function_call_id: id.into(),
            function_id: "shell::exec".into(),
            content,
            details: json!({}),
            is_error: false,
            timestamp: 2,
        })
    }

    fn result() -> AgentMessage {
        result_with_content(
            "call-1",
            vec![ContentBlock::Text {
                text: "/tmp".into(),
            }],
        )
    }

    fn user(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            timestamp: 3,
        })
    }

    #[test]
    fn tool_names_and_results_follow_each_native_schema() {
        let transcript = vec![assistant_call(), result()];
        let chat = chat_messages(&transcript, "");
        assert_eq!(chat[0]["tool_calls"][0]["function"]["name"], "shell__exec");
        assert_eq!(chat[1]["role"], "tool");
        assert_eq!(chat[1]["tool_call_id"], "call-1");

        let messages = anthropic_messages(&transcript, &mut Vec::new());
        assert_eq!(messages[0]["content"][0]["type"], "tool_use");
        assert_eq!(messages[0]["content"][0]["name"], "shell__exec");
        assert_eq!(messages[1]["content"][0]["type"], "tool_result");
    }

    #[test]
    fn orphan_calls_get_native_placeholders() {
        let transcript = vec![assistant_call()];
        let chat = chat_messages(&transcript, "");
        assert_eq!(chat[1]["role"], "tool");
        assert!(chat[1]["content"].as_str().unwrap().contains("interrupted"));

        let messages = anthropic_messages(&transcript, &mut Vec::new());
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"][0]["type"], "tool_result");
    }

    #[test]
    fn chat_result_images_follow_the_contiguous_tool_row_run() {
        let transcript = vec![
            assistant_calls(&["call-1", "call-2"]),
            result_with_content(
                "call-1",
                vec![
                    ContentBlock::Text {
                        text: "screenshot".into(),
                    },
                    ContentBlock::Image {
                        mime: "image/png".into(),
                        data: "QUJD".into(),
                    },
                ],
            ),
            result_with_content("call-2", vec![ContentBlock::Text { text: "ok".into() }]),
            user("next"),
        ];
        let chat = chat_messages(&transcript, "");
        assert_eq!(chat.len(), 5);
        assert_eq!(chat[0]["role"], "assistant");
        assert_eq!(chat[1]["role"], "tool");
        assert_eq!(chat[1]["tool_call_id"], "call-1");
        assert_eq!(chat[1]["content"], "screenshot");
        assert_eq!(chat[2]["role"], "tool");
        assert_eq!(chat[2]["tool_call_id"], "call-2");
        assert_eq!(chat[2]["content"], "ok");
        assert_eq!(chat[3]["role"], "user");
        let parts = chat[3]["content"].as_array().expect("image parts");
        assert_eq!(parts[0]["type"], "text");
        assert_eq!(parts[0]["text"], "[image result of tool call call-1]");
        assert_eq!(parts[1]["type"], "image_url");
        assert_eq!(parts[1]["image_url"]["url"], "data:image/png;base64,QUJD");
        assert_eq!(chat[4]["role"], "user");
        assert_eq!(chat[4]["content"], "next");
    }
}

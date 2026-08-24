use crate::catalog::upstream_id;
use crate::config::{endpoint, CommandCodeConfig};
use crate::wire::{anthropic_messages, chat_messages};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use llm_router::types::router::ResponseFormat;
use serde_json::{json, Value};

pub const EMPTY_MESSAGES_ERROR: &str = "refusing to call Command Code with an empty messages array";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireDialect {
    ChatCompletions,
    AnthropicMessages,
}

pub fn dialect_for_model(model: &str) -> WireDialect {
    let leaf = upstream_id(model).rsplit('/').next().unwrap_or(model);
    if leaf.starts_with("claude-") {
        WireDialect::AnthropicMessages
    } else {
        WireDialect::ChatCompletions
    }
}

pub struct RequestArgs {
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<AgentFunction>,
    pub response_format: Option<ResponseFormat>,
}

pub struct BuiltRequest {
    pub dialect: WireDialect,
    pub url: String,
    pub body: Value,
    pub headers: Vec<(&'static str, String)>,
}

fn tools_for_chat(tools: &[AgentFunction]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "type": "function",
                "function": {
                    "name": llm_router::provider_scaffold::names::encode_tool_name(&tool.name),
                    "description": tool.description,
                    "parameters": tool.parameters,
                }
            })
        })
        .collect()
}

fn tools_for_anthropic(tools: &[AgentFunction]) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            json!({
                "name": llm_router::provider_scaffold::names::encode_tool_name(&tool.name),
                "description": tool.description,
                "input_schema": tool.parameters,
            })
        })
        .collect()
}

fn chat_response_format(format: &ResponseFormat) -> Value {
    match &format.schema {
        Some(schema) => json!({
            "type": "json_schema",
            "json_schema": { "name": "response", "strict": true, "schema": schema }
        }),
        None => json!({ "type": "json_object" }),
    }
}

pub fn build_request(
    config: &CommandCodeConfig,
    args: &RequestArgs,
    warnings: &mut Vec<String>,
) -> Result<BuiltRequest, &'static str> {
    let upstream_model = upstream_id(&config.model);
    let dialect = dialect_for_model(upstream_model);
    let messages = match dialect {
        WireDialect::ChatCompletions => chat_messages(&args.messages, &args.system_prompt),
        WireDialect::AnthropicMessages => anthropic_messages(&args.messages, warnings),
    };
    if messages.is_empty() {
        return Err(EMPTY_MESSAGES_ERROR);
    }
    let mut body = match dialect {
        WireDialect::ChatCompletions => json!({
            "model": upstream_model,
            "max_tokens": config.max_tokens,
            "messages": messages,
            "stream": true,
            "stream_options": { "include_usage": true },
        }),
        WireDialect::AnthropicMessages => json!({
            "model": upstream_model,
            "max_tokens": config.max_tokens,
            "messages": messages,
            "stream": true,
        }),
    };

    match dialect {
        WireDialect::ChatCompletions => {
            let tools = tools_for_chat(&args.tools);
            if !tools.is_empty() {
                body["tools"] = Value::Array(tools);
            }
            if let Some(format) = &args.response_format {
                body["response_format"] = chat_response_format(format);
            }
        }
        WireDialect::AnthropicMessages => {
            let tools = tools_for_anthropic(&args.tools);
            if !tools.is_empty() {
                body["tools"] = Value::Array(tools);
            }
            if !args.system_prompt.is_empty() {
                body["system"] = Value::String(args.system_prompt.clone());
            }
            if args.response_format.is_some() {
                warnings.push(
                    "response_format ignored: Command Code's /messages route uses the native Anthropic schema"
                        .to_string(),
                );
            }
        }
    }

    let mut headers = vec![
        (
            "authorization",
            format!("Bearer {}", config.credential_value),
        ),
        ("content-type", "application/json".to_string()),
    ];
    if config.zdr {
        headers.push(("x-cmd-zdr", "1".to_string()));
    }
    let path = match dialect {
        WireDialect::ChatCompletions => "chat/completions",
        WireDialect::AnthropicMessages => "messages",
    };
    Ok(BuiltRequest {
        dialect,
        url: endpoint(&config.base_url, path),
        body,
        headers,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_API_URL;
    use llm_router::types::content::ContentBlock;
    use llm_router::types::messages::{CustomMessage, CustomRoleTag, UserMessage, UserRoleTag};

    fn config(model: &str, zdr: bool) -> CommandCodeConfig {
        CommandCodeConfig {
            credential_value: "cmd-secret".into(),
            model: model.into(),
            max_tokens: 2048,
            base_url: DEFAULT_API_URL.into(),
            zdr,
        }
    }

    fn args() -> RequestArgs {
        RequestArgs {
            system_prompt: "be precise".into(),
            messages: vec![AgentMessage::User(UserMessage {
                role: UserRoleTag::User,
                content: vec![ContentBlock::Text {
                    text: "hello".into(),
                }],
                timestamp: 1,
            })],
            tools: vec![],
            response_format: None,
        }
    }

    #[test]
    fn claude_ids_use_messages_and_everything_else_uses_chat() {
        assert_eq!(
            dialect_for_model("command-code/claude-sonnet-4-6"),
            WireDialect::AnthropicMessages
        );
        assert_eq!(
            dialect_for_model("command-code/gpt-5.4"),
            WireDialect::ChatCompletions
        );
        assert_eq!(
            dialect_for_model("command-code/deepseek/deepseek-v4-flash"),
            WireDialect::ChatCompletions
        );
    }

    #[test]
    fn request_paths_and_native_system_shapes_follow_the_dialect() {
        let mut warnings = Vec::new();
        let chat = build_request(
            &config("command-code/gpt-5.4", false),
            &args(),
            &mut warnings,
        )
        .expect("chat request");
        assert!(chat.url.ends_with("/chat/completions"));
        assert_eq!(chat.body["messages"][0]["role"], "system");
        assert!(chat.body.get("system").is_none());
        assert!(!chat.headers.iter().any(|(name, _)| *name == "x-cmd-zdr"));

        let messages = build_request(
            &config("command-code/claude-sonnet-4-6", true),
            &args(),
            &mut warnings,
        )
        .expect("messages request");
        assert!(messages.url.ends_with("/messages"));
        assert_eq!(messages.body["system"], "be precise");
        assert!(messages.headers.contains(&("x-cmd-zdr", "1".to_string())));
        assert!(messages
            .headers
            .contains(&("authorization", "Bearer cmd-secret".to_string())));
    }

    #[test]
    fn converted_empty_transcripts_are_rejected_for_both_dialects() {
        let args = RequestArgs {
            system_prompt: String::new(),
            messages: vec![AgentMessage::Custom(CustomMessage {
                role: CustomRoleTag::Custom,
                custom_type: "ignored".into(),
                content: vec![],
                display: None,
                details: None,
                timestamp: 1,
            })],
            tools: vec![],
            response_format: None,
        };
        for model in ["command-code/gpt-5.4", "command-code/claude-sonnet-4-6"] {
            assert!(matches!(
                build_request(&config(model, false), &args, &mut Vec::new()),
                Err(EMPTY_MESSAGES_ERROR)
            ));
        }
    }
}

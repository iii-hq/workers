use crate::catalog::upstream_id;
use crate::config::{endpoint, CommandCodeConfig};
use crate::wire::{anthropic_messages, chat_messages};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use llm_router::types::router::ResponseFormat;
use serde_json::{json, Value};

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
) -> BuiltRequest {
    let upstream_model = upstream_id(&config.model);
    let dialect = dialect_for_model(upstream_model);
    let mut body = match dialect {
        WireDialect::ChatCompletions => json!({
            "model": upstream_model,
            "max_tokens": config.max_tokens,
            "messages": chat_messages(&args.messages, &args.system_prompt),
            "stream": true,
            "stream_options": { "include_usage": true },
        }),
        WireDialect::AnthropicMessages => json!({
            "model": upstream_model,
            "max_tokens": config.max_tokens,
            "messages": anthropic_messages(&args.messages, warnings),
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
    BuiltRequest {
        dialect,
        url: endpoint(&config.base_url, path),
        body,
        headers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DEFAULT_API_URL;

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
            messages: vec![],
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
        );
        assert!(chat.url.ends_with("/chat/completions"));
        assert_eq!(chat.body["messages"][0]["role"], "system");
        assert!(chat.body.get("system").is_none());
        assert!(!chat.headers.iter().any(|(name, _)| *name == "x-cmd-zdr"));

        let messages = build_request(
            &config("command-code/claude-sonnet-4-6", true),
            &args(),
            &mut warnings,
        );
        assert!(messages.url.ends_with("/messages"));
        assert_eq!(messages.body["system"], "be precise");
        assert!(messages.headers.contains(&("x-cmd-zdr", "1".to_string())));
        assert!(messages
            .headers
            .contains(&("authorization", "Bearer cmd-secret".to_string())));
    }
}

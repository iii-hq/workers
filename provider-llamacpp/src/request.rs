//! Full Chat Completions request assembly: body (messages, tools,
//! response_format) + headers.
use crate::config::LlamacppConfig;
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use llm_router::types::router::ResponseFormat;
use serde_json::{json, Value};

pub struct BodyArgs {
    pub model: String,
    pub max_tokens: u64,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<AgentFunction>,
    pub response_format: Option<ResponseFormat>,
    /// Caller's reasoning preference mapped onto llama.cpp's only per-request
    /// lever, the `enable_thinking` chat-template kwarg (see `build_body`).
    pub enable_thinking: bool,
}

/// `ResponseFormat { type: "json", schema? }` → llama.cpp's real
/// schema-constrained decoding (grammar-backed) when a schema is given,
/// falling back to the looser `json_object` mode otherwise. Unlike OpenAI,
/// llama.cpp's server nests the schema directly under `response_format`
/// (`{"type":"json_schema","schema":{...}}`), not under an extra
/// `json_schema` key (tools/server/README.md § response_format).
pub fn build_response_format(rf: &ResponseFormat) -> Value {
    match &rf.schema {
        Some(schema) => json!({ "type": "json_schema", "schema": schema }),
        None => json!({ "type": "json_object" }),
    }
}

/// llama.cpp's `/v1/chat/completions` documents `max_tokens`; no
/// `temperature` here so the server/model default applies.
pub fn build_body(args: &BodyArgs) -> Value {
    let mut body = json!({
        "model": args.model,
        "max_tokens": args.max_tokens,
        "messages": to_wire_messages(&args.messages, &args.system_prompt),
        "stream": true,
        // llama.cpp reports usage natively on the final chunk and tolerates
        // the OpenAI knob; a llama-server run behind a reverse proxy or a
        // different OpenAI-compatible backend may need it to emit usage at all.
        "stream_options": { "include_usage": true },
        // llama.cpp has no dedicated per-request reasoning switch; its only
        // lever is `chat_template_kwargs`, merged into the Jinja render vars.
        // Reasoning GGUFs conventionally gate their thinking channel on an
        // `enable_thinking` template var, so mapping the caller's preference
        // onto it makes the console's thinking toggle bite. A template that
        // doesn't reference the key ignores it — never breaks a non-reasoning
        // model (same optimistic contract as tools).
        "chat_template_kwargs": { "enable_thinking": args.enable_thinking },
    });
    let wire_tools = functions_to_wire(&args.tools);
    if !wire_tools.is_empty() {
        body["tools"] = Value::Array(wire_tools);
    }
    if let Some(rf) = &args.response_format {
        body["response_format"] = build_response_format(rf);
    }
    body
}

pub fn build_headers(cfg: &LlamacppConfig) -> Vec<(&'static str, String)> {
    let mut headers = vec![("content-type", "application/json".to_string())];
    if let Some(credential) = &cfg.credential_value {
        headers.push(("authorization", format!("Bearer {credential}")));
    }
    headers
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::content::ContentBlock;
    use llm_router::types::messages::{UserMessage, UserRoleTag};

    fn args() -> BodyArgs {
        BodyArgs {
            model: "qwen2.5-coder-7b-instruct".into(),
            max_tokens: 4096,
            system_prompt: "be brief".into(),
            messages: vec![AgentMessage::User(UserMessage {
                role: UserRoleTag::User,
                content: vec![ContentBlock::Text { text: "hi".into() }],
                timestamp: 1,
            })],
            tools: vec![],
            response_format: None,
            enable_thinking: false,
        }
    }

    fn tool() -> AgentFunction {
        AgentFunction {
            name: "agent::trigger".into(),
            description: "d".into(),
            parameters: serde_json::json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }
    }

    #[test]
    fn body_has_required_fields_and_documented_max_tokens_param() {
        let body = build_body(&args());
        assert_eq!(body["model"], "qwen2.5-coder-7b-instruct");
        assert_eq!(body["max_tokens"], 4096);
        assert!(
            body.get("max_completion_tokens").is_none(),
            "llama.cpp documents max_tokens, not the OpenAI replacement"
        );
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert!(body.get("tools").is_none(), "empty tools array omitted");
        assert!(body.get("response_format").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn thinking_preference_maps_to_enable_thinking_kwarg() {
        // Absent thinking (off) → enable_thinking:false; any level → true.
        // The console's toggle rides llama.cpp's only per-request lever.
        let off = build_body(&args());
        assert_eq!(off["chat_template_kwargs"]["enable_thinking"], false);

        let mut on = args();
        on.enable_thinking = true;
        assert_eq!(
            build_body(&on)["chat_template_kwargs"]["enable_thinking"],
            true
        );
    }

    #[test]
    fn tools_serialize_when_present() {
        let mut a = args();
        a.tools = vec![tool()];
        let body = build_body(&a);
        assert_eq!(body["tools"][0]["function"]["name"], "agent__trigger");
    }

    #[test]
    fn response_format_passes_schema_through_for_json_schema_mode() {
        let schema = serde_json::json!({ "type": "object", "properties": {} });
        let with_schema = build_response_format(&ResponseFormat {
            r#type: "json".into(),
            schema: Some(schema.clone()),
        });
        assert_eq!(with_schema["type"], "json_schema");
        assert_eq!(with_schema["schema"], schema);
        assert!(
            with_schema.get("json_schema").is_none(),
            "llama.cpp nests schema directly, not OpenAI's extra json_schema wrapper"
        );

        let without_schema = build_response_format(&ResponseFormat {
            r#type: "json".into(),
            schema: None,
        });
        assert_eq!(without_schema["type"], "json_object");

        let mut a = args();
        a.response_format = Some(ResponseFormat {
            r#type: "json".into(),
            schema: None,
        });
        assert_eq!(build_body(&a)["response_format"]["type"], "json_object");
    }

    #[test]
    fn headers_carry_bearer_auth_only_when_a_credential_is_configured() {
        let cfg = LlamacppConfig {
            credential_value: Some("sk-test".into()),
            model: "qwen2.5-coder-7b-instruct".into(),
            max_tokens: 4096,
            api_url: "http://127.0.0.1:8080/v1/chat/completions".into(),
        };
        let h = build_headers(&cfg);
        assert!(h.contains(&("authorization", "Bearer sk-test".to_string())));
        assert!(h.contains(&("content-type", "application/json".to_string())));

        let cfg = LlamacppConfig {
            credential_value: None,
            ..cfg
        };
        let h = build_headers(&cfg);
        assert!(
            !h.iter().any(|(k, _)| *k == "authorization"),
            "no credential configured → no Authorization header sent"
        );
    }
}

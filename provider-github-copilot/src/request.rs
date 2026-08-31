//! Full Chat Completions request assembly: body (messages, tools,
//! response_format) + headers. The Copilot API is OpenAI Chat
//! Completions-compatible but client-gated: every call must carry the
//! integration headers below or the gateway rejects it. `X-Initiator`
//! distinguishes agent-initiated turns per Copilot's billing convention.
use crate::config::CopilotConfig;
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use llm_router::types::router::ResponseFormat;
use serde_json::{json, Value};

/// Client-identity headers the Copilot gateway requires; sent on the token
/// exchange, discovery, and every chat call.
pub const INTEGRATION_ID: &str = "vscode-chat";
pub const EDITOR_VERSION: &str = "vscode/1.99.0";
pub const EDITOR_PLUGIN_VERSION: &str = "copilot-chat/0.26.0";
pub const USER_AGENT: &str = "GitHubCopilotChat/0.26.0";

/// The shared client-identity header set (no authorization).
pub fn client_headers() -> Vec<(&'static str, String)> {
    vec![
        ("copilot-integration-id", INTEGRATION_ID.to_string()),
        ("editor-version", EDITOR_VERSION.to_string()),
        ("editor-plugin-version", EDITOR_PLUGIN_VERSION.to_string()),
        ("user-agent", USER_AGENT.to_string()),
    ]
}

pub struct BodyArgs {
    /// The id the Copilot API expects — already stripped of the catalog prefix.
    pub model: String,
    pub max_tokens: u64,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<AgentFunction>,
    pub response_format: Option<ResponseFormat>,
    /// Whether the model declares strict `json_schema` support; without it a
    /// requested schema degrades to `json_object` (with a warning upstream).
    pub allow_json_schema: bool,
}

/// A requested schema maps to strict `json_schema` mode when the model
/// declares `structured_outputs`; otherwise — and when no schema was given —
/// plain `json_object` mode (the caller's prompt must mention JSON per the
/// OpenAI-compatible contract).
pub fn build_response_format(rf: &ResponseFormat, allow_json_schema: bool) -> Value {
    match &rf.schema {
        Some(schema) if allow_json_schema => json!({
            "type": "json_schema",
            "json_schema": {
                "name": "response",
                "strict": true,
                "schema": schema,
            }
        }),
        _ => json!({ "type": "json_object" }),
    }
}

pub fn build_body(args: &BodyArgs) -> Value {
    let mut body = json!({
        "model": args.model,
        "max_tokens": args.max_tokens,
        "messages": to_wire_messages(&args.messages, &args.system_prompt),
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    let wire_tools = functions_to_wire(&args.tools);
    if !wire_tools.is_empty() {
        body["tools"] = Value::Array(wire_tools);
    }
    if let Some(rf) = &args.response_format {
        body["response_format"] = build_response_format(rf, args.allow_json_schema);
    }
    body
}

pub fn build_headers(cfg: &CopilotConfig) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        ("authorization", format!("Bearer {}", cfg.bearer)),
        ("content-type", "application/json".to_string()),
        ("openai-intent", "conversation-panel".to_string()),
        // Agent-initiated turn: billed per Copilot's convention for
        // non-interactive requests, never misattributed as a user keystroke.
        ("x-initiator", "agent".to_string()),
    ];
    headers.extend(client_headers());
    headers
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::content::ContentBlock;
    use llm_router::types::messages::{UserMessage, UserRoleTag};

    fn args() -> BodyArgs {
        BodyArgs {
            model: "gpt-5.2".into(),
            max_tokens: 4096,
            system_prompt: "be brief".into(),
            messages: vec![AgentMessage::User(UserMessage {
                role: UserRoleTag::User,
                content: vec![ContentBlock::Text { text: "hi".into() }],
                timestamp: 1,
            })],
            tools: vec![],
            response_format: None,
            allow_json_schema: true,
        }
    }

    #[test]
    fn body_has_required_fields_and_streaming() {
        let body = build_body(&args());
        assert_eq!(body["model"], "gpt-5.2");
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert!(body.get("tools").is_none(), "empty tools array omitted");
        assert!(body.get("response_format").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn tools_serialize_when_present() {
        let mut a = args();
        a.tools = vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "d".into(),
            parameters: serde_json::json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }];
        let body = build_body(&a);
        assert_eq!(body["tools"][0]["function"]["name"], "agent__trigger");
    }

    #[test]
    fn schema_maps_to_strict_json_schema_when_supported() {
        let schema = serde_json::json!({ "type": "object", "additionalProperties": false });
        let rf = ResponseFormat {
            r#type: "json".into(),
            schema: Some(schema.clone()),
        };
        let wire = build_response_format(&rf, true);
        assert_eq!(wire["type"], "json_schema");
        assert_eq!(wire["json_schema"]["strict"], true);
        let wire = build_response_format(&rf, false);
        assert_eq!(wire["type"], "json_object");
        let rf = ResponseFormat {
            r#type: "json".into(),
            schema: None,
        };
        assert_eq!(build_response_format(&rf, true)["type"], "json_object");
    }

    #[test]
    fn headers_carry_bearer_and_client_identity() {
        let cfg = CopilotConfig {
            bearer: "tid=test".into(),
            model: "gpt-5.2".into(),
            max_tokens: 4096,
            api_url: "https://api.githubcopilot.com/chat/completions".into(),
        };
        let h = build_headers(&cfg);
        assert!(h.contains(&("authorization", "Bearer tid=test".to_string())));
        assert!(h.contains(&("copilot-integration-id", INTEGRATION_ID.to_string())));
        assert!(h.contains(&("editor-version", EDITOR_VERSION.to_string())));
        assert!(h.contains(&("x-initiator", "agent".to_string())));
        assert!(h.contains(&("user-agent", USER_AGENT.to_string())));
    }
}

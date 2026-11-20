//! Interactions request assembly: body + headers.
//!
//! Stateless: `store: false` and the whole history every request. The router
//! and harness own and rewrite the history (compaction, reordering, provider
//! switches), which a server-held `previous_interaction_id` chain could not
//! follow.
use crate::config::GeminiConfig;
use crate::errors::invalid_request;
use crate::wire::messages::to_wire_steps;
use crate::wire::tools::{functions_to_wire, response_format_to_wire};
use iii_sdk::errors::Error;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use llm_router::types::router::ResponseFormat;
use serde_json::{Value, json};

pub struct BodyArgs {
    pub model: String,
    pub max_tokens: u64,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<AgentFunction>,
    /// Pre-resolved wire `thinking_level`; None omits the field.
    pub thinking_level: Option<&'static str>,
    /// Send `thinking_summaries: "auto"` (thinking models only).
    pub thinking_summaries: bool,
    pub response_format: Option<ResponseFormat>,
}

/// An input that ends in `model_output` or `thought` is a 400: "Requests
/// ending with a model turn are not supported." The router sends such a
/// history after an aborted stream (a dead partial). A trailing
/// `function_call` cannot occur: the orphan rule always puts a result after
/// it. Only the tail goes, so the cached prefix is untouched. True when it
/// dropped.
pub fn drop_trailing_model_steps(steps: &mut Vec<Value>) -> bool {
    let mut dropped = false;
    while matches!(
        steps.last().and_then(|s| s["type"].as_str()),
        Some("model_output" | "thought")
    ) {
        steps.pop();
        dropped = true;
    }
    dropped
}

/// Not sent: `temperature`/`top_p`/`top_k` (deprecated), `tool_choice`
/// (default auto), `safety_settings`, `service_tier`. An unknown top-level
/// field is a 400, so the key set is pinned by a test.
pub fn build_body(args: &BodyArgs, warnings: &mut Vec<String>) -> Result<Value, Error> {
    let mut input = to_wire_steps(&args.messages, warnings);
    if drop_trailing_model_steps(&mut input) {
        warnings.push(
            "trailing partial assistant message dropped: the conversation must end with a user turn"
                .to_string(),
        );
    }
    if input.is_empty() {
        return Err(invalid_request(
            "provider-gemini: no input left to send (the conversation has no user turn)",
        ));
    }
    // A window that opens on a model turn (compaction cuts at an assistant
    // boundary) is a 400 when that turn holds a call: "function call turn
    // comes immediately after a user turn". Constant bytes keep the prefix
    // cacheable.
    if input[0]["type"] != "user_input" {
        input.insert(
            0,
            json!({ "type": "user_input", "content": [{ "type": "text", "text": "(continued from earlier in the conversation)" }] }),
        );
    }
    let mut generation_config = json!({ "max_output_tokens": args.max_tokens });
    if let Some(level) = args.thinking_level {
        generation_config["thinking_level"] = json!(level);
    }
    if args.thinking_summaries {
        generation_config["thinking_summaries"] = json!("auto");
    }
    let mut body = json!({
        "model": args.model,
        "input": input,
        "generation_config": generation_config,
        "stream": true,
        "store": false,
    });
    if !args.system_prompt.is_empty() {
        body["system_instruction"] = json!(args.system_prompt);
    }
    let tools = functions_to_wire(&args.tools);
    if !tools.is_empty() {
        body["tools"] = Value::Array(tools);
    }
    if let Some(rf) = &args.response_format {
        body["response_format"] = response_format_to_wire(rf);
    }
    Ok(body)
}

pub fn build_headers(cfg: &GeminiConfig) -> Vec<(&'static str, String)> {
    vec![
        cfg.auth.clone(),
        ("content-type", "application/json".to_string()),
        ("accept", "text/event-stream".to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::content::ContentBlock;
    use llm_router::types::events::StopReason;
    use llm_router::types::messages::{
        AssistantMessage, AssistantRoleTag, FunctionResultMessage, FunctionResultRoleTag,
        UserMessage, UserRoleTag,
    };

    fn user(t: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![ContentBlock::Text { text: t.into() }],
            timestamp: 1,
        })
    }

    fn assistant(content: Vec<ContentBlock>) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content,
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            warnings: None,
            usage: None,
            model: "gemini-3.8-flash".into(),
            provider: "gemini".into(),
            timestamp: 1,
        })
    }

    fn args(messages: Vec<AgentMessage>) -> BodyArgs {
        BodyArgs {
            model: "gemini-3.8-flash".into(),
            max_tokens: 1024,
            system_prompt: String::new(),
            messages,
            tools: vec![],
            thinking_level: None,
            thinking_summaries: false,
            response_format: None,
        }
    }

    fn keys(v: &Value) -> Vec<&str> {
        let mut k: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
        k.sort();
        k
    }

    #[test]
    fn full_key_set() {
        let mut a = args(vec![user("hi")]);
        a.system_prompt = "be brief".into();
        a.tools = vec![AgentFunction {
            name: "f".into(),
            description: "d".into(),
            parameters: json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }];
        a.thinking_level = Some("high");
        a.thinking_summaries = true;
        a.response_format = Some(ResponseFormat {
            r#type: "json".into(),
            schema: None,
        });
        let body = build_body(&a, &mut Vec::new()).unwrap();
        assert_eq!(
            keys(&body),
            [
                "generation_config",
                "input",
                "model",
                "response_format",
                "store",
                "stream",
                "system_instruction",
                "tools"
            ]
        );
        assert_eq!(
            body["generation_config"],
            json!({ "max_output_tokens": 1024, "thinking_level": "high", "thinking_summaries": "auto" })
        );
        assert_eq!(body["system_instruction"], "be brief");
        assert_eq!(body["store"], false);
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn optional_fields_are_omitted() {
        let body = build_body(&args(vec![user("hi")]), &mut Vec::new()).unwrap();
        assert_eq!(
            keys(&body),
            ["generation_config", "input", "model", "store", "stream"]
        );
        assert_eq!(
            body["generation_config"],
            json!({ "max_output_tokens": 1024 })
        );
    }

    #[test]
    fn trailing_model_steps_are_dropped_with_a_warning() {
        let mut warnings = Vec::new();
        let body = build_body(
            &args(vec![
                user("hi"),
                assistant(vec![
                    ContentBlock::Thinking {
                        text: String::new(),
                        signature: Some("s".into()),
                    },
                    ContentBlock::Text {
                        text: "partial".into(),
                    },
                ]),
            ]),
            &mut warnings,
        )
        .unwrap();
        assert_eq!(body["input"].as_array().unwrap().len(), 1);
        assert_eq!(body["input"][0]["type"], "user_input");
        assert_eq!(
            warnings,
            [
                "trailing partial assistant message dropped: the conversation must end with a user turn"
            ]
        );
    }

    #[test]
    fn a_window_opening_on_a_model_turn_gets_a_leading_user_input() {
        let body = build_body(
            &args(vec![
                assistant(vec![
                    ContentBlock::Thinking {
                        text: String::new(),
                        signature: Some("s".into()),
                    },
                    ContentBlock::FunctionCall {
                        id: "c1".into(),
                        function_id: "f".into(),
                        arguments: json!({}),
                    },
                ]),
                AgentMessage::FunctionResult(FunctionResultMessage {
                    role: FunctionResultRoleTag::FunctionResult,
                    function_call_id: "c1".into(),
                    function_id: "f".into(),
                    content: vec![ContentBlock::Text { text: "ok".into() }],
                    details: json!({}),
                    is_error: false,
                    timestamp: 1,
                }),
            ]),
            &mut Vec::new(),
        )
        .unwrap();
        let types: Vec<&str> = body["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["type"].as_str().unwrap())
            .collect();
        assert_eq!(
            types,
            ["user_input", "thought", "function_call", "function_result"]
        );
        // A history that already starts with the user is left alone.
        let body = build_body(&args(vec![user("hi")]), &mut Vec::new()).unwrap();
        assert_eq!(body["input"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn nothing_left_is_invalid_request() {
        let err = build_body(
            &args(vec![assistant(vec![ContentBlock::Text {
                text: "x".into(),
            }])]),
            &mut Vec::new(),
        )
        .unwrap_err();
        match err {
            Error::Remote { code, .. } => assert_eq!(code, "provider/invalid_request"),
            other => panic!("want Remote, got {other:?}"),
        }
    }

    #[test]
    fn headers_carry_the_auth_pair() {
        let cfg = GeminiConfig {
            auth: ("x-goog-api-key", "k".into()),
            model: "m".into(),
            max_tokens: 1,
            api_url: "https://h/v1beta/interactions".into(),
        };
        let h = build_headers(&cfg);
        assert_eq!(h[0], ("x-goog-api-key", "k".to_string()));
        assert!(h.contains(&("accept", "text/event-stream".to_string())));
    }
}

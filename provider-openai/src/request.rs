//! Request assembly for the Responses API and the legacy Chat Completions
//! compatibility path.
use crate::config::{ApiMode, OpenaiConfig};
use crate::wire::messages::{to_responses_input, to_responses_input_sections, to_wire_messages};
use crate::wire::tools::{functions_to_responses_wire, functions_to_wire};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use llm_router::types::router::{PromptSection, ResponseFormat};
use serde_json::{json, Value};

pub struct BodyArgs {
    pub model: String,
    /// `None` omits the output-token parameter: the model's own maximum applies.
    pub max_tokens: Option<u64>,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<AgentFunction>,
    /// Pre-resolved effort string (Task 6); None omits the param.
    pub reasoning_effort: Option<&'static str>,
    pub response_format: Option<ResponseFormat>,
    pub prompt_cache_key: Option<String>,
    /// Ordered sections behind `system_prompt`; used on the Responses path
    /// only when `explicit_cache_breakpoints` is set (GPT-5.6 and later).
    pub system_sections: Option<Vec<PromptSection>>,
    /// The model takes `prompt_cache_breakpoint` marks (see
    /// [`supports_explicit_cache_breakpoints`]).
    pub explicit_cache_breakpoints: bool,
    /// `prompt_cache_retention` to send (`24h` keeps an entry up to a day
    /// instead of 5-10 minutes; no extra write charge on these models). `None`
    /// omits the field. See [`supports_extended_cache_retention`].
    pub cache_retention: Option<&'static str>,
}

const CACHE_RETENTION_ENV: &str = "PROVIDER_OPENAI_CACHE_RETENTION";

/// Unset or `24h` = extended retention; `in_memory` = the short default;
/// `off` = do not send the field at all.
pub fn cache_retention() -> Option<&'static str> {
    match std::env::var(CACHE_RETENTION_ENV).as_deref() {
        Ok("off") => None,
        Ok("in_memory") => Some("in_memory"),
        _ => Some("24h"),
    }
}

/// Models that document `prompt_cache_retention: "24h"` (the docs' list, plus
/// their dated snapshots), official endpoint only. GPT-5.6 and later replace it
/// with `prompt_cache_options.ttl`, whose single value (30m) is already the
/// default, so nothing is sent there.
pub fn supports_extended_cache_retention(model: &str, api_url: &str) -> bool {
    const EXTENDED: &[&str] = &[
        "gpt-5.5",
        "gpt-5.5-pro",
        "gpt-5.4",
        "gpt-5.2",
        "gpt-5.1-codex-max",
        "gpt-5.1",
        "gpt-5.1-codex",
        "gpt-5.1-codex-mini",
        "gpt-5.1-chat-latest",
        "gpt-5",
        "gpt-5-codex",
        "gpt-4.1",
    ];
    let official = reqwest::Url::parse(api_url)
        .ok()
        .and_then(|url| url.host_str().map(|h| h == "api.openai.com"))
        .unwrap_or(false);
    // strip a dated snapshot suffix like `-2026-03-05`
    let base = match model.len().checked_sub(11) {
        Some(cut)
            if model.as_bytes()[cut] == b'-'
                && model[cut + 1..].len() == 10
                && model[cut + 1..].bytes().enumerate().all(|(i, b)| {
                    if i == 4 || i == 7 {
                        b == b'-'
                    } else {
                        b.is_ascii_digit()
                    }
                }) =>
        {
            &model[..cut]
        }
        _ => model,
    };
    official && EXTENDED.contains(&base)
}

/// Explicit cache breakpoints exist on GPT-5.6 and later, on the official
/// endpoint only: a gateway that speaks Responses may reject the field, and
/// earlier models document it as unsupported. `gpt-<major>[.<minor>]-…`
/// compares as a (major, minor) pair, so `gpt-5.10` and `gpt-6-astra` count.
pub fn supports_explicit_cache_breakpoints(model: &str, api_url: &str) -> bool {
    let official = reqwest::Url::parse(api_url)
        .ok()
        .and_then(|url| url.host_str().map(|h| h == "api.openai.com"))
        .unwrap_or(false);
    let Some(rest) = model.strip_prefix("gpt-") else {
        return false;
    };
    let version: Vec<u32> = rest
        .split('-')
        .next()
        .unwrap_or_default()
        .split('.')
        .map(|part| part.parse::<u32>().unwrap_or(u32::MAX))
        .collect();
    let (major, minor) = (
        version.first().copied().unwrap_or(u32::MAX),
        version.get(1).copied().unwrap_or(0),
    );
    official && major != u32::MAX && minor != u32::MAX && (major, minor) >= (5, 6)
}

/// `ResponseFormat { type: "json", schema? }` → native OpenAI knob.
/// With a schema: strict json_schema mode (constrained decoding — a schema
/// that violates OpenAI's strict-mode rules 400s as `permanent`, which is
/// correct: retrying cannot fix the schema). Without: json_object mode
/// (OpenAI requires the word "JSON" somewhere in the messages — the
/// caller's contract per spec § Model capabilities).
pub fn build_chat_response_format(rf: &ResponseFormat) -> Value {
    match &rf.schema {
        Some(schema) => json!({
            "type": "json_schema",
            "json_schema": { "name": "response", "strict": true, "schema": schema }
        }),
        None => json!({ "type": "json_object" }),
    }
}

/// `max_completion_tokens`, not the deprecated `max_tokens`: the o-series
/// and gpt-5 families reject the old param. Reasoning tokens count toward
/// it; the router-clamped budget leaves ample room. No `temperature`: the
/// API default applies (reasoning models reject non-default values).
fn build_chat_body(args: &BodyArgs) -> Value {
    let mut body = json!({
        "model": args.model,
        "messages": to_wire_messages(&args.messages, &args.system_prompt),
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    if let Some(max_tokens) = args.max_tokens {
        body["max_completion_tokens"] = json!(max_tokens);
    }
    let wire_tools = functions_to_wire(&args.tools);
    if !wire_tools.is_empty() {
        body["tools"] = Value::Array(wire_tools);
    }
    if let Some(effort) = args.reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }
    if let Some(rf) = &args.response_format {
        body["response_format"] = build_chat_response_format(rf);
    }
    if let Some(key) = &args.prompt_cache_key {
        body["prompt_cache_key"] = json!(key);
    }
    if let Some(retention) = args.cache_retention {
        body["prompt_cache_retention"] = json!(retention);
    }
    body
}

fn build_responses_text(rf: &ResponseFormat) -> Value {
    let format = match &rf.schema {
        Some(schema) => json!({
            "type": "json_schema",
            "name": "response",
            "strict": true,
            "schema": schema,
        }),
        None => json!({ "type": "json_object" }),
    };
    json!({ "format": format })
}

fn build_responses_body(args: &BodyArgs) -> Value {
    let input = match args.system_sections.as_deref() {
        Some(sections) if args.explicit_cache_breakpoints && !sections.is_empty() => {
            to_responses_input_sections(&args.messages, sections)
        }
        _ => to_responses_input(&args.messages, &args.system_prompt),
    };
    let mut body = json!({
        "model": args.model,
        "input": input,
        "stream": true,
        "store": false,
    });
    if let Some(max_tokens) = args.max_tokens {
        body["max_output_tokens"] = json!(max_tokens);
    }
    let wire_tools = functions_to_responses_wire(&args.tools);
    if !wire_tools.is_empty() {
        body["tools"] = Value::Array(wire_tools);
    }
    if let Some(effort) = args.reasoning_effort {
        body["reasoning"] = json!({ "effort": effort });
    }
    if let Some(response_format) = &args.response_format {
        body["text"] = build_responses_text(response_format);
    }
    if let Some(key) = &args.prompt_cache_key {
        body["prompt_cache_key"] = json!(key);
    }
    if let Some(retention) = args.cache_retention {
        body["prompt_cache_retention"] = json!(retention);
    }
    body
}

pub fn build_body(args: &BodyArgs, api_mode: ApiMode) -> Value {
    match api_mode {
        ApiMode::Responses => build_responses_body(args),
        ApiMode::ChatCompletions => build_chat_body(args),
    }
}

pub fn build_headers(cfg: &OpenaiConfig) -> Vec<(&'static str, String)> {
    vec![
        ("authorization", format!("Bearer {}", cfg.credential_value)),
        ("content-type", "application/json".to_string()),
        ("accept", "text/event-stream".to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::content::ContentBlock;
    use llm_router::types::messages::{UserMessage, UserRoleTag};

    fn args() -> BodyArgs {
        BodyArgs {
            model: "gpt-5.2".into(),
            max_tokens: Some(4096),
            system_prompt: "be brief".into(),
            messages: vec![AgentMessage::User(UserMessage {
                role: UserRoleTag::User,
                content: vec![ContentBlock::Text { text: "hi".into() }],
                timestamp: 1,
            })],
            tools: vec![],
            reasoning_effort: None,
            response_format: None,
            prompt_cache_key: None,
            system_sections: None,
            explicit_cache_breakpoints: false,
            cache_retention: None,
        }
    }

    fn sections() -> Vec<PromptSection> {
        vec![
            PromptSection {
                text: "stable profile".into(),
                cache_boundary: true,
            },
            PromptSection {
                text: "Your session id is s_1.".into(),
                cache_boundary: false,
            },
        ]
    }

    #[test]
    fn sectioned_responses_body_marks_the_stable_developer_block() {
        let mut a = args();
        a.system_sections = Some(sections());
        a.explicit_cache_breakpoints = true;
        let body = build_body(&a, ApiMode::Responses);
        let input = body["input"].as_array().unwrap();
        assert_eq!(input[0]["role"], "developer");
        assert_eq!(input[0]["content"][0]["text"], "stable profile");
        assert_eq!(
            input[0]["content"][0]["prompt_cache_breakpoint"]["mode"],
            "explicit"
        );
        assert_eq!(input[1]["role"], "developer");
        assert!(input[1]["content"][0]
            .get("prompt_cache_breakpoint")
            .is_none());
        assert_eq!(input[2]["role"], "user");
        // implicit caching stays on: no mode override on the request
        assert!(body.get("prompt_cache_options").is_none());
        // Chat Completions and pre-5.6 models keep the flat system message
        assert_eq!(
            build_body(&a, ApiMode::ChatCompletions)["messages"][0]["role"],
            "system"
        );
        a.explicit_cache_breakpoints = false;
        let flat = build_body(&a, ApiMode::Responses);
        assert_eq!(flat["input"][0]["role"], "system");
        assert_eq!(flat["input"][0]["content"][0]["text"], "be brief");
        assert!(flat["input"][0]["content"][0]
            .get("prompt_cache_breakpoint")
            .is_none());
    }

    #[test]
    fn explicit_breakpoints_only_for_gpt_5_6_and_later_on_the_official_endpoint() {
        let official = crate::config::DEFAULT_API_URL;
        for model in [
            "gpt-5.6",
            "gpt-5.6-sol",
            "gpt-5.10-x",
            "gpt-6-astra",
            "gpt-7",
        ] {
            assert!(
                supports_explicit_cache_breakpoints(model, official),
                "{model}"
            );
        }
        for model in ["gpt-5.5", "gpt-5-mini", "gpt-5.2", "gpt-4.1", "o3", "gpt-x"] {
            assert!(
                !supports_explicit_cache_breakpoints(model, official),
                "{model}"
            );
        }
        assert!(!supports_explicit_cache_breakpoints(
            "gpt-5.6",
            "https://gateway.example.com/v1/responses"
        ));
    }

    #[test]
    fn body_has_required_fields_and_stream_options() {
        let body = build_body(&args(), ApiMode::ChatCompletions);
        assert_eq!(body["model"], "gpt-5.2");
        assert_eq!(body["max_completion_tokens"], 4096);
        assert!(
            body.get("max_tokens").is_none(),
            "deprecated param never sent"
        );
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert!(body.get("tools").is_none(), "empty tools array omitted");
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("response_format").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn reasoning_effort_and_tools_serialize_when_present() {
        let mut a = args();
        a.reasoning_effort = Some("high");
        a.tools = vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "d".into(),
            parameters: serde_json::json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }];
        let body = build_body(&a, ApiMode::ChatCompletions);
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["tools"][0]["function"]["name"], "agent__trigger");
    }

    #[test]
    fn response_format_maps_to_json_schema_or_json_object() {
        let with_schema = build_chat_response_format(&ResponseFormat {
            r#type: "json".into(),
            schema: Some(serde_json::json!({ "type": "object", "additionalProperties": false })),
        });
        assert_eq!(with_schema["type"], "json_schema");
        assert_eq!(with_schema["json_schema"]["name"], "response");
        assert_eq!(with_schema["json_schema"]["strict"], true);
        assert_eq!(with_schema["json_schema"]["schema"]["type"], "object");

        let without = build_chat_response_format(&ResponseFormat {
            r#type: "json".into(),
            schema: None,
        });
        assert_eq!(without["type"], "json_object");

        let mut a = args();
        a.response_format = Some(ResponseFormat {
            r#type: "json".into(),
            schema: None,
        });
        assert_eq!(
            build_body(&a, ApiMode::ChatCompletions)["response_format"]["type"],
            "json_object"
        );
    }

    #[test]
    fn responses_body_uses_items_tools_and_reasoning_object() {
        let mut a = args();
        a.reasoning_effort = Some("high");
        a.tools = vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "d".into(),
            parameters: json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }];
        a.response_format = Some(ResponseFormat {
            r#type: "json".into(),
            schema: Some(json!({ "type": "object", "additionalProperties": false })),
        });
        let body = build_body(&a, ApiMode::Responses);
        assert_eq!(body["model"], "gpt-5.2");
        assert_eq!(body["max_output_tokens"], 4096);
        assert_eq!(body["store"], false);
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["tools"][0]["name"], "agent__trigger");
        assert!(body["tools"][0].get("function").is_none());
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["text"]["format"]["type"], "json_schema");
        assert!(body.get("messages").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn uncapped_requests_omit_the_output_token_parameter_in_both_modes() {
        let mut a = args();
        a.max_tokens = None;
        let chat = build_body(&a, ApiMode::ChatCompletions);
        assert!(chat.get("max_completion_tokens").is_none());
        assert!(chat.get("max_tokens").is_none());
        let responses = build_body(&a, ApiMode::Responses);
        assert!(responses.get("max_output_tokens").is_none());
    }

    #[test]
    fn extended_retention_only_on_the_documented_models_and_endpoint() {
        let official = crate::config::DEFAULT_API_URL;
        for model in [
            "gpt-5",
            "gpt-5.5",
            "gpt-5.4-2026-03-05",
            "gpt-4.1",
            "gpt-5.1-codex-mini",
        ] {
            assert!(
                supports_extended_cache_retention(model, official),
                "{model}"
            );
        }
        for model in [
            "gpt-5.6",
            "gpt-5.6-sol",
            "gpt-6-astra",
            "gpt-5-mini",
            "gpt-5-nano",
            "o3",
        ] {
            assert!(
                !supports_extended_cache_retention(model, official),
                "{model}"
            );
        }
        assert!(!supports_extended_cache_retention(
            "gpt-5",
            "https://gateway.example.com/v1/responses"
        ));
        let mut a = args();
        a.cache_retention = Some("24h");
        assert_eq!(
            build_body(&a, ApiMode::Responses)["prompt_cache_retention"],
            "24h"
        );
        assert_eq!(
            build_body(&a, ApiMode::ChatCompletions)["prompt_cache_retention"],
            "24h"
        );
        a.cache_retention = None;
        assert!(build_body(&a, ApiMode::Responses)
            .get("prompt_cache_retention")
            .is_none());
    }

    #[test]
    fn both_api_modes_send_the_prompt_cache_key() {
        let mut a = args();
        a.prompt_cache_key = Some("stable-session-key".into());
        assert_eq!(
            build_body(&a, ApiMode::Responses)["prompt_cache_key"],
            "stable-session-key"
        );
        assert_eq!(
            build_body(&a, ApiMode::ChatCompletions)["prompt_cache_key"],
            "stable-session-key"
        );
    }

    #[test]
    fn headers_carry_bearer_auth() {
        let cfg = OpenaiConfig {
            credential_value: "sk-test".into(),
            model: "gpt-5.2".into(),
            max_tokens: Some(4096),
            api_url: "https://api.openai.com/v1/chat/completions".into(),
            api_mode: ApiMode::ChatCompletions,
        };
        let h = build_headers(&cfg);
        assert!(h.contains(&("authorization", "Bearer sk-test".to_string())));
        assert!(h.contains(&("content-type", "application/json".to_string())));
    }
}

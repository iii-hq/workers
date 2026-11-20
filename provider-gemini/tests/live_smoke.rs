//! End-to-end checks against the live Interactions API through the real
//! request builder, transport and SSE decoder. Needs a key, so ignored by
//! default:
//!
//!   GEMINI_API_KEY=… cargo test --test live_smoke -- --ignored --nocapture
//!
//! `GEMINI_SMOKE_MODEL` picks the model (default `gemini-3.8-flash`). Five
//! short requests in all.

use llm_router::types::content::ContentBlock;
use llm_router::types::events::{AssistantMessageEvent, StopReason};
use llm_router::types::messages::{
    AgentMessage, AssistantMessage, AssistantRoleTag, FunctionResultMessage, FunctionResultRoleTag,
    UserMessage, UserRoleTag,
};
use llm_router::types::model::AgentFunction;
use provider_gemini::config::{DEFAULT_API_URL, GeminiConfig};
use provider_gemini::request::{BodyArgs, build_body, build_headers};
use provider_gemini::upstream::{HEARTBEAT, UpstreamArgs, spawn_upstream};
use serde_json::{Value, json};
use std::time::Duration;

fn model() -> String {
    std::env::var("GEMINI_SMOKE_MODEL").unwrap_or_else(|_| "gemini-3.8-flash".into())
}

fn user(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage {
        role: UserRoleTag::User,
        content: vec![ContentBlock::Text { text: text.into() }],
        timestamp: 1,
    })
}

fn result(call_id: &str, function_id: &str, text: &str) -> AgentMessage {
    AgentMessage::FunctionResult(FunctionResultMessage {
        role: FunctionResultRoleTag::FunctionResult,
        function_call_id: call_id.into(),
        function_id: function_id.into(),
        content: vec![ContentBlock::Text { text: text.into() }],
        details: json!({}),
        is_error: false,
        timestamp: 3,
    })
}

/// A copy of `harness/src/policy.rs::agent_call_schema(None)`.
fn agent_call_tool() -> AgentFunction {
    AgentFunction {
        name: "agent_call".into(),
        description: "Call any allowed iii function by id. Discover task-capability IDs with \
            directory::search_functions (its `capabilities` entries always in English); when \
            `<discovery_assist>` is present, follow it. Fall back to engine::functions::list only \
            when directory::search_functions is unavailable. Fixed-prefix inventory checks for a \
            documented surface or after an install still apply. Before first use, get selected \
            contracts with engine::functions::info. Ordinary calls in one assistant response \
            execute sequentially in content order. For independent same-function work, use that \
            function's supported bulk input."
            .into(),
        parameters: json!({
            "type": "object",
            "properties": {
                "function": { "type": "string", "description": "Target iii function id." },
                "description": {
                    "type": "string",
                    "minLength": 1,
                    "maxLength": 120,
                    "description": "Short user-facing description of the action, in the response \
                        language named in the session context (else the language of the user's \
                        own prose); never inferred from names, paths or locale. Describe the \
                        work, not the function id."
                },
                "payload": { "type": "object", "description": "Arguments for the target function." }
            },
            "required": ["function", "description"]
        }),
        label: None,
        execution_mode: Some("sequential".into()),
    }
}

fn weather_tool() -> AgentFunction {
    AgentFunction {
        name: "get_weather".into(),
        description: "Current weather for one city.".into(),
        parameters: json!({
            "type": "object",
            "properties": { "city": { "type": "string" } },
            "required": ["city"]
        }),
        label: None,
        execution_mode: None,
    }
}

/// One streamed turn through the production path; panics unless it ends in
/// `Done`.
async fn turn(messages: Vec<AgentMessage>, tools: Vec<AgentFunction>) -> AssistantMessage {
    let key = std::env::var("GEMINI_API_KEY").expect("set GEMINI_API_KEY");
    let cfg = GeminiConfig {
        auth: ("x-goog-api-key", key.trim().to_string()),
        model: model(),
        max_tokens: 8_192,
        api_url: DEFAULT_API_URL.into(),
    };
    let mut warnings = Vec::new();
    let body = build_body(
        &BodyArgs {
            model: cfg.model.clone(),
            max_tokens: cfg.max_tokens,
            system_prompt: "You are a terse assistant.".into(),
            messages,
            tools,
            thinking_level: Some("low"),
            thinking_summaries: true,
            response_format: None,
        },
        &mut warnings,
    )
    .expect("body builds");
    let mut rx = spawn_upstream(
        reqwest::Client::new(),
        UpstreamArgs {
            api_url: cfg.api_url.clone(),
            model: cfg.model.clone(),
            body,
            headers: build_headers(&cfg),
            warnings,
            first_token_timeout: Duration::from_secs(120),
            heartbeat: HEARTBEAT,
        },
    );
    let mut last = None;
    while let Some(ev) = rx.recv().await {
        last = Some(ev);
    }
    match last {
        Some(AssistantMessageEvent::Done { message }) => message,
        Some(AssistantMessageEvent::Error { error }) => panic!(
            "request failed ({:?}): {}",
            error.error_kind,
            error.error_message.unwrap_or_default()
        ),
        other => panic!("no terminal frame: {other:?}"),
    }
}

fn calls(m: &AssistantMessage) -> Vec<(String, String, Value)> {
    m.content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::FunctionCall {
                id,
                function_id,
                arguments,
            } => Some((id.clone(), function_id.clone(), arguments.clone())),
            _ => None,
        })
        .collect()
}

fn text_of(m: &AssistantMessage) -> String {
    m.content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
#[ignore = "needs GEMINI_API_KEY and the network"]
async fn text_turn() {
    let m = turn(vec![user("Reply with the single word: pong")], vec![]).await;
    println!("text turn: {:?} {:?}", m.stop_reason, text_of(&m));
    assert_eq!(m.stop_reason, StopReason::End);
    assert!(text_of(&m).to_lowercase().contains("pong"));
    let usage = m.usage.expect("usage");
    assert!(usage.input.unwrap_or(0) > 0 && usage.output.unwrap_or(0) > 0);
}

#[tokio::test]
#[ignore = "needs GEMINI_API_KEY and the network"]
async fn agent_call_tool_loop() {
    let mut history = vec![user(
        "Use agent_call to call the iii function math::add with payload {\"a\": 2, \"b\": 3}, \
         then tell me the result in one short sentence.",
    )];
    let first = turn(history.clone(), vec![agent_call_tool()]).await;
    assert_eq!(first.stop_reason, StopReason::FunctionCall, "{first:?}");
    let (id, function_id, arguments) = calls(&first).remove(0);
    println!("call: {function_id} {arguments}");
    assert_eq!(function_id, "agent_call");
    assert_eq!(arguments["function"], "math::add");
    history.push(AgentMessage::Assistant(first));
    // Under agent_call exposure the result names the unwrapped target.
    history.push(result(&id, "math::add", "5"));
    let second = turn(history, vec![agent_call_tool()]).await;
    println!("answer: {:?}", text_of(&second));
    assert!(text_of(&second).contains('5'), "{second:?}");
}

/// Tripwire for the placeholder thought: a tool call from another provider in
/// the current turn has no thought step, which the API rejects unless the
/// placeholder signature is accepted.
#[tokio::test]
#[ignore = "needs GEMINI_API_KEY and the network"]
async fn foreign_call_in_the_current_turn_is_accepted() {
    let foreign = AgentMessage::Assistant(AssistantMessage {
        role: AssistantRoleTag::Assistant,
        content: vec![
            ContentBlock::Text {
                text: "I'll check.".into(),
            },
            ContentBlock::FunctionCall {
                id: "toolu_01abc".into(),
                function_id: "get_weather".into(),
                arguments: json!({ "city": "Lisbon" }),
            },
        ],
        stop_reason: StopReason::FunctionCall,
        native_stop_reason: None,
        error_message: None,
        error_kind: None,
        warnings: None,
        usage: None,
        model: "claude-test".into(),
        provider: "anthropic".into(),
        timestamp: 2,
    });
    let m = turn(
        vec![
            user("What is the weather in Lisbon? Use the tool."),
            foreign,
            result("toolu_01abc", "get_weather", "18 C, sunny"),
        ],
        vec![weather_tool()],
    )
    .await;
    println!("after foreign call: {:?}", text_of(&m));
    assert!(text_of(&m).contains("18"), "{m:?}");
}

/// A compacted window can open on an assistant message whose first step is a
/// call; the API rejects that unless a user turn comes first.
#[tokio::test]
#[ignore = "needs GEMINI_API_KEY and the network"]
async fn window_opening_on_a_call_is_accepted() {
    let opening = AgentMessage::Assistant(AssistantMessage {
        role: AssistantRoleTag::Assistant,
        content: vec![ContentBlock::FunctionCall {
            id: "toolu_02abc".into(),
            function_id: "get_weather".into(),
            arguments: json!({ "city": "Porto" }),
        }],
        stop_reason: StopReason::FunctionCall,
        native_stop_reason: None,
        error_message: None,
        error_kind: None,
        warnings: None,
        usage: None,
        model: "claude-test".into(),
        provider: "anthropic".into(),
        timestamp: 2,
    });
    let m = turn(
        vec![
            opening,
            result("toolu_02abc", "get_weather", "21 C, cloudy"),
            user("Tell me the temperature you got, in one short sentence."),
        ],
        vec![weather_tool()],
    )
    .await;
    println!("after leading call: {:?}", text_of(&m));
    assert!(text_of(&m).contains("21"), "{m:?}");
}

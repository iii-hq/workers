//! Live check of `count_tokens` against what Moonshot bills (MOT-5566).
//!
//! Moonshot documents its estimator with only `model` and `messages`, so it may
//! skip `tools` and `response_format` while the chat route bills them. This
//! sends each shape twice: the chat body `request::build_body` builds (one
//! output token, not streamed) for `usage.prompt_tokens`, and the same request
//! through `count_tokens::count`. Needs a key, so it is ignored by default:
//!
//!   MOONSHOT_API_KEY=… cargo test --test live_count_probe -- --ignored --nocapture
//!
//! `KIMI_PROBE_MODEL` picks the model (a thinking model shows whether the
//! estimator counts the thinking prefix). About 18 requests, one output token
//! each.

use llm_router::types::content::ContentBlock;
use llm_router::types::events::StopReason;
use llm_router::types::messages::{
    AgentMessage, AssistantMessage, AssistantRoleTag, FunctionResultMessage, FunctionResultRoleTag,
    UserMessage, UserRoleTag,
};
use llm_router::types::model::AgentFunction;
use llm_router::types::router::ResponseFormat;
use provider_kimi::config::{KimiConfig, DEFAULT_API_URL};
use provider_kimi::count_tokens::{count, CountTokensRequest};
use provider_kimi::request::{build_body, build_headers, BodyArgs};
use serde_json::{json, Value};

const DEFAULT_PROBE_MODEL: &str = "kimi-k2-0905-preview";

fn user(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage {
        role: UserRoleTag::User,
        content: vec![ContentBlock::Text { text: text.into() }],
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
        model: DEFAULT_PROBE_MODEL.into(),
        provider: "kimi".into(),
        timestamp: 2,
    })
}

fn call(id: &str) -> ContentBlock {
    ContentBlock::FunctionCall {
        id: id.into(),
        function_id: "docs::read".into(),
        arguments: json!({ "segment": 3 }),
    }
}

fn result(id: &str) -> AgentMessage {
    AgentMessage::FunctionResult(FunctionResultMessage {
        role: FunctionResultRoleTag::FunctionResult,
        function_call_id: id.into(),
        function_id: "docs::read".into(),
        content: vec![ContentBlock::Text {
            text: "segment three".into(),
        }],
        details: json!({}),
        is_error: false,
        timestamp: 3,
    })
}

fn tool(name: &str) -> AgentFunction {
    AgentFunction {
        name: name.into(),
        description: format!("Run {name} on the workspace."),
        parameters: json!({
            "type": "object",
            "properties": { "segment": { "type": "integer" }, "path": { "type": "string" } },
        }),
        label: None,
        execution_mode: None,
    }
}

struct Shape {
    label: &'static str,
    system: &'static str,
    tools: Vec<AgentFunction>,
    response_format: Option<ResponseFormat>,
    messages: Vec<AgentMessage>,
}

fn shape(label: &'static str, tools: Vec<AgentFunction>, messages: Vec<AgentMessage>) -> Shape {
    Shape {
        label,
        system: "",
        tools,
        response_format: None,
        messages,
    }
}

fn shapes() -> Vec<Shape> {
    let read = || vec![tool("docs::read")];
    let three = vec![tool("docs::read"), tool("fs::write"), tool("shell::run")];
    let json_mode = Some(ResponseFormat {
        r#type: "json".into(),
        schema: None,
    });
    vec![
        shape("hi, 0 tools", vec![], vec![user("hi")]),
        shape("hi, 1 tool", read(), vec![user("hi")]),
        shape("hi, 3 tools", three, vec![user("hi")]),
        Shape {
            system: "You are a careful reader. Keep identifiers verbatim.",
            ..shape("hi + system", vec![], vec![user("hi")])
        },
        // json_object mode needs the word JSON in the prompt; the pair isolates
        // what the mode itself adds.
        shape("JSON hi", vec![], vec![user("Reply in JSON: hi")]),
        Shape {
            response_format: json_mode,
            ..shape(
                "JSON hi + json_object",
                vec![],
                vec![user("Reply in JSON: hi")],
            )
        },
        shape(
            "1 call",
            read(),
            vec![user("hi"), assistant(vec![call("c1")]), result("c1")],
        ),
        shape(
            "2 calls, 1 turn",
            read(),
            vec![
                user("hi"),
                assistant(vec![call("c1"), call("c2")]),
                result("c1"),
                result("c2"),
            ],
        ),
        shape(
            "text + call",
            read(),
            vec![
                user("hi"),
                assistant(vec![
                    ContentBlock::Text {
                        text: "reading".into(),
                    },
                    call("c1"),
                ]),
                result("c1"),
            ],
        ),
    ]
}

async fn billed(http: &reqwest::Client, cfg: &KimiConfig, s: &Shape) -> u64 {
    let mut body = build_body(&BodyArgs {
        model: cfg.model.clone(),
        max_tokens: 1,
        system_prompt: s.system.into(),
        messages: s.messages.clone(),
        tools: s.tools.clone(),
        response_format: s.response_format.clone(),
        prompt_cache_key: None,
    });
    body["stream"] = json!(false);
    body.as_object_mut().unwrap().remove("stream_options");
    let mut request = http.post(&cfg.api_url).json(&body);
    for (name, value) in build_headers(cfg) {
        request = request.header(name, value);
    }
    let reply: Value = request.send().await.unwrap().json().await.unwrap();
    reply["usage"]["prompt_tokens"]
        .as_u64()
        .unwrap_or_else(|| panic!("{}: no usage in {reply}", s.label))
}

async fn counted(http: &reqwest::Client, cfg: &KimiConfig, s: &Shape) -> u64 {
    let req = CountTokensRequest {
        model: cfg.model.clone(),
        system_prompt: Some(s.system.into()),
        tools: Some(s.tools.clone()),
        response_format: s.response_format.clone(),
        messages: s.messages.clone(),
    };
    count(http, cfg, &req)
        .await
        .unwrap_or_else(|e| panic!("{}: {e}", s.label))
        .tokens
}

#[tokio::test]
#[ignore = "live: needs MOONSHOT_API_KEY and bills a few hundred tokens"]
async fn estimator_counts_what_moonshot_bills() {
    let cfg = KimiConfig {
        credential_value: std::env::var("MOONSHOT_API_KEY")
            .expect("the live probe needs MOONSHOT_API_KEY"),
        model: std::env::var("KIMI_PROBE_MODEL").unwrap_or_else(|_| DEFAULT_PROBE_MODEL.into()),
        max_tokens: 1,
        api_url: DEFAULT_API_URL.into(),
    };
    let http = reqwest::Client::new();

    let mut rows = Vec::new();
    for s in shapes() {
        let (b, c) = (
            billed(&http, &cfg, &s).await,
            counted(&http, &cfg, &s).await,
        );
        println!(
            "{:<24} billed={b:<6} counted={c:<6} gap={:+}",
            s.label,
            b as i64 - c as i64
        );
        rows.push((s.label, b, c));
    }
    let delta = |from: &str, to: &str| {
        let get = |l: &str| rows.iter().find(|r| r.0 == l).unwrap();
        let (a, b) = (get(from), get(to));
        (b.1 as i64 - a.1 as i64, b.2 as i64 - a.2 as i64)
    };
    let (tools_billed, tools_counted) = delta("hi, 0 tools", "hi, 3 tools");
    println!("3 tools add: billed {tools_billed:+}, counted {tools_counted:+}");
    let (rf_billed, rf_counted) = delta("JSON hi", "JSON hi + json_object");
    println!("json_object adds: billed {rf_billed:+}, counted {rf_counted:+}");

    let off: Vec<_> = rows.iter().filter(|r| r.1 != r.2).map(|r| r.0).collect();
    assert!(
        off.is_empty(),
        "{} counted off the bill on: {off:?}",
        cfg.model
    );
}

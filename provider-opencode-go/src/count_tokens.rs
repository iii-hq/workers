//! `provider::opencode_go::count_tokens` — local prompt token counting with
//! each model's own vocabulary.
//!
//! The OpenCode Go gateway exposes no metering endpoint, so the count is
//! computed locally from the rows the wire mappers send. The gateway serves
//! open-weight families (DeepSeek, GLM, Qwen, MiMo, MiniMax, Hunyuan), and
//! counting them with o200k is wrong by whatever the vocabularies disagree
//! about: +24% on digit- and hex-heavy text for Qwen. Each family is counted
//! with its published `tokenizer.json`, fetched once and cached on disk; only
//! the gpt models keep tiktoken. DeepSeek-V4 models count the prompt
//! DeepSeek's encoder renders, the way provider-deepseek does, since that is
//! what DeepSeek bills: in the mode the effort `stream` sends puts it in,
//! with the response format `stream` sends rendered into the system message.
//!
//! Exposed behind `router::count_tokens`.

use iii_sdk::errors::Error;
use llm_router::provider_scaffold::chat_framing::Framed;
use llm_router::provider_scaffold::deepseek_v41::{
    self, effort_budget, Mode, DEFAULT_EFFORT_BUDGET,
};
use llm_router::provider_scaffold::tiktoken_count::{encoder_for, ESTIMATOR_TIKTOKEN};
use llm_router::provider_scaffold::vocabulary_count::{
    resolve, VocabularyRef, ESTIMATOR_TOKENIZER,
};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::{AgentFunction, ThinkingLevel};
use llm_router::types::router::ResponseFormat;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::reasoning::reasoning_effort_for;
use crate::request::build_chat_response_format;
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets; its family selects the vocabulary.
    pub model: String,
    /// System prompt counted as its own wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each counts as the tool entry the wire
    /// sends.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::opencode_go::stream`
    /// accepts. Must be non-empty.
    pub messages: Vec<AgentMessage>,
    /// The turn's thinking level. DeepSeek-V4 models count the prompt at the
    /// effort `stream` sends for it; other families ignore it.
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
    /// The turn's response format. DeepSeek-V4 models render the body it
    /// becomes into the system message, so it is billed as prompt; other
    /// families ignore it.
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens for the assembled request.
    pub tokens: u64,
    /// `tokenizer` when the model's own vocabulary produced the count,
    /// `tiktoken` for the gpt models.
    pub estimator: String,
}

enum Counter {
    Tiktoken,
    Vocabulary(VocabularyRef),
}

/// The vocabulary a model was trained with, by family. Kimi publishes only a
/// tiktoken-format file and Grok none, so they have no counter here.
fn counter(model: &str) -> Option<Counter> {
    let family = |prefix: &str| model.starts_with(prefix);
    let repo = if family("gpt-") {
        return Some(Counter::Tiktoken);
    } else if family("deepseek-") {
        return Some(Counter::Vocabulary(deepseek_v41::vocabulary()));
    } else if family("glm-") {
        "zai-org/GLM-5"
    } else if family("qwen") {
        // Qwen3.5 onwards share a 248k vocabulary, not Qwen3's 151k one.
        "Qwen/Qwen3.6-27B"
    } else if family("mimo-") {
        "XiaomiMiMo/MiMo-V2-Flash"
    } else if family("minimax-") {
        "MiniMaxAI/MiniMax-M2"
    } else if family("hy3") {
        "tencent/Hy3"
    } else {
        return None;
    };
    Some(Counter::Vocabulary(VocabularyRef::huggingface(repo)))
}

pub async fn handle(req: CountTokensRequest) -> Result<CountTokensResponse, Error> {
    // Dumb pipe: an empty request is a caller bug, never padded into a
    // countable one with placeholder messages.
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }
    let rows = to_wire_messages(
        &req.messages,
        req.system_prompt.as_deref().unwrap_or_default(),
    );
    let tools = functions_to_wire(req.tools.as_deref().unwrap_or(&[]));
    let no_counter = |why: String| Error::Handler(format!("router/no_token_counter: {why}"));
    let (tokens, estimator) = match counter(&req.model) {
        None => {
            return Err(no_counter(format!(
                "no published vocabulary for {}",
                req.model
            )))
        }
        Some(Counter::Tiktoken) => {
            let bpe = encoder_for(&req.model);
            let tokens = count_rows(&rows, &tools, |text| bpe.encode_ordinary(text).len() as u64);
            (tokens, ESTIMATOR_TIKTOKEN)
        }
        Some(Counter::Vocabulary(reference)) => {
            // A cold cache with no network leaves the caller on its own
            // estimate rather than on a count from the wrong vocabulary.
            let tokenizer = resolve(&reference).await.ok_or_else(|| {
                no_counter(format!(
                    "{}'s vocabulary is not cached and could not be fetched",
                    reference.id
                ))
            })?;
            let tokens = if req.model.starts_with("deepseek-v4") {
                let response_format = req.response_format.as_ref().map(build_chat_response_format);
                let prompt = deepseek_v41::prompt(
                    &rows,
                    &tools,
                    response_format.as_ref(),
                    deepseek_mode(&req.model, req.thinking_level),
                );
                tokenizer
                    .encode(prompt, false)
                    .map_err(|err| no_counter(err.to_string()))?
                    .len() as u64
            } else {
                count_rows(&rows, &tools, |text| {
                    // A row the vocabulary cannot encode still counts its framing.
                    tokenizer
                        .encode(text, false)
                        .map_or(0, |encoded| encoded.len() as u64)
                })
            };
            (tokens, ESTIMATOR_TOKENIZER)
        }
    };
    Ok(CountTokensResponse {
        model: req.model,
        tokens,
        estimator: estimator.into(),
    })
}

/// The encoder mode `stream` puts a DeepSeek-V4 turn in: thinking at the
/// effort it sends as `reasoning_effort`, or at the API default when it
/// sends none. It never disables thinking (`off` maps to `none`, which these
/// models do not list), so chat mode is never rendered.
fn deepseek_mode(model: &str, level: Option<ThinkingLevel>) -> Mode {
    Mode::Thinking {
        budget: reasoning_effort_for(level, model)
            .and_then(effort_budget)
            .unwrap_or(DEFAULT_EFFORT_BUDGET),
    }
}

/// The generic chat framing over the rows and tool entries the wire sends.
/// shortcut: these families' own templates (tool blocks, call markup) are not
/// rendered, so the count stays under the bill by that much; render them once
/// a live probe with an OpenCode Go key can check the result.
fn count_rows(rows: &[Value], tools: &[Value], count_text: impl Fn(&str) -> u64) -> u64 {
    let framed = Framed {
        rows: rows.iter().map(row_text).collect(),
        schemas: tools.iter().map(Value::to_string).collect(),
    };
    let text_tokens = framed
        .rows
        .iter()
        .chain(&framed.schemas)
        .map(|text| count_text(text))
        .sum();
    framed.total_from(text_tokens)
}

/// A wire row's text: its content, or the text parts of a content array (an
/// image's data URI is not text), then each call's name and arguments.
fn row_text(row: &Value) -> String {
    let mut parts: Vec<&str> = match &row["content"] {
        Value::String(text) => vec![text],
        Value::Array(items) => items
            .iter()
            .filter_map(|part| part["text"].as_str())
            .collect(),
        _ => vec![],
    };
    for call in row["tool_calls"].as_array().into_iter().flatten() {
        let function = &call["function"];
        parts.push(function["name"].as_str().unwrap_or_default());
        parts.push(function["arguments"].as_str().unwrap_or_default());
    }
    parts.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::provider_scaffold::chat_framing::{TOKENS_PER_MESSAGE, TOKENS_REPLY_PRIMING};
    use llm_router::types::content::ContentBlock;
    use llm_router::types::events::StopReason;
    use llm_router::types::messages::{
        AssistantMessage, AssistantRoleTag, FunctionResultMessage, FunctionResultRoleTag,
        UserMessage, UserRoleTag,
    };
    use serde_json::json;

    fn user(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            timestamp: 1,
        })
    }

    fn call(id: &str) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content: vec![ContentBlock::FunctionCall {
                id: id.into(),
                function_id: "shell::exec".into(),
                arguments: json!({ "cmd": "ls" }),
            }],
            stop_reason: StopReason::FunctionCall,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            warnings: None,
            usage: None,
            model: "m".into(),
            provider: "opencode_go".into(),
            timestamp: 2,
        })
    }

    fn result(id: &str, content: Vec<ContentBlock>, details: serde_json::Value) -> AgentMessage {
        AgentMessage::FunctionResult(FunctionResultMessage {
            role: FunctionResultRoleTag::FunctionResult,
            function_call_id: id.into(),
            function_id: "shell::exec".into(),
            content,
            details,
            is_error: false,
            timestamp: 3,
        })
    }

    fn text(text: &str) -> ContentBlock {
        ContentBlock::Text { text: text.into() }
    }

    fn request(model: &str, messages: Vec<AgentMessage>) -> CountTokensRequest {
        CountTokensRequest {
            model: model.into(),
            system_prompt: None,
            tools: None,
            messages,
            thinking_level: None,
            response_format: None,
        }
    }

    fn o200k(text: &str) -> u64 {
        encoder_for("gpt-5.6-luna").encode_ordinary(text).len() as u64
    }

    /// The expected count of `rows` framed rows whose texts are `texts`.
    fn framed(rows: u64, texts: &[&str]) -> u64 {
        TOKENS_REPLY_PRIMING
            + rows * TOKENS_PER_MESSAGE
            + texts.iter().map(|t| o200k(t)).sum::<u64>()
    }

    async fn count(req: CountTokensRequest) -> u64 {
        handle(req).await.unwrap().tokens
    }

    #[tokio::test]
    async fn empty_messages_are_rejected() {
        assert!(handle(request("gpt-5.6-luna", vec![])).await.is_err());
    }

    #[tokio::test]
    async fn gpt_counts_with_tiktoken() {
        let resp = handle(request("gpt-5.6-luna", vec![user("hello world")]))
            .await
            .unwrap();
        assert_eq!(resp.tokens, framed(1, &["hello world"]));
        assert_eq!(resp.estimator, "tiktoken");
        assert_eq!(resp.model, "gpt-5.6-luna");
    }

    #[tokio::test]
    async fn an_orphan_call_counts_the_placeholder_row_the_wire_adds() {
        let tokens = count(request("gpt-5.6-luna", vec![call("t1")])).await;
        assert_eq!(
            tokens,
            framed(
                2,
                &[
                    "shell__exec\n{\"cmd\":\"ls\"}",
                    "Tool call was interrupted before completing. Continue without its output.",
                ]
            )
        );
    }

    #[tokio::test]
    async fn a_denied_result_counts_its_permission_envelope() {
        let details = json!({ "status": "denied", "reason": "no" });
        let tokens = count(request(
            "gpt-5.6-luna",
            vec![
                call("t1"),
                result("t1", vec![text("denied")], details.clone()),
            ],
        ))
        .await;
        let envelope = format!("[PERMISSION_DENIED]\n{details}\n\ndenied");
        assert_eq!(
            tokens,
            framed(2, &["shell__exec\n{\"cmd\":\"ls\"}", &envelope])
        );
    }

    #[tokio::test]
    async fn a_replaced_result_counts_once() {
        let tokens = count(request(
            "gpt-5.6-luna",
            vec![
                call("t1"),
                result("t1", vec![text("first")], json!({})),
                result("t1", vec![text("second")], json!({})),
            ],
        ))
        .await;
        assert_eq!(
            tokens,
            framed(2, &["shell__exec\n{\"cmd\":\"ls\"}", "second"])
        );
    }

    #[tokio::test]
    async fn a_hoisted_image_row_counts_its_text_but_not_the_data_uri() {
        let image = ContentBlock::Image {
            mime: "image/png".into(),
            data: "QUJD".repeat(64),
            attachment_id: None,
        };
        let tokens = count(request(
            "gpt-5.6-luna",
            vec![
                call("t1"),
                result("t1", vec![text("shot"), image], json!({})),
            ],
        ))
        .await;
        assert_eq!(
            tokens,
            framed(
                3,
                &[
                    "shell__exec\n{\"cmd\":\"ls\"}",
                    "shot",
                    "[image result of tool call t1]",
                ]
            )
        );
    }

    #[tokio::test]
    async fn tools_count_in_the_envelope_the_wire_sends() {
        let tool = AgentFunction {
            name: "agent::trigger".into(),
            description: "Invoke an iii function".into(),
            parameters: json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        };
        let mut req = request("gpt-5.6-luna", vec![user("hi")]);
        req.tools = Some(vec![tool.clone()]);
        let wire = crate::wire::tools::functions_to_wire(&[tool])[0].to_string();
        assert!(wire.contains(r#""type":"function""#));
        assert_eq!(count(req).await, framed(1, &["hi", &wire]));
    }

    #[tokio::test]
    async fn a_model_without_a_known_vocabulary_has_no_counter() {
        // Kimi publishes a tiktoken-format vocabulary and Grok none at all;
        // o200k would be a confident wrong number.
        for model in ["kimi-k3", "grok-4.5", "mystery-model"] {
            let err = handle(request(model, vec![user("hi")])).await.unwrap_err();
            assert!(
                err.to_string().contains("router/no_token_counter"),
                "{model}: {err}"
            );
        }
    }

    #[test]
    fn each_open_weight_family_counts_with_its_own_vocabulary() {
        let repo = |model: &str| match counter(model) {
            Some(Counter::Vocabulary(reference)) => reference.id,
            Some(Counter::Tiktoken) => "o200k".into(),
            None => "none".into(),
        };
        for (model, expected) in [
            ("gpt-5.6-luna", "o200k"),
            ("deepseek-v4-pro", "deepseek-ai--DeepSeek-V4.1-Flash"),
            ("deepseek-v4-flash", "deepseek-ai--DeepSeek-V4.1-Flash"),
            ("glm-5.2", "zai-org--GLM-5"),
            ("glm-5", "zai-org--GLM-5"),
            ("qwen3.5-plus", "Qwen--Qwen3.6-27B"),
            ("qwen3.8-max", "Qwen--Qwen3.6-27B"),
            ("mimo-v2-pro", "XiaomiMiMo--MiMo-V2-Flash"),
            ("mimo-v2.5-pro", "XiaomiMiMo--MiMo-V2-Flash"),
            ("minimax-m3", "MiniMaxAI--MiniMax-M2"),
            ("minimax-m2.5", "MiniMaxAI--MiniMax-M2"),
            ("hy3", "tencent--Hy3"),
            ("hy3-preview", "tencent--Hy3"),
            ("kimi-k2.6", "none"),
            ("grok-4.5", "none"),
        ] {
            assert_eq!(repo(model), expected, "{model}");
        }
    }

    /// A word-level stand-in for a published vocabulary, served from a
    /// private cache so a test never reaches HuggingFace: one token per word
    /// or run of punctuation.
    fn fixture_vocabularies() {
        static DIR: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();
        DIR.get_or_init(|| {
            let dir =
                std::env::temp_dir().join(format!("opencode-go-vocab-{}", std::process::id()));
            std::fs::create_dir_all(&dir).unwrap();
            let tokenizer = json!({
                "version": "1.0", "truncation": null, "padding": null, "added_tokens": [],
                "normalizer": null, "pre_tokenizer": { "type": "Whitespace" },
                "post_processor": null, "decoder": null,
                "model": { "type": "WordLevel", "vocab": { "[UNK]": 0 }, "unk_token": "[UNK]" }
            });
            for id in ["deepseek-ai--DeepSeek-V4.1-Flash", "zai-org--GLM-5"] {
                std::fs::write(dir.join(format!("{id}.json")), tokenizer.to_string()).unwrap();
            }
            std::env::set_var("III_TOKENIZER_CACHE_DIR", &dir);
            dir
        });
    }

    /// The fixture's count of `text`.
    async fn words(text: &str) -> u64 {
        let tokenizer = resolve(&VocabularyRef::huggingface("zai-org/GLM-5"))
            .await
            .unwrap();
        tokenizer.encode(text, false).unwrap().len() as u64
    }

    fn round_trip() -> CountTokensRequest {
        let tool = AgentFunction {
            name: "shell::exec".into(),
            description: "Run a command".into(),
            parameters: json!({ "type": "object", "properties": { "cmd": { "type": "string" } } }),
            label: None,
            execution_mode: None,
        };
        let mut req = request(
            "",
            vec![
                user("list it"),
                call("t1"),
                result("t1", vec![text("a.txt")], json!({})),
            ],
        );
        req.system_prompt = Some("be brief".into());
        req.tools = Some(vec![tool]);
        req
    }

    #[test]
    fn deepseek_counts_at_the_effort_stream_sends() {
        // high is the only level stream sends for these models (their ladder
        // is high/max); every other level, `off` included, sends nothing and
        // runs at the API default, which is also high.
        for level in [
            None,
            Some(ThinkingLevel::Off),
            Some(ThinkingLevel::Low),
            Some(ThinkingLevel::High),
            Some(ThinkingLevel::Xhigh),
        ] {
            assert_eq!(
                deepseek_mode("deepseek-v4-flash", level),
                Mode::Thinking { budget: 75 },
                "{level:?}"
            );
        }
    }

    #[tokio::test]
    async fn deepseek_v4_counts_the_prompt_its_encoder_renders_from_the_wire() {
        fixture_vocabularies();
        let format = ResponseFormat {
            r#type: "json".into(),
            schema: Some(json!({ "type": "object" })),
        };
        let req = CountTokensRequest {
            model: "deepseek-v4-flash".into(),
            thinking_level: Some(ThinkingLevel::High),
            response_format: Some(format.clone()),
            ..round_trip()
        };
        let rows = to_wire_messages(&req.messages, "be brief");
        let tools = functions_to_wire(req.tools.as_deref().unwrap());
        let rendered = deepseek_v41::prompt(
            &rows,
            &tools,
            Some(&build_chat_response_format(&format)),
            Mode::Thinking { budget: 75 },
        );
        assert!(rendered.contains("json_schema"));
        let expected = words(&rendered).await;
        let resp = handle(req).await.unwrap();
        assert_eq!(resp.tokens, expected);
        assert_eq!(resp.estimator, "tokenizer");
    }

    #[tokio::test]
    async fn other_families_count_the_wire_rows_with_their_vocabulary() {
        fixture_vocabularies();
        let req = CountTokensRequest {
            model: "glm-5.2".into(),
            ..round_trip()
        };
        let tools = functions_to_wire(req.tools.as_deref().unwrap());
        let texts = [
            "be brief",
            "list it",
            "shell__exec\n{\"cmd\":\"ls\"}",
            "a.txt",
            &tools[0].to_string(),
        ];
        let mut expected = TOKENS_REPLY_PRIMING + 4 * TOKENS_PER_MESSAGE;
        for text in texts {
            expected += words(text).await;
        }
        let resp = handle(req).await.unwrap();
        assert_eq!(resp.tokens, expected);
        assert_eq!(resp.estimator, "tokenizer");
    }
}

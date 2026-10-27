//! `provider::deepseek::count_tokens` — prompt token counting with DeepSeek's
//! own vocabulary.
//!
//! DeepSeek publishes no metering endpoint, so the count is computed locally.
//! It is computed with DeepSeek's published tokenizer rather than a borrowed
//! one: the OpenAI-compatible wire shape does not imply an OpenAI vocabulary,
//! and counting these models with tiktoken would produce a number that looks
//! authoritative while being wrong by whatever the two vocabularies disagree
//! about. The vocabulary is fetched once and cached on disk, so a model
//! DeepSeek ships tomorrow counts correctly today.
//!
//! What gets counted is the prompt DeepSeek builds from this provider's wire
//! rows, not the rows' text: the tool block, the DSML each call becomes and
//! the replayed `reasoning_content` are billed as prompt tokens too. Counting
//! the text alone left `context_pressure` ~7% under its bill.
//!
//! Exposed behind `router::count_tokens`.

use iii_sdk::errors::Error;
use llm_router::provider_scaffold::vocabulary_count::{
    resolve, VocabularyRef, ESTIMATOR_TOKENIZER,
};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;

/// Every DeepSeek chat model shares one vocabulary, so the reference is fixed
/// rather than per-model: the id list changes far more often than the
/// tokenizer behind it. V4.1's file has the same vocabulary as V3's plus the
/// prompt's own markers (`<think>`, `｜DSML｜`, `<｜System｜>`) as single tokens.
fn vocabulary() -> VocabularyRef {
    VocabularyRef::huggingface("deepseek-ai/DeepSeek-V4.1-Flash")
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets. Reported back unchanged; every DeepSeek
    /// chat model shares the one vocabulary.
    pub model: String,
    /// System prompt counted as its own wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward the
    /// total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::deepseek::stream`
    /// accepts. Must be non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens for the assembled request, by DeepSeek's vocabulary.
    pub tokens: u64,
    /// Always `tokenizer`: the model's own vocabulary produced the count.
    pub estimator: String,
}

pub async fn handle(req: CountTokensRequest) -> Result<CountTokensResponse, Error> {
    // Dumb pipe: an empty request is a caller bug, never padded into a
    // countable one with placeholder messages.
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }
    // A cold cache with no network is the one case with no honest answer
    // here. Reporting the typed no_token_counter error leaves the caller on
    // its own estimate, which is the same place it would have been without
    // this function — better than a number computed from the wrong
    // vocabulary.
    let tokenizer = resolve(&vocabulary()).await.ok_or_else(|| {
        Error::Handler(
            "router/no_token_counter: DeepSeek's vocabulary is not cached and could \
             not be fetched"
                .into(),
        )
    })?;
    let rows = to_wire_messages(
        &req.messages,
        req.system_prompt.as_deref().unwrap_or_default(),
    );
    let tools = functions_to_wire(req.tools.as_deref().unwrap_or(&[]));
    let tokens = tokenizer
        .encode(prompt(&rows, &tools), false)
        .map_err(|err| Error::Handler(format!("router/no_token_counter: {err}")))?
        .len() as u64;
    Ok(CountTokensResponse {
        model: req.model,
        tokens,
        estimator: ESTIMATOR_TOKENIZER.into(),
    })
}

/// The opening of every prompt in thinking mode, the API's default: the
/// `high` effort budget.
const THINKING_PREFIX: &str =
    "<｜System｜>Reasoning Effort: 75 (range 1-100, the higher the value, \
the more thorough the reasoning)\n\n";

/// The tool block appended to the system message, verbatim from the
/// encoder's `TOOLS_TEMPLATE`; the schemas go between header and footer.
const TOOLS_HEADER: &str = "\n\n## Tools\n\nYou have access to a set of tools to help answer the \
user's question. You can invoke tools by writing a \"<｜DSML｜ calls>\" block like the following:\n\n\
<｜DSML｜ calls>\n<｜DSML｜ invoke name=\"$TOOL_NAME\">\n<｜DSML｜ parameter name=\"$PARAMETER_NAME\" \
string=\"true|false\">$PARAMETER_VALUE</｜DSML｜ parameter>\n...\n</｜DSML｜ invoke>\n<｜DSML｜ invoke \
name=\"$TOOL_NAME2\">\n...\n</｜DSML｜ invoke>\n</｜DSML｜ calls>\n\nString parameters should be \
specified as is and set `string=\"true\"`. For all other types (numbers, booleans, arrays, objects), \
pass the value in JSON format and set `string=\"false\"`.\n\nIf thinking_mode is enabled (triggered \
by <think>), you MUST output your complete reasoning inside <think>...</think> BEFORE any tool calls \
or final response.\n\nOtherwise, output directly after </think> with tool calls or final \
response.\n\n### Available Tool Schemas\n\n";
const TOOLS_FOOTER: &str =
    "\n\nYou MUST strictly follow the above defined tool name and parameter \
schemas to invoke tool calls.\n";

/// A turn of the encoded conversation. The encoder has no tool role: results,
/// and user text sent right after them, merge into one user turn.
enum Turn<'a> {
    System(&'a str),
    User { parts: Vec<String>, text: bool },
    Assistant(&'a Value),
}

/// The prompt DeepSeek builds from this provider's wire rows and tools, in
/// the thinking mode the API defaults to: DeepSeek-V4.1's published encoder
/// (`encoding/encoding.py`), reduced to the rows this wire sends.
fn prompt(rows: &[Value], tools: &[Value]) -> String {
    let mut turns = Vec::new();
    // Tools ride on the system message; a request without one gets an empty one.
    if !tools.is_empty() && rows.first().is_none_or(|row| row["role"] != "system") {
        turns.push(Turn::System(""));
    }
    for row in rows {
        let content = row["content"].as_str().unwrap_or_default();
        let (part, text) = match row["role"].as_str() {
            Some("system") => {
                turns.push(Turn::System(content));
                continue;
            }
            Some("assistant") => {
                turns.push(Turn::Assistant(row));
                continue;
            }
            Some("tool") => (format!("<tool_result>{content}</tool_result>"), false),
            _ => (content.to_string(), true),
        };
        match turns.last_mut() {
            Some(Turn::User { parts, text: any }) => {
                parts.push(part);
                *any |= text;
            }
            _ => turns.push(Turn::User {
                parts: vec![part],
                text,
            }),
        }
    }
    // A turn of tool results alone does not start a new user turn.
    let last_user = turns
        .iter()
        .rposition(|turn| matches!(turn, Turn::User { text: true, .. }));
    // With tools the encoder keeps every turn's reasoning; without, only the
    // reasoning after the last user turn.
    let reasons = |index: usize| !tools.is_empty() || last_user.is_none_or(|user| index > user);

    let mut out = String::from("<｜begin▁of▁sentence｜>");
    out.push_str(THINKING_PREFIX);
    for (index, turn) in turns.iter().enumerate() {
        match turn {
            Turn::System(content) => {
                out.push_str(content);
                if !tools.is_empty() {
                    let schemas: Vec<String> = tools
                        .iter()
                        .map(|tool| python_json(&tool["function"]))
                        .collect();
                    out.push_str(TOOLS_HEADER);
                    out.push_str(&schemas.join("\n"));
                    out.push_str(TOOLS_FOOTER);
                }
            }
            Turn::User { parts, .. } => {
                out.push_str("<｜User｜>");
                out.push_str(&parts.join("\n\n"));
                if !matches!(
                    turns.get(index + 1),
                    Some(Turn::User { .. } | Turn::System(_))
                ) {
                    out.push_str("<｜Assistant｜>");
                    let opens = !tools.is_empty() || last_user.is_none_or(|user| index >= user);
                    out.push_str(if opens { "<think>" } else { "</think>" });
                }
            }
            Turn::Assistant(row) => {
                if reasons(index) {
                    out.push_str(row["reasoning_content"].as_str().unwrap_or_default());
                    out.push_str("</think>");
                }
                out.push_str(row["content"].as_str().unwrap_or_default());
                if let Some(calls) = row["tool_calls"].as_array() {
                    let invokes: Vec<String> = calls.iter().map(invoke).collect();
                    out.push_str("\n\n<｜DSML｜ calls>\n");
                    out.push_str(&invokes.join("\n"));
                    out.push_str("\n</｜DSML｜ calls>");
                }
                out.push_str("<｜end▁of▁sentence｜>");
            }
        }
    }
    out
}

/// One call as DSML: a parameter tag per argument, strings bare and every
/// other value as JSON.
fn invoke(call: &Value) -> String {
    let raw = call["function"]["arguments"].as_str().unwrap_or_default();
    // The encoder tolerates double-encoded arguments and wraps anything that
    // is not an object.
    let mut arguments = serde_json::from_str::<Value>(raw).unwrap_or(Value::Null);
    if let Value::String(inner) = &arguments {
        arguments = serde_json::from_str(inner).unwrap_or(Value::Null);
    }
    let arguments = match arguments {
        Value::Object(map) => map,
        _ => serde_json::Map::from_iter([("arguments".to_string(), Value::from(raw))]),
    };
    let parameters: Vec<String> = arguments
        .iter()
        .map(|(key, value)| {
            let (string, value) = match value {
                Value::String(text) => ("true", text.clone()),
                other => ("false", python_json(other)),
            };
            format!("<｜DSML｜ parameter name=\"{key}\" string=\"{string}\">{value}</｜DSML｜ parameter>")
        })
        .collect();
    format!(
        "<｜DSML｜ invoke name=\"{}\">\n{}\n</｜DSML｜ invoke>",
        call["function"]["name"].as_str().unwrap_or_default(),
        parameters.join("\n")
    )
}

/// JSON as Python's `json.dumps` writes it, which is what the encoder uses:
/// a space after every `:` and `,`.
fn python_json(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let fields: Vec<String> = map
                .iter()
                .map(|(key, value)| {
                    format!("{}: {}", Value::from(key.as_str()), python_json(value))
                })
                .collect();
            format!("{{{}}}", fields.join(", "))
        }
        Value::Array(items) => {
            let items: Vec<String> = items.iter().map(python_json).collect();
            format!("[{}]", items.join(", "))
        }
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Prompts DeepSeek's published V4.1 encoder (`encoding/encoding.py` in
    /// deepseek-ai/DeepSeek-V4.1-Flash, MIT) renders: its own tool-calling
    /// case and an agent-shaped one, keys sorted as serde_json sends them.
    #[test]
    fn requests_render_as_deepseeks_encoder_renders_them() {
        let cases: Value =
            serde_json::from_str(include_str!("../tests/golden/encoding/dsv41_prompts.json"))
                .unwrap();
        for (name, case) in cases.as_object().unwrap() {
            let rendered = prompt(
                case["messages"].as_array().unwrap(),
                case["tools"].as_array().unwrap(),
            );
            assert_eq!(rendered, case["prompt"].as_str().unwrap(), "{name}");
        }
    }

    #[test]
    fn without_tools_reasoning_before_the_last_user_turn_is_dropped() {
        let rows = [
            serde_json::json!({"role": "user", "content": "Hello"}),
            serde_json::json!({"role": "assistant", "reasoning_content": "Greet back.", "content": "Hi!"}),
            serde_json::json!({"role": "user", "content": "And again?"}),
            serde_json::json!({"role": "user", "content": "Twice."}),
            serde_json::json!({"role": "assistant", "content": "Sure."}),
            serde_json::json!({"role": "user", "content": "Last one."}),
        ];
        assert_eq!(
            prompt(&rows, &[]),
            format!(
                "<｜begin▁of▁sentence｜>{THINKING_PREFIX}<｜User｜>Hello<｜Assistant｜></think>Hi!\
                 <｜end▁of▁sentence｜><｜User｜>And again?\n\nTwice.<｜Assistant｜></think>Sure.\
                 <｜end▁of▁sentence｜><｜User｜>Last one.<｜Assistant｜><think>"
            )
        );
    }
}

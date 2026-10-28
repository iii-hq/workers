//! `provider::zai::count_tokens` — prompt token counting with GLM's own
//! vocabulary.
//!
//! Z.AI publishes no metering endpoint, so the count is computed locally, and
//! computed with the vocabulary GLM was trained with rather than a borrowed
//! one. The OpenAI-compatible wire shape does not imply an OpenAI tokenizer;
//! counting these models with tiktoken would produce a number that reads as
//! authoritative while being wrong by whatever the two vocabularies disagree
//! about — and GLM's disagrees most on exactly the Chinese text these models
//! are used for.
//!
//! What gets counted is the prompt GLM's chat template builds from the body
//! `provider::zai::stream` sends, not the rows' text: the tool block, the
//! Python-spaced schemas, the `<tool_call>` markup, the denied envelope and
//! orphan placeholders the wire adds, and the effort line on glm-5.2+ are
//! billed as prompt tokens too. Counting the text alone left a request with
//! one small tool at 35 tokens for a bill of 145.
//!
//! Exposed behind `router::count_tokens`.

use iii_sdk::errors::Error;
use llm_router::provider_scaffold::vocabulary_count::{
    resolve, VocabularyRef, ESTIMATOR_TOKENIZER,
};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::{AgentFunction, ThinkingLevel};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::reasoning::{
    accepts_effort, always_reasons, effective_level, is_reasoning_model, reasoning_effort_for,
    thinking_type,
};
use crate::request::{build_body, BodyArgs};

/// The GLM chat family shares one vocabulary, so the reference is fixed
/// rather than per-model: the id list turns over far faster than the
/// tokenizer behind it.
fn vocabulary() -> VocabularyRef {
    VocabularyRef::huggingface("zai-org/GLM-4.6")
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets, reported back unchanged. It picks the
    /// chat template (glm-5.2 and newer have their own); the GLM chat models
    /// share the one vocabulary.
    pub model: String,
    /// System prompt counted as its own wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward the
    /// total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::zai::stream` accepts.
    /// Must be non-empty.
    pub messages: Vec<AgentMessage>,
    /// The turn's thinking level, as `provider::zai::stream` takes it. On
    /// glm-5.2 and newer it decides the `Reasoning Effort` line the template
    /// adds; absent counts the request a turn without a level sends.
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens for the assembled request, by GLM's vocabulary.
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
    let tokenizer = resolve(&vocabulary()).await.ok_or_else(|| {
        Error::Handler(
            "router/no_token_counter: GLM's vocabulary is not cached and could not \
             be fetched"
                .into(),
        )
    })?;
    // The body stream sends, so the wire's own rules (denied envelope, orphan
    // placeholders, deduplicated results, omitted empty turns) are counted.
    // No catalog lookup here: the id pattern decides, as stream's fallback.
    let reasoning = is_reasoning_model(&req.model, None);
    let level = effective_level(req.thinking_level, &req.model, reasoning);
    let body = build_body(&BodyArgs {
        model: req.model.clone(),
        max_tokens: 0,
        system_prompt: req.system_prompt.unwrap_or_default(),
        messages: req.messages,
        tools: req.tools.unwrap_or_default(),
        thinking: thinking_type(level, reasoning),
        reasoning_effort: reasoning
            .then(|| reasoning_effort_for(level, &req.model))
            .flatten(),
        response_format: None,
    });
    let tokens = tokenizer
        .encode(prompt(&body), false)
        .map_err(|err| Error::Handler(format!("router/no_token_counter: {err}")))?
        .len() as u64;
    Ok(CountTokensResponse {
        model: req.model,
        tokens,
        estimator: ESTIMATOR_TOKENIZER.into(),
    })
}

/// The tool block GLM's template opens the prompt with; the schemas go
/// between header and footer, one per line.
const TOOLS_HEADER: &str = "<|system|>\n# Tools\n\nYou may call one or more functions to assist \
with the user query.\n\nYou are provided with function signatures within <tools></tools> XML \
tags:\n<tools>\n";
const TOOLS_FOOTER: &str = "</tools>\n\nFor each function call, output the function name and \
arguments within the following XML format:\n<tool_call>{function-name}<arg_key>{arg-key-1}\
</arg_key><arg_value>{arg-value-1}</arg_value><arg_key>{arg-key-2}</arg_key><arg_value>\
{arg-value-2}</arg_value>...</tool_call>";

/// What glm-5.2+'s template writes for a media part it cannot read.
const IMAGE_REMINDER: &str = "<reminder>You are unable to process this image because you don't \
have multi-modal input ability. Try different methods.</reminder>";

/// The prompt GLM's chat template (`chat_template.jinja` of zai-org/GLM-4.7,
/// GLM-5.2 and GLM-5.3) builds from a request body this provider sends,
/// reduced to the rows this wire sends. glm-4.x through 5.1 share the first
/// form; 5.2 and newer close past turns with `<think></think>`, drop `strict`
/// from the schemas, and open with the effort line.
fn prompt(body: &Value) -> String {
    let model = body["model"].as_str().unwrap_or_default();
    let modern = accepts_effort(model);
    let always = always_reasons(model);
    let thinking = always || body["thinking"]["type"] == "enabled";

    let mut out = String::from("[gMASK]<sop>");
    if modern && thinking {
        // 5.2 knows `high` and `max`, 5.3 adds `low`; anything else is `Max`.
        let effort = match body["reasoning_effort"].as_str() {
            Some("high") => "High",
            Some("low") if always => "Low",
            _ => "Max",
        };
        out.push_str("<|system|>Reasoning Effort: ");
        out.push_str(effort);
    }
    if let Some(tools) = body["tools"].as_array() {
        out.push_str(TOOLS_HEADER);
        for tool in tools {
            let mut function = tool["function"].clone();
            // Z.AI bills glm-4.x schemas with `strict` appended (measured);
            // 5.2+'s template drops the key.
            if !modern {
                function["strict"] = Value::Bool(false);
            }
            out.push_str(&python_json(&function));
            out.push('\n');
        }
        out.push_str(TOOLS_FOOTER);
    }
    let mut previous = None;
    for row in body["messages"].as_array().into_iter().flatten() {
        let role = row["role"].as_str();
        match role {
            Some("assistant") => {
                out.push_str("<|assistant|>");
                // The wire replays no reasoning, so every past turn is closed.
                out.push_str(if modern {
                    "<think></think>"
                } else {
                    "</think>"
                });
                out.push_str(visible_text(&row["content"], modern).trim());
                for call in row["tool_calls"].as_array().into_iter().flatten() {
                    out.push_str(&tool_call(call));
                }
            }
            Some("tool") => {
                if previous != Some("tool") {
                    out.push_str("<|observation|>");
                }
                out.push_str("<tool_response>");
                out.push_str(row["content"].as_str().unwrap_or_default());
                out.push_str("</tool_response>");
            }
            Some("system") => {
                out.push_str("<|system|>");
                out.push_str(&visible_text(&row["content"], modern));
            }
            _ => {
                out.push_str("<|user|>");
                out.push_str(&visible_text(&row["content"], modern));
            }
        }
        previous = role;
    }
    out.push_str("<|assistant|>");
    out.push_str(match (thinking, modern) {
        (true, _) => "<think>",
        (false, true) => "<think></think>",
        (false, false) => "</think>",
    });
    out
}

/// A row's text: a string as is, a part array as its text parts joined with
/// nothing between them (plus the reminder for each image on 5.2+).
fn visible_text(content: &Value, modern: bool) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .map(|part| match part["type"].as_str() {
                Some("text") => part["text"].as_str().unwrap_or_default(),
                Some("image_url") if modern => IMAGE_REMINDER,
                _ => "",
            })
            .collect(),
        _ => String::new(),
    }
}

/// One call as GLM's template writes it: a key/value pair per argument,
/// strings bare and every other value as JSON.
fn tool_call(call: &Value) -> String {
    let function = &call["function"];
    let raw = function["arguments"].as_str().unwrap_or_default();
    // shortcut: arguments that are not a JSON object count as one `arguments`
    // pair; how Z.AI renders them is unmeasured, revisit if they show up.
    let arguments = match serde_json::from_str::<Value>(raw) {
        Ok(Value::Object(map)) => map,
        _ => serde_json::Map::from_iter([("arguments".to_string(), Value::from(raw))]),
    };
    let mut out = format!(
        "<tool_call>{}",
        function["name"].as_str().unwrap_or_default()
    );
    for (key, value) in &arguments {
        let value = match value {
            Value::String(text) => text.clone(),
            other => python_json(other),
        };
        out.push_str(&format!(
            "<arg_key>{key}</arg_key><arg_value>{value}</arg_value>"
        ));
    }
    out.push_str("</tool_call>");
    out
}

/// JSON as Python's `json.dumps` writes it, which is what the template's
/// `tojson` uses: a space after every `:` and `,`.
// shortcut: a copy of provider-deepseek's; move both into llm-router's
// provider_scaffold when the shared helper lands (MOT-5568).
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

    /// Prompts GLM's published chat templates (`chat_template.jinja` of
    /// zai-org/GLM-4.7, GLM-5.2 and GLM-5.3, rendered with transformers'
    /// Jinja settings) build from wire bodies this provider sends, keys sorted
    /// as serde_json sends them. glm-4.x tools go in as Z.AI bills them: the
    /// function object with `"strict": false`.
    #[test]
    fn requests_render_as_glms_chat_template_renders_them() {
        let cases: Value =
            serde_json::from_str(include_str!("../tests/golden/encoding/glm_prompts.json"))
                .unwrap();
        for (name, case) in cases.as_object().unwrap() {
            assert_eq!(
                prompt(&case["body"]),
                case["prompt"].as_str().unwrap(),
                "{name}"
            );
        }
    }
}

//! `provider::sarvam::count_tokens` — prompt token counting with Sarvam's own
//! vocabulary and chat templates.
//!
//! Sarvam exposes no metering endpoint, so the count is computed locally from
//! what Sarvam publishes on HuggingFace: sarvam-30b and sarvam-105b share one
//! `tokenizer.json` and one `chat_template.jinja`, and sarvam-m ships Mistral's
//! Tekken vocabulary with its own template. o200k under-counted code and hex by
//! a fifth to a third and over-counted Indic scripts, and the generic framing
//! missed the tool block and the call markup the template renders.
//!
//! What gets counted is the prompt the template renders from this provider's
//! wire rows, so orphan placeholders, denied envelopes and the latest-wins
//! result dedup count as they are sent. Ids outside the Sarvam lineup (a custom
//! endpoint behind an `api_url` override), and a cold cache with no network,
//! keep the tiktoken estimate.
//!
//! Exposed behind `router::count_tokens`.
use iii_sdk::errors::Error;
use llm_router::provider_scaffold::tiktoken_count::{count_chat_tokens, ESTIMATOR_TIKTOKEN};
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets; reported back unchanged.
    pub model: String,
    /// System prompt counted as its own wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward
    /// the total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::sarvam::stream`
    /// accepts. Must be non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens for the assembled request.
    pub tokens: u64,
    /// `tokenizer` when Sarvam's own vocabulary and template produced the
    /// count; `tiktoken` for the estimate kept for other ids and a cold
    /// offline cache.
    pub estimator: String,
}

type Render = fn(&[Value], &[Value]) -> String;

/// The published vocabulary and template for a Sarvam chat id. Every
/// `sarvam-*` id but sarvam-m is the 30b/105b family: both repos ship the
/// same files, and a new size is likelier to join it than to bring a third.
fn family(model: &str) -> Option<(VocabularyRef, Render)> {
    let model = model.to_ascii_lowercase();
    if model == "sarvam-m" {
        Some((
            VocabularyRef::huggingface("sarvamai/sarvam-m"),
            sarvam_m_prompt,
        ))
    } else if model.starts_with("sarvam-") {
        Some((VocabularyRef::huggingface("sarvamai/sarvam-30b"), prompt))
    } else {
        None
    }
}

pub async fn handle(req: CountTokensRequest) -> Result<CountTokensResponse, Error> {
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }
    let tools = req.tools.as_deref().unwrap_or(&[]);
    let tokenizer = match family(&req.model) {
        Some((vocabulary, render)) => resolve(&vocabulary).await.map(|t| (t, render)),
        None => None,
    };
    let (tokens, estimator) = match tokenizer {
        Some((tokenizer, render)) => {
            let rows = to_wire_messages(
                &req.messages,
                req.system_prompt.as_deref().unwrap_or_default(),
            );
            let tokens = tokenizer
                .encode(render(&rows, &functions_to_wire(tools)), false)
                .map_err(|err| Error::Handler(format!("router/no_token_counter: {err}")))?
                .len() as u64;
            (tokens, ESTIMATOR_TOKENIZER)
        }
        None => (
            count_chat_tokens(
                &req.model,
                req.system_prompt.as_deref(),
                tools,
                &req.messages,
            ),
            ESTIMATOR_TIKTOKEN,
        ),
    };
    Ok(CountTokensResponse {
        model: req.model,
        tokens,
        estimator: estimator.into(),
    })
}

/// sarvam-30b/105b's published `chat_template.jinja`, reduced to the rows
/// this wire sends, with the generation prompt. The template's `<|nothink|>`
/// needs `enable_thinking: false`, which Sarvam has no switch for.
fn prompt(rows: &[Value], tools: &[Value]) -> String {
    let mut out = String::from("[@BOS@]\n");
    if !tools.is_empty() {
        out.push_str("<|start_of_turn|><|tool_declare|>\n<tools>\n");
        for tool in tools {
            out.push_str(&python_json(tool));
            out.push('\n');
        }
        out.push_str("</tools><|end_of_turn|>\n");
    }
    let last_user = rows.iter().rposition(|row| row["role"] == "user");
    for (index, row) in rows.iter().enumerate() {
        match row["role"].as_str() {
            Some(role @ ("user" | "system")) => {
                out.push_str(&format!("<|start_of_turn|><|{role}|>\n"));
                out.push_str(&visible_text(&row["content"]));
                out.push_str("<|end_of_turn|>\n");
            }
            Some("assistant") => {
                out.push_str("<|start_of_turn|><|assistant|>\n");
                let (reasoning, content) = split_think(visible_text(&row["content"]));
                // Reasoning is replayed only after the last user turn.
                if last_user.is_none_or(|user| index > user) && !reasoning.is_empty() {
                    out.push_str(&format!("<think>{}</think>", reasoning.trim()));
                } else {
                    out.push_str("<think></think>");
                }
                if !content.trim().is_empty() {
                    out.push('\n');
                    out.push_str(content.trim());
                }
                for call in row["tool_calls"].as_array().into_iter().flatten() {
                    let call = &call["function"];
                    out.push_str("\n<tool_call>");
                    out.push_str(call["name"].as_str().unwrap_or_default());
                    out.push('\n');
                    for (key, value) in arguments(call) {
                        let value = match value {
                            Value::String(text) => text,
                            other => python_json(&other),
                        };
                        out.push_str(&format!(
                            "<arg_key>{key}</arg_key>\n<arg_value>{value}</arg_value>\n"
                        ));
                    }
                    out.push_str("</tool_call>");
                }
                out.push_str("<|end_of_turn|>\n");
            }
            Some("tool") => {
                // A run of tool rows is one observation turn.
                if index == 0 || rows[index - 1]["role"] != "tool" {
                    out.push_str("<|start_of_turn|><|observation|>");
                }
                out.push_str("\n<tool_response>\n");
                out.push_str(row["content"].as_str().unwrap_or_default());
                out.push_str("\n</tool_response>");
                if rows
                    .get(index + 1)
                    .is_none_or(|next| next["role"] != "tool")
                {
                    out.push_str("<|end_of_turn|>\n");
                }
            }
            _ => {}
        }
    }
    out.push_str("<|start_of_turn|><|assistant|>\n");
    out
}

const SARVAM_M_THINK: &str =
    "Think deeply before answering the user's question. Do the thinking inside <think>...</think> tags.";

/// sarvam-m's published template (tokenizer_config.json) in the thinking mode
/// Sarvam always runs: it has no off switch.
fn sarvam_m_prompt(rows: &[Value], tools: &[Value]) -> String {
    let (mut system, rows) = match rows.first() {
        Some(row) if row["role"] == "system" => (
            format!("{SARVAM_M_THINK}\n\n{}", visible_text(&row["content"])),
            &rows[1..],
        ),
        _ => (
            format!("You are a helpful assistant. {SARVAM_M_THINK}"),
            rows,
        ),
    };
    // shortcut: the published template has no tool markup (it raises on a tool
    // row), so tools, calls and results count as their bare wire text; render
    // the hosted markup once a live probe shows it.
    for tool in tools {
        system.push_str("\n\n");
        system.push_str(&python_json(tool));
    }
    let mut out = format!("<s>[SYSTEM_PROMPT]{system}[/SYSTEM_PROMPT]");
    for (index, row) in rows.iter().enumerate() {
        match row["role"].as_str() {
            Some("assistant") => {
                let (_, content) = split_think(visible_text(&row["content"]));
                out.push_str(&content);
                for call in row["tool_calls"].as_array().into_iter().flatten() {
                    out.push_str(call["function"]["name"].as_str().unwrap_or_default());
                    out.push_str(call["function"]["arguments"].as_str().unwrap_or_default());
                }
                out.push_str("</s>");
            }
            Some("tool") => out.push_str(row["content"].as_str().unwrap_or_default()),
            _ => {
                out.push_str("[INST]");
                out.push_str(&visible_text(&row["content"]));
                out.push_str("[/INST]");
                if index + 1 == rows.len() {
                    out.push_str("<think>\n");
                }
            }
        }
    }
    out
}

/// The template's `visible_text`: a string as is, the text parts of a part
/// list joined bare (images render nothing), anything else empty.
fn visible_text(content: &Value) -> String {
    match content {
        Value::String(text) => text.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter(|part| part["type"] == "text")
            .filter_map(|part| part["text"].as_str())
            .collect(),
        _ => String::new(),
    }
}

/// Reasoning the assistant text carries inline, split off as the templates
/// do: what precedes the first `</think>` (after any `<think>`), and the text
/// after the last one.
fn split_think(content: String) -> (String, String) {
    if !content.contains("</think>") {
        return (String::new(), content);
    }
    let head = content.split("</think>").next().unwrap_or_default();
    let reasoning = head
        .trim_end_matches('\n')
        .rsplit("<think>")
        .next()
        .unwrap_or_default()
        .trim_start_matches('\n');
    let tail = content.rsplit("</think>").next().unwrap_or_default();
    (
        reasoning.to_string(),
        tail.trim_start_matches('\n').to_string(),
    )
}

/// A call's wire arguments as the object the server renders: it parses the
/// JSON string before applying the template, and a non-object has no keys.
fn arguments(call: &Value) -> serde_json::Map<String, Value> {
    match serde_json::from_str(call["arguments"].as_str().unwrap_or_default()) {
        Ok(Value::Object(map)) => map,
        _ => serde_json::Map::new(),
    }
}

/// JSON as Python's `json.dumps` writes it, which the template's `tojson`
/// uses: a space after every `:` and `,`. A copy of provider-deepseek's until
/// the shared scaffold helper lands.
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
    use llm_router::types::content::ContentBlock;
    use llm_router::types::messages::{UserMessage, UserRoleTag};

    fn user(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            timestamp: 1,
        })
    }

    /// Prompts Sarvam's published templates render (sarvamai/sarvam-30b and
    /// sarvam-105b `chat_template.jinja`, sarvamai/sarvam-m
    /// `tokenizer_config.json`), rendered as transformers does, with tool-call
    /// arguments parsed into objects, keys sorted as serde_json sends them.
    #[test]
    fn requests_render_as_sarvams_templates_render_them() {
        let cases: Value =
            serde_json::from_str(include_str!("../tests/golden/encoding/sarvam_prompts.json"))
                .unwrap();
        for (name, case) in cases.as_object().unwrap() {
            let (_, render) = family(case["model"].as_str().unwrap()).unwrap();
            let rendered = render(
                case["messages"].as_array().unwrap(),
                case["tools"].as_array().unwrap(),
            );
            assert_eq!(rendered, case["prompt"].as_str().unwrap(), "{name}");
        }
    }

    #[test]
    fn sarvam_m_counts_the_tool_text_its_template_cannot_render() {
        let rows = [
            serde_json::json!({"role": "user", "content": "ls"}),
            serde_json::json!({"role": "assistant", "tool_calls": [
                {"id": "c1", "type": "function",
                 "function": {"name": "shell__exec", "arguments": "{\"cmd\":\"ls\"}"}}]}),
            serde_json::json!({"role": "tool", "tool_call_id": "c1", "content": "a.txt"}),
        ];
        let tools = [serde_json::json!({"type": "function", "function": {"name": "shell__exec"}})];
        assert_eq!(
            sarvam_m_prompt(&rows, &tools),
            format!(
                "<s>[SYSTEM_PROMPT]You are a helpful assistant. {SARVAM_M_THINK}\n\n\
                 {{\"function\": {{\"name\": \"shell__exec\"}}, \"type\": \"function\"}}\
                 [/SYSTEM_PROMPT][INST]ls[/INST]shell__exec{{\"cmd\":\"ls\"}}</s>a.txt"
            )
        );
    }

    #[test]
    fn every_sarvam_chat_id_has_a_published_vocabulary() {
        for model in crate::curated::chat_models() {
            assert!(family(&model.id).is_some(), "{}", model.id);
        }
        assert!(family("qwen2.5-coder-7b-instruct").is_none());
    }

    #[tokio::test]
    async fn empty_messages_are_rejected() {
        let req = CountTokensRequest {
            model: "sarvam-105b".into(),
            system_prompt: None,
            tools: None,
            messages: vec![],
        };
        assert!(handle(req).await.is_err());
    }

    #[tokio::test]
    async fn other_ids_keep_the_tiktoken_estimate() {
        let model = "qwen2.5-coder-7b-instruct";
        let req = CountTokensRequest {
            model: model.into(),
            system_prompt: None,
            tools: None,
            messages: vec![user("namaste duniya")],
        };
        let resp = handle(req).await.unwrap();
        let expected = count_chat_tokens(model, None, &[], &[user("namaste duniya")]);
        assert_eq!(resp.tokens, expected);
        assert_eq!(resp.estimator, "tiktoken");
    }
}

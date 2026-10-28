//! The prompt DeepSeek-V4.1 builds from OpenAI-shaped Chat Completions rows,
//! for counting what DeepSeek bills.
//!
//! DeepSeek bills the prompt its encoder renders, not the rows' text: the tool
//! block, the DSML each call becomes and the replayed `reasoning_content` are
//! prompt tokens too. Any provider that reaches a DeepSeek-V4 model — DeepSeek
//! itself, or a gateway in front of it — counts the same render over its own
//! wire rows.

use serde_json::Value;

use crate::provider_scaffold::vocabulary_count::VocabularyRef;

/// Every DeepSeek chat model shares one vocabulary, so the reference is fixed
/// rather than per-model: the id list changes far more often than the
/// tokenizer behind it. V4.1's file has the same vocabulary as V3's plus the
/// prompt's own markers (`<think>`, `｜DSML｜`, `<｜System｜>`) as single tokens.
pub fn vocabulary() -> VocabularyRef {
    VocabularyRef::huggingface("deepseek-ai/DeepSeek-V4.1-Flash")
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
pub fn prompt(rows: &[Value], tools: &[Value]) -> String {
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
        let cases: Value = serde_json::from_str(include_str!(
            "../../tests/golden/encoding/dsv41_prompts.json"
        ))
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

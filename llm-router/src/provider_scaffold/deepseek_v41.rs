//! The prompt DeepSeek-V4.1's published encoder (`encoding/encoding.py` in
//! deepseek-ai/DeepSeek-V4.1-Flash, MIT) renders from OpenAI-shaped chat
//! rows, reduced to the rows a provider's wire sends: system, user,
//! assistant (text, `reasoning_content`, `tool_calls`) and tool results.
//!
//! A provider serving DeepSeek models bills this prompt, not the rows' text:
//! the tool block, the DSML each call becomes and the replayed reasoning are
//! prompt tokens too. Counting it with the model's vocabulary is what makes
//! a local count match the bill.

use serde_json::Value;

use crate::provider_scaffold::python_json::python_json;

pub const BOS: &str = "<｜begin▁of▁sentence｜>";
pub const EOS: &str = "<｜end▁of▁sentence｜>";
pub const SYSTEM: &str = "<｜System｜>";
pub const USER: &str = "<｜User｜>";
pub const ASSISTANT: &str = "<｜Assistant｜>";
pub const THINK_START: &str = "<think>";
pub const THINK_END: &str = "</think>";

/// The tool block appended to the system message, verbatim from the
/// encoder's `TOOLS_TEMPLATE`; the schemas go between header and footer,
/// one `json.dumps` per line.
pub const TOOLS_HEADER: &str = "\n\n## Tools\n\nYou have access to a set of tools to help answer the \
user's question. You can invoke tools by writing a \"<｜DSML｜ calls>\" block like the following:\n\n\
<｜DSML｜ calls>\n<｜DSML｜ invoke name=\"$TOOL_NAME\">\n<｜DSML｜ parameter name=\"$PARAMETER_NAME\" \
string=\"true|false\">$PARAMETER_VALUE</｜DSML｜ parameter>\n...\n</｜DSML｜ invoke>\n<｜DSML｜ invoke \
name=\"$TOOL_NAME2\">\n...\n</｜DSML｜ invoke>\n</｜DSML｜ calls>\n\nString parameters should be \
specified as is and set `string=\"true\"`. For all other types (numbers, booleans, arrays, objects), \
pass the value in JSON format and set `string=\"false\"`.\n\nIf thinking_mode is enabled (triggered \
by <think>), you MUST output your complete reasoning inside <think>...</think> BEFORE any tool calls \
or final response.\n\nOtherwise, output directly after </think> with tool calls or final \
response.\n\n### Available Tool Schemas\n\n";
pub const TOOLS_FOOTER: &str =
    "\n\nYou MUST strictly follow the above defined tool name and parameter \
schemas to invoke tool calls.\n";

/// What the encoder's `response_format_template` puts before the format's
/// JSON, after the system message's tool block.
pub const RESPONSE_FORMAT_HEADER: &str =
    "\n\n## Response Format:\n\nYou MUST strictly adhere to the following schema to reply:\n";

/// The budget the encoder renders when no effort is given (`high`), which
/// is also what DeepSeek's API runs with no `reasoning_effort`.
pub const DEFAULT_EFFORT_BUDGET: u8 = 75;

/// The encoder's `REASONING_EFFORT_MAPPINGS`: DeepSeek's named efforts as
/// the numeric budget the prompt's first line carries. `None` for a name
/// the encoder does not know; callers map their own ladder onto these three.
pub fn effort_budget(effort: &str) -> Option<u8> {
    match effort {
        "low" => Some(50),
        "high" => Some(75),
        "max" => Some(100),
        _ => None,
    }
}

/// The encoder's `thinking_mode`: `chat` when the request disables
/// thinking, otherwise `thinking` at a reasoning-effort budget (1-100).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Chat,
    Thinking { budget: u8 },
}

/// The line that opens every thinking-mode prompt, after `<｜System｜>`.
pub fn reasoning_effort(budget: u8) -> String {
    format!(
        "Reasoning Effort: {budget} (range 1-100, the higher the value, the more thorough \
         the reasoning)\n\n"
    )
}

/// A turn of the encoded conversation. The encoder has no tool role: results,
/// and user text sent right after them, merge into one user turn.
enum Turn<'a> {
    System(&'a str),
    User { parts: Vec<String>, text: bool },
    Assistant(&'a Value),
}

/// The prompt the encoder renders for OpenAI-shaped `rows`, `tools`
/// (`{"type": "function", "function": {...}}` each) and the body's
/// `response_format` (e.g. `{"type": "json_object"}`) in `mode`.
pub fn prompt(
    rows: &[Value],
    tools: &[Value],
    response_format: Option<&Value>,
    mode: Mode,
) -> String {
    let thinking = matches!(mode, Mode::Thinking { .. });
    let mut turns = Vec::new();
    // Tools and the response format ride on the system message; a request
    // without one gets an empty one.
    if (!tools.is_empty() || response_format.is_some())
        && rows.first().is_none_or(|row| row["role"] != "system")
    {
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
    // In thinking mode, with tools the encoder keeps every turn's reasoning;
    // without, only the reasoning after the last user turn. Chat mode
    // replays none.
    let reasons =
        |index: usize| thinking && (!tools.is_empty() || last_user.is_none_or(|user| index > user));

    let mut out = String::from(BOS);
    match mode {
        Mode::Thinking { budget } => {
            out.push_str(SYSTEM);
            out.push_str(&reasoning_effort(budget));
        }
        Mode::Chat if matches!(turns.first(), Some(Turn::System(_))) => out.push_str(SYSTEM),
        Mode::Chat => {}
    }
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
                if let Some(format) = response_format {
                    out.push_str(RESPONSE_FORMAT_HEADER);
                    out.push_str(&python_json(format));
                }
            }
            Turn::User { parts, .. } => {
                out.push_str(USER);
                out.push_str(&parts.join("\n\n"));
                if !matches!(
                    turns.get(index + 1),
                    Some(Turn::User { .. } | Turn::System(_))
                ) {
                    out.push_str(ASSISTANT);
                    let opens = thinking
                        && (!tools.is_empty() || last_user.is_none_or(|user| index >= user));
                    out.push_str(if opens { THINK_START } else { THINK_END });
                }
            }
            Turn::Assistant(row) => {
                if reasons(index) {
                    out.push_str(row["reasoning_content"].as_str().unwrap_or_default());
                    out.push_str(THINK_END);
                }
                out.push_str(row["content"].as_str().unwrap_or_default());
                if let Some(calls) = row["tool_calls"].as_array() {
                    let invokes: Vec<String> = calls.iter().map(invoke).collect();
                    out.push_str("\n\n<｜DSML｜ calls>\n");
                    out.push_str(&invokes.join("\n"));
                    out.push_str("\n</｜DSML｜ calls>");
                }
                out.push_str(EOS);
            }
        }
    }
    out
}

/// One OpenAI-shaped call as DSML: a parameter tag per argument, strings
/// bare and every other value as JSON.
pub fn invoke(call: &Value) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn cases(golden: &str) -> Vec<(String, Value)> {
        let cases: Value = serde_json::from_str(golden).unwrap();
        cases.as_object().unwrap().clone().into_iter().collect()
    }

    fn render(case: &Value, mode: Mode) -> String {
        prompt(
            case["messages"].as_array().unwrap(),
            case["tools"].as_array().unwrap(),
            case.get("response_format"),
            mode,
        )
    }

    /// Prompts the encoder renders in thinking mode at its default effort:
    /// its own tool-calling case and an agent-shaped one, keys sorted as
    /// serde_json sends them.
    #[test]
    fn requests_render_as_deepseeks_encoder_renders_them() {
        for (name, case) in cases(include_str!(
            "../../tests/golden/encoding/dsv41_prompts.json"
        )) {
            let mode = Mode::Thinking {
                budget: DEFAULT_EFFORT_BUDGET,
            };
            assert_eq!(
                render(&case, mode),
                case["prompt"].as_str().unwrap(),
                "{name}"
            );
        }
    }

    /// The same cases (plus one without tools) rendered by the encoder in
    /// chat mode and at the `low` and `max` efforts, and the agent case with
    /// a `json_object` response format.
    #[test]
    fn chat_mode_and_every_effort_render_as_the_encoder_renders_them() {
        for (name, case) in cases(include_str!("../../tests/golden/encoding/dsv41_modes.json")) {
            let mode = match case["reasoning_effort"].as_str() {
                None => Mode::Chat,
                Some(effort) => Mode::Thinking {
                    budget: effort_budget(effort).unwrap(),
                },
            };
            assert_eq!(
                case["thinking_mode"] == "chat",
                mode == Mode::Chat,
                "{name}"
            );
            assert_eq!(
                render(&case, mode),
                case["prompt"].as_str().unwrap(),
                "{name}"
            );
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
        let prefix = reasoning_effort(DEFAULT_EFFORT_BUDGET);
        assert_eq!(
            prompt(&rows, &[], None, Mode::Thinking { budget: 75 }),
            format!(
                "<｜begin▁of▁sentence｜><｜System｜>{prefix}<｜User｜>Hello<｜Assistant｜></think>Hi!\
                 <｜end▁of▁sentence｜><｜User｜>And again?\n\nTwice.<｜Assistant｜></think>Sure.\
                 <｜end▁of▁sentence｜><｜User｜>Last one.<｜Assistant｜><think>"
            )
        );
    }

    #[test]
    fn only_the_three_named_efforts_have_a_budget() {
        assert_eq!(effort_budget("low"), Some(50));
        assert_eq!(effort_budget("high"), Some(DEFAULT_EFFORT_BUDGET));
        assert_eq!(effort_budget("max"), Some(100));
        assert_eq!(effort_budget("medium"), None);
    }
}

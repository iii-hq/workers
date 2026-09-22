//! SemIf's frozen direct prompt (`semif_phase1/core.py` + `direct.py`):
//! a system instruction, `{"evidence", "criterion", "options"}` as Python
//! `json.dumps(ensure_ascii=False)`, the Qwen chat template with thinking
//! disabled, and the uppercase option letters read from the next token.
use serde_json::{json, Value};

pub const SYSTEM: &str = "Apply the supplied criterion to the supplied evidence. Choose exactly one listed option. Respond with only its uppercase letter, with no explanation or reasoning.";
pub const LETTERS: &str = "ABCDEFGHIJKLMNOP";
/// SemIf accepts 2-16 options, one letter each.
pub const MAX_OPTIONS: usize = LETTERS.len();

/// Python `json.dumps` spelling: `", "` and `": "` separators. The model saw
/// prompts built this way; every space is a token.
struct PythonJson;
impl serde_json::ser::Formatter for PythonJson {
    fn begin_object_key<W: ?Sized + std::io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        if first {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }
    fn begin_object_value<W: ?Sized + std::io::Write>(&mut self, w: &mut W) -> std::io::Result<()> {
        w.write_all(b": ")
    }
    fn begin_array_value<W: ?Sized + std::io::Write>(
        &mut self,
        w: &mut W,
        first: bool,
    ) -> std::io::Result<()> {
        if first {
            Ok(())
        } else {
            w.write_all(b", ")
        }
    }
}

pub fn python_json(value: &Value) -> String {
    let mut out = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, PythonJson);
    serde::Serialize::serialize(value, &mut serializer).expect("JSON values serialize");
    String::from_utf8(out).expect("serde_json writes UTF-8")
}

/// Qwen chat template, `add_generation_prompt=True, enable_thinking=False`.
fn chat(user: &str) -> String {
    format!(
        "<|im_start|>system\n{SYSTEM}<|im_end|>\n<|im_start|>user\n{user}<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\n"
    )
}

/// The full prompt for one decision over `options` (option descriptions).
pub fn render(state: &Value, criterion: &str, options: &[String]) -> String {
    let options: Vec<Value> = options
        .iter()
        .enumerate()
        .map(|(i, description)| json!({"letter": &LETTERS[i..=i], "description": description}))
        .collect();
    // preserve_order keeps SemIf's key order: evidence first, so every
    // question about one state shares the prompt up to the evidence.
    chat(&python_json(
        &json!({"evidence": state, "criterion": criterion, "options": options}),
    ))
}

/// The prompt text every question about `state` starts with (SemIf's
/// `_state_prefix`): the template head plus `{"evidence": <state>`. The
/// caller drops the prefix's last token, which can merge with what follows.
pub fn state_prefix(state: &Value) -> String {
    let evidence = python_json(&json!({"evidence": state}));
    let head = chat("");
    let user_at = head
        .find("<|im_end|>\n<|im_start|>assistant")
        .expect("template has a user turn");
    format!("{}{}", &head[..user_at], &evidence[..evidence.len() - 1])
}

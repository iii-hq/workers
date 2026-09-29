//! decider's plain state-first prompt (`decider/prompt_fast.py` and
//! `decider/systemone.py`, decider-ai 1.5.0): `Context:\n<state>`, then one
//! `\n\nQuestion: …\nOptions:\n(A) …\nAnswer: (` block per decision; the
//! answer is read at the final ` (`. The context and each question block are
//! tokenized separately, as decider does.
use serde_json::{json, Value};

/// decider's widest label head: `A`–`Z`, then the two-letter labels that are
/// one token each.
pub const MAX_OPTIONS: usize = iii_llama_runtime::scorer::MAX_LABELS;
/// Up to ten options render as one string, `(A)`–`(J)`.
const NARROW: usize = 10;
/// The question of a noul without instructions (`NOUL_WITHOUT_INSTRUCTIONS`);
/// its options carry what is asked.
pub const NO_INSTRUCTIONS: &str = "Which answer fits the context?";

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

/// `render_state`: strings as-is, other values as Python JSON with every
/// element of an 8+ element array carrying its `_index`.
pub fn render_state(state: &Value) -> String {
    match state {
        Value::String(s) => s.clone(),
        other => python_json(&annotate_indices(other)),
    }
}

fn annotate_indices(value: &Value) -> Value {
    match value {
        Value::Array(items) if items.len() >= 8 => items
            .iter()
            .enumerate()
            .map(|(i, item)| match annotate_indices(item) {
                Value::Object(fields) => {
                    let mut out = serde_json::Map::from_iter([("_index".into(), json!(i))]);
                    out.extend(fields);
                    Value::Object(out)
                }
                other => json!({"_index": i, "value": other}),
            })
            .collect(),
        Value::Array(items) => items.iter().map(annotate_indices).collect(),
        Value::Object(fields) => fields
            .iter()
            .map(|(k, v)| (k.clone(), annotate_indices(v)))
            .collect(),
        other => other.clone(),
    }
}

/// One question block, split where decider tokenizes separately: `Whole` is
/// one string; `Wide` is the head, each option's text after its `\n(` and
/// label tokens, and the tail.
pub enum Piece {
    Whole(String),
    Wide {
        head: String,
        options: Vec<String>,
        tail: &'static str,
    },
}

/// `question_piece` for one independent question (no `Question 1:` numbering).
pub fn question_piece(question: &str, options: &[String]) -> Piece {
    let head = format!("\n\nQuestion: {question}\nOptions:");
    const TAIL: &str = "\nAnswer: (";
    if options.len() <= NARROW {
        let lines: String = options
            .iter()
            .zip('A'..)
            .map(|(option, letter)| format!("\n({letter}) {option}"))
            .collect();
        Piece::Whole(format!("{head}{lines}{TAIL}"))
    } else {
        Piece::Wide {
            head,
            options: options.iter().map(|o| format!(") {o}")).collect(),
            tail: TAIL,
        }
    }
}

/// `isolated_rows`: a Score level judged on its own, without its number or
/// its neighbours, as a yes/no question.
pub fn level_question(question: &str, level: &str) -> String {
    format!(
        "{question}\nProposed answer: {}\nDoes the proposed answer fit?",
        strip_level_number(level)
    )
}

/// `re.sub(r"^\s*-?\d+\s*:\s*", "", text)`: "2: somewhat" -> "somewhat".
fn strip_level_number(text: &str) -> &str {
    let rest = text.trim_start();
    let rest = rest.strip_prefix('-').unwrap_or(rest);
    let digits = rest.trim_start_matches(|c: char| c.is_ascii_digit());
    if digits.len() == rest.len() {
        return text;
    }
    match digits.trim_start().strip_prefix(':') {
        Some(after) => after.trim_start(),
        None => text,
    }
}

//! Clef's record encoding (`joint_schema_model.py` `encode_record`, `render`,
//! `question_options`): hand-written ChatML around the state, then a schema
//! listing every field and its options. Each piece is tokenized on its own,
//! as the reference does; the head reads the question and option spans.
use judge_contract::{ErrorCode, Evaluation, Question};
use serde::Serialize;
use serde_json::{json, Value};
use std::ops::Range;

pub const PREFIX: &str = "<|im_start|>system\nRead the complete state and schema. Decide every field jointly. Each answer must be exactly one of that field's allowed options.<|im_end|>\n<|im_start|>user\nSTATE:\n";
const SUFFIX: &str =
    "\n<|im_end|>\n<|im_start|>assistant\n<think>\n\n</think>\n\nJOINT SCHEMA DECISIONS:";

/// What a schema piece is to the head.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Text,
    /// The question's instruction span; noul 0, choice 1, score 2.
    Question(u32),
    /// One option's `{"description":…,"option_id":…}` span.
    Option,
}

/// One question as laid out in the prompt: [start, end) token ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// noul 0, choice 1, score 2 (the head's type embedding row).
    pub kind: u32,
    pub span: Range<usize>,
    pub options: Vec<Range<usize>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Encoded {
    pub ids: Vec<u32>,
    /// One per question, in the evaluation's (sorted) question order.
    pub fields: Vec<Field>,
    /// State tokens cut to fit the window.
    pub dropped: usize,
}

/// Python's float `repr`, which `json.dumps` writes. ryu picks the same
/// shortest digits, breaking a tie between two of them to even as CPython
/// does (`{:?}` rounds it up: 637.1682739257813 for Python's
/// 637.1682739257812), and switches to an exponent from 1e16; Python also
/// uses one below 1e-4, and signs and pads it to two digits: `1e-05`, `1e+16`.
struct PythonFloats;
impl serde_json::ser::Formatter for PythonFloats {
    fn write_f64<W: ?Sized + std::io::Write>(
        &mut self,
        w: &mut W,
        value: f64,
    ) -> std::io::Result<()> {
        let mut buf = ryu::Buffer::new();
        let repr = buf.format_finite(value);
        let (neg, body) = repr
            .strip_prefix('-')
            .map_or(("", repr), |body| ("-", body));
        // ryu keeps 1e-5 <= |v| < 1e-4 in fixed notation; Python writes e-05 there.
        if let Some(digits) = body.strip_prefix("0.0000") {
            let (head, tail) = digits.split_at(1);
            return if tail.is_empty() {
                write!(w, "{neg}{head}e-05")
            } else {
                write!(w, "{neg}{head}.{tail}e-05")
            };
        }
        match repr.split_once('e') {
            None => w.write_all(repr.as_bytes()),
            Some((digits, exponent)) => {
                let (sign, exponent) = exponent
                    .strip_prefix('-')
                    .map_or(("+", exponent), |exponent| ("-", exponent));
                write!(w, "{digits}e{sign}{exponent:0>2}")
            }
        }
    }
}

/// `render`: a string as is, anything else as `json.dumps(sort_keys=True,
/// separators=(",", ":"), ensure_ascii=False)`.
fn render(value: &Value) -> String {
    if let Value::String(text) = value {
        return text.clone();
    }
    let mut value = value.clone();
    value.sort_all_objects();
    let mut out = Vec::new();
    let mut serializer = serde_json::Serializer::with_formatter(&mut out, PythonFloats);
    value
        .serialize(&mut serializer)
        .expect("JSON values serialize");
    String::from_utf8(out).expect("serde_json writes UTF-8")
}

fn value(content: &impl Serialize) -> Value {
    serde_json::to_value(content).expect("contract values are JSON")
}

/// The question's kind and its options in prompt order: noul `[true, false]`
/// (default descriptions), choice sorted by id, score `"0".."n-1"`. A `Null`
/// description is left out of the option JSON.
pub fn options(question: &Question) -> (u32, Vec<(String, Value)>) {
    match question {
        // `criteria.update(...)` over the defaults: a given null drops the description.
        Question::Noul { criteria, .. } => (
            0,
            [
                ("true", "The proposition is true or the answer is yes."),
                ("false", "The proposition is false or the answer is no."),
            ]
            .map(|(id, default)| {
                let given = criteria.as_ref().and_then(|criteria| criteria.get(id));
                (id.to_owned(), given.map_or_else(|| default.into(), value))
            })
            .into(),
        ),
        Question::Choice { criteria, .. } => (
            1,
            criteria
                .iter()
                .map(|(id, c)| (id.clone(), value(c)))
                .collect(),
        ),
        Question::Score { criteria, .. } => (
            2,
            criteria
                .iter()
                .enumerate()
                .map(|(index, level)| (index.to_string(), value(level)))
                .collect(),
        ),
    }
}

/// The rendered state, then every later piece in prompt order (schema through
/// suffix); `PREFIX` precedes both.
pub fn pieces(evaluation: &Evaluation) -> (String, Vec<(Role, String)>) {
    let mut out = vec![(Role::Text, "\n\nSCHEMA FIELDS:\n".to_owned())];
    for (n, (id, question)) in evaluation.questions.iter().enumerate() {
        let (Question::Noul { instructions, .. }
        | Question::Choice { instructions, .. }
        | Question::Score { instructions, .. }) = question;
        let (kind, options) = options(question);
        let name = ["noul", "choice", "score"][kind as usize];
        out.push((
            Role::Text,
            format!("\nFIELD {}\nID: {id}\nTYPE: {name}\nINSTRUCTION: ", n + 1),
        ));
        let instructions = match value(instructions) {
            instructions if instructions.is_null() || instructions == "" => id.clone(),
            instructions => render(&instructions),
        };
        out.push((Role::Question(kind), instructions));
        out.push((Role::Text, "\nALLOWED OPTIONS:\n".to_owned()));
        for (j, (option_id, description)) in options.into_iter().enumerate() {
            let mut option = json!({ "option_id": option_id });
            if !description.is_null() {
                option["description"] = description;
            }
            out.extend([
                (Role::Text, format!("OPTION {}: ", j + 1)),
                (Role::Option, render(&option)),
                (Role::Text, "\n".to_owned()),
            ]);
        }
        out.push((Role::Text, "END FIELD\n".to_owned()));
    }
    out.push((Role::Text, SUFFIX.to_owned()));
    (render(&evaluation.state), out)
}

/// `encode_record(max_length = max_tokens)`: each piece tokenized on its own,
/// only the state truncated (its head kept); a schema that alone exceeds
/// `max_tokens` is `payload_too_large`, an empty question span
/// `invalid_request`.
pub fn encode(
    evaluation: &Evaluation,
    max_tokens: usize,
    tokenize: impl Fn(&str) -> Result<Vec<u32>, ErrorCode>,
) -> Result<Encoded, ErrorCode> {
    let (state, schema_pieces) = pieces(evaluation);
    let mut ids = tokenize(PREFIX)?;
    let (mut schema, mut fields) = (Vec::new(), Vec::<Field>::new());
    for (role, text) in schema_pieces {
        let piece = tokenize(&text)?;
        let span = schema.len()..schema.len() + piece.len();
        match role {
            // The reference averages an empty span into NaN, which spreads to every field.
            Role::Question(_) if span.is_empty() => return Err(ErrorCode::InvalidRequest),
            Role::Question(kind) => fields.push(Field {
                kind,
                span,
                options: Vec::new(),
            }),
            Role::Option => fields
                .last_mut()
                .expect("options follow their question")
                .options
                .push(span),
            Role::Text => {}
        }
        schema.extend(piece);
    }
    let room = max_tokens
        .checked_sub(ids.len() + schema.len())
        .ok_or(ErrorCode::PayloadTooLarge)?;
    let mut state = tokenize(&state)?;
    let dropped = state.len().saturating_sub(room);
    state.truncate(room);
    let offset = ids.len() + state.len();
    for field in &mut fields {
        for span in std::iter::once(&mut field.span).chain(&mut field.options) {
            *span = span.start + offset..span.end + offset;
        }
    }
    ids.extend(state);
    ids.extend(schema);
    Ok(Encoded {
        ids,
        fields,
        dropped,
    })
}

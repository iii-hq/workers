//! Clef's record encoding (`joint_schema_model.py` `encode_record`, `render`,
//! `question_options`). STUB: the interface is fixed; the body is implemented
//! in its own change.
use judge_contract::{ErrorCode, Evaluation, Question};
use serde_json::Value;
use std::ops::Range;

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

/// The question's kind and its options in prompt order: noul `[true, false]`
/// (default descriptions), choice sorted by id, score `"0".."n-1"`. A `Null`
/// description is left out of the option JSON.
pub fn options(question: &Question) -> (u32, Vec<(String, Value)>) {
    let _ = question;
    todo!("encode::options")
}

/// The rendered state, then every later piece in prompt order (schema through
/// suffix); `PREFIX` precedes both.
pub fn pieces(evaluation: &Evaluation) -> (String, Vec<(Role, String)>) {
    let _ = evaluation;
    todo!("encode::pieces")
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
    let _ = (evaluation, max_tokens, tokenize);
    todo!("encode::encode")
}

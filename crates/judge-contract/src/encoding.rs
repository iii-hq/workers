use crate::{
    Content, EncodingLimits, ErrorCode, EvaluateRequest, Evaluation, Question, ScoreLevel,
    MAX_EVALUATIONS,
};
use serde::Serialize;
use serde_json::Value;

pub fn validate_request(request: &EvaluateRequest) -> Result<(), ErrorCode> {
    validate_request_with_limits(request, EncodingLimits::default())
}
pub fn validate_request_with_limits(
    request: &EvaluateRequest,
    limits: EncodingLimits,
) -> Result<(), ErrorCode> {
    if limits.max_body_bytes == 0 {
        return Err(ErrorCode::InvalidRequest);
    }
    if request
        .model
        .as_ref()
        .is_some_and(|model| model.len() > limits.max_body_bytes)
    {
        return Err(ErrorCode::PayloadTooLarge);
    }
    if request.timeout_ms == 0
        || request.model.as_ref().is_some_and(|m| m.trim().is_empty())
        || request.evaluations.is_empty()
        || request.evaluations.len() > MAX_EVALUATIONS
    {
        return Err(ErrorCode::InvalidRequest);
    }
    let mut ids = std::collections::HashSet::new();
    for evaluation in &request.evaluations {
        // Bound IDs before scanning/hashing them for uniqueness.
        validate_evaluation(evaluation, limits)?;
        if !ids.insert(&evaluation.id) {
            return Err(ErrorCode::InvalidRequest);
        }
    }
    Ok(())
}
fn validate_evaluation(evaluation: &Evaluation, limits: EncodingLimits) -> Result<(), ErrorCode> {
    if evaluation.id.len() > limits.max_body_bytes
        || evaluation.questions.len() > limits.max_body_bytes
    {
        return Err(ErrorCode::PayloadTooLarge);
    }
    if evaluation.id.trim().is_empty()
        || evaluation.questions.is_empty()
        || !matches!(
            evaluation.state,
            Value::String(_) | Value::Object(_) | Value::Array(_)
        )
    {
        return Err(ErrorCode::InvalidRequest);
    }
    let mut remaining = limits.max_body_bytes;
    state_lower_bound(&evaluation.state, &mut remaining, 0)?;
    for (id, question) in &evaluation.questions {
        charge(&mut remaining, id.len().saturating_add(4))?;
        content_lower_bound(question.instructions(), &mut remaining)?;
        match question {
            Question::Noul {
                criteria: Some(criteria),
                ..
            } => {
                if criteria.len() > 2 || criteria.keys().any(|key| key != "true" && key != "false")
                {
                    return Err(ErrorCode::InvalidRequest);
                }
                for (key, value) in criteria {
                    charge(&mut remaining, key.len())?;
                    content_lower_bound(value, &mut remaining)?;
                }
            }
            Question::Noul { criteria: None, .. } => {}
            Question::Choice { criteria, .. } => {
                if !(1..=255).contains(&criteria.len()) {
                    return Err(ErrorCode::InvalidRequest);
                }
                for (key, value) in criteria {
                    charge(&mut remaining, key.len().saturating_add(4))?;
                    content_lower_bound(value, &mut remaining)?;
                }
            }
            Question::Score { criteria, .. } => {
                if !(2..=10).contains(&criteria.len()) {
                    return Err(ErrorCode::InvalidRequest);
                }
                for value in criteria {
                    match value {
                        ScoreLevel::Text(text) => {
                            charge(&mut remaining, text.len().saturating_add(2))?
                        }
                        ScoreLevel::Object(values) => {
                            charge(&mut remaining, 2)?;
                            for (key, value) in values {
                                charge(&mut remaining, key.len().saturating_add(3))?;
                                state_lower_bound(value, &mut remaining, 1)?;
                            }
                        }
                        ScoreLevel::Array(values) => {
                            charge(&mut remaining, 2)?;
                            for value in values {
                                state_lower_bound(value, &mut remaining, 1)?;
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}

pub fn encode_evaluation(model: &str, evaluation: &Evaluation) -> Result<Vec<u8>, ErrorCode> {
    encode_evaluation_with_limits(model, evaluation, EncodingLimits::default())
}
/// IDs and deadlines never leave iii. Byte guards are transport policy; provider
/// context limits are measured in tokens and enforced by the provider.
pub fn encode_evaluation_with_limits(
    model: &str,
    evaluation: &Evaluation,
    limits: EncodingLimits,
) -> Result<Vec<u8>, ErrorCode> {
    if model.len() > limits.max_body_bytes {
        return Err(ErrorCode::PayloadTooLarge);
    }
    if model.trim().is_empty() || limits.max_body_bytes == 0 {
        return Err(ErrorCode::InvalidRequest);
    }
    validate_evaluation(evaluation, limits)?;
    #[derive(Serialize)]
    struct Upstream<'a> {
        model: &'a str,
        state: &'a Value,
        questions: &'a std::collections::BTreeMap<String, Question>,
    }
    if let Some(cap) = limits.max_state_question_bytes {
        let state_bytes = bounded_json(&evaluation.state, cap)?.len();
        for question in evaluation.questions.values() {
            bounded_json(question, cap - state_bytes)?;
        }
    }
    bounded_json(
        &Upstream {
            model,
            state: &evaluation.state,
            questions: &evaluation.questions,
        },
        limits.max_body_bytes,
    )
}
fn charge(remaining: &mut usize, bytes: usize) -> Result<(), ErrorCode> {
    *remaining = remaining
        .checked_sub(bytes)
        .ok_or(ErrorCode::PayloadTooLarge)?;
    Ok(())
}
fn content_lower_bound(content: &Content, remaining: &mut usize) -> Result<(), ErrorCode> {
    match content {
        Content::Text(text) => charge(remaining, text.len().saturating_add(2)),
        Content::Null => charge(remaining, 4),
        Content::Array(values) => {
            charge(remaining, 2)?;
            for value in values {
                state_lower_bound(value, remaining, 1)?;
                charge(remaining, 1)?;
            }
            Ok(())
        }
        Content::Object(values) => {
            charge(remaining, 2)?;
            for (key, value) in values {
                charge(remaining, key.len().saturating_add(3))?;
                state_lower_bound(value, remaining, 1)?;
            }
            Ok(())
        }
    }
}
fn state_lower_bound(value: &Value, remaining: &mut usize, depth: usize) -> Result<(), ErrorCode> {
    if depth >= 128 {
        return Err(ErrorCode::PayloadTooLarge);
    }
    match value {
        Value::String(text) => charge(remaining, text.len().saturating_add(2)),
        Value::Array(values) => {
            charge(remaining, 2)?;
            for (i, value) in values.iter().enumerate() {
                charge(remaining, usize::from(i > 0))?;
                state_lower_bound(value, remaining, depth + 1)?;
            }
            Ok(())
        }
        Value::Object(values) => {
            charge(remaining, 2)?;
            for (i, (key, value)) in values.iter().enumerate() {
                charge(remaining, key.len().saturating_add(3 + usize::from(i > 0)))?;
                state_lower_bound(value, remaining, depth + 1)?;
            }
            Ok(())
        }
        _ => charge(remaining, 1),
    }
}
struct BoundedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}
impl std::io::Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        if bytes.len() > self.limit - self.bytes.len() {
            return Err(std::io::Error::other("JEV payload budget exceeded"));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn bounded_json(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, ErrorCode> {
    let mut writer = BoundedBuffer {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut writer, value).map_err(|error| {
        if error.is_io() {
            ErrorCode::PayloadTooLarge
        } else {
            ErrorCode::InvalidRequest
        }
    })?;
    Ok(writer.bytes)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    #[test]
    fn oversized_write_leaves_buffer_unchanged() {
        let mut writer = BoundedBuffer {
            bytes: Vec::new(),
            limit: 16,
        };
        writer.write_all(b"prefix").unwrap();
        assert!(writer.write_all(&[b'x'; 1024]).is_err());
        assert_eq!(writer.bytes, b"prefix");
        assert!(writer.bytes.capacity() <= 16);
    }
}

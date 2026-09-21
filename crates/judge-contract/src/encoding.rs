use crate::{
    ErrorCode, EvaluateRequest, Evaluation, Question, DEFAULT_MAX_REQUEST_BYTES, MAX_EVALUATIONS,
};
use serde::Serialize;
use serde_json::Value;

pub fn validate_request(request: &EvaluateRequest) -> Result<(), ErrorCode> {
    validate_request_with_limits(request, DEFAULT_MAX_REQUEST_BYTES)
}
/// Structural validation; byte limits are enforced while encoding each body.
pub fn validate_request_with_limits(
    request: &EvaluateRequest,
    max_body_bytes: usize,
) -> Result<(), ErrorCode> {
    if max_body_bytes == 0 {
        return Err(ErrorCode::InvalidRequest);
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
        validate_evaluation(evaluation)?;
        if !ids.insert(&evaluation.id) {
            return Err(ErrorCode::InvalidRequest);
        }
    }
    Ok(())
}
fn validate_evaluation(evaluation: &Evaluation) -> Result<(), ErrorCode> {
    if evaluation.id.trim().is_empty()
        || evaluation.questions.is_empty()
        || !matches!(
            evaluation.state,
            Value::String(_) | Value::Object(_) | Value::Array(_)
        )
    {
        return Err(ErrorCode::InvalidRequest);
    }
    for question in evaluation.questions.values() {
        let valid = match question {
            Question::Noul { criteria, .. } => criteria.as_ref().is_none_or(|criteria| {
                criteria.len() <= 2 && criteria.keys().all(|key| key == "true" || key == "false")
            }),
            Question::Choice { criteria, .. } => (1..=255).contains(&criteria.len()),
            Question::Score { criteria, .. } => (2..=10).contains(&criteria.len()),
        };
        if !valid {
            return Err(ErrorCode::InvalidRequest);
        }
    }
    Ok(())
}

pub fn encode_evaluation(model: &str, evaluation: &Evaluation) -> Result<Vec<u8>, ErrorCode> {
    encode_evaluation_with_limits(model, evaluation, DEFAULT_MAX_REQUEST_BYTES)
}
/// IDs and deadlines never leave iii. The byte guard is transport policy; provider
/// context limits are measured in tokens and enforced by the provider.
pub fn encode_evaluation_with_limits(
    model: &str,
    evaluation: &Evaluation,
    max_body_bytes: usize,
) -> Result<Vec<u8>, ErrorCode> {
    if model.trim().is_empty() || max_body_bytes == 0 {
        return Err(ErrorCode::InvalidRequest);
    }
    validate_evaluation(evaluation)?;
    #[derive(Serialize)]
    struct Upstream<'a> {
        model: &'a str,
        state: &'a Value,
        questions: &'a std::collections::BTreeMap<String, Question>,
    }
    let mut writer = BoundedBuffer {
        bytes: Vec::new(),
        limit: max_body_bytes,
    };
    serde_json::to_writer(
        &mut writer,
        &Upstream {
            model,
            state: &evaluation.state,
            questions: &evaluation.questions,
        },
    )
    .map_err(|error| {
        if error.is_io() {
            ErrorCode::PayloadTooLarge
        } else {
            ErrorCode::InvalidRequest
        }
    })?;
    Ok(writer.bytes)
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

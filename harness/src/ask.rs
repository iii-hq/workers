//! `harness::ask` — structured questions the agent puts to the user (KAN-7).
//!
//! The model calls `harness::ask` instead of writing a question in markdown;
//! the console renders the questions as a card of clickable options and the
//! user's pick arrives as their next message. This module owns the request
//! and response shapes, their validation, the results the turn loop records,
//! and the direct-call entry (always an error: an ask only means something
//! inside a turn). No engine, no I/O.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::HarnessError;
use crate::trigger::ResultData;
use crate::types::content::ContentBlock;

/// Fewest questions one `harness::ask` call may carry.
pub const MIN_QUESTIONS: usize = 1;
/// Most questions one `harness::ask` call may carry.
pub const MAX_QUESTIONS: usize = 4;
/// Fewest options a question may offer (the UI adds a free-text "Other").
pub const MIN_OPTIONS: usize = 2;
/// Most options a question may offer.
pub const MAX_OPTIONS: usize = 4;
/// Longest `header`, in characters (not bytes).
pub const MAX_HEADER_CHARS: usize = 16;
/// Longest option `label`, in characters (not bytes).
pub const MAX_LABEL_CHARS: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AskRequest {
    /// 1-4 questions shown together on one card; the answer arrives as the
    /// user's next message, one line per question.
    #[schemars(length(min = 1, max = 4))]
    pub questions: Vec<AskQuestion>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AskQuestion {
    /// Short chip label such as `Approach`, 1-16 characters; prefixes the
    /// answer line (`Approach: <label>`).
    #[schemars(length(min = 1, max = 16))]
    pub header: String,
    /// The full question text shown above the options.
    #[schemars(length(min = 1))]
    pub question: String,
    /// Let the user pick several options (checkboxes) instead of one.
    #[serde(default)]
    pub multi_select: bool,
    /// 2-4 discrete choices; do not add an "Other" option, the UI adds a
    /// free-text one.
    #[schemars(length(min = 2, max = 4))]
    pub options: Vec<AskOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AskOption {
    /// The choice as the user sees and sends it, 1-80 characters, unique
    /// within its question.
    #[schemars(length(min = 1, max = 80))]
    pub label: String,
    /// Optional one-line explanation shown under the label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AskStatus {
    /// The card is shown; the answer arrives as the user's next message.
    AwaitingAnswer,
}

/// What an accepted ask records as its result `details`: the card the UI
/// draws and the ids it uses to tell an open question from an answered one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AskResponse {
    pub status: AskStatus,
    /// The function-call id of this ask.
    pub question_id: String,
    pub session_id: String,
    /// The turn that asked; once the session's current turn differs, the
    /// question has been answered.
    pub turn_id: String,
    /// The questions as shown, in request order.
    pub questions: Vec<AskQuestion>,
}

/// Check a request against the `harness::ask` limits. The error names the
/// violated limit and the offending question index, so the model can fix
/// its arguments and call again.
pub fn validate(req: &AskRequest) -> Result<(), String> {
    let count = req.questions.len();
    if !(MIN_QUESTIONS..=MAX_QUESTIONS).contains(&count) {
        return Err(format!(
            "questions must have {MIN_QUESTIONS}-{MAX_QUESTIONS} entries (got {count})"
        ));
    }
    for (qi, q) in req.questions.iter().enumerate() {
        validate_question(qi, q)?;
    }
    Ok(())
}

fn validate_question(qi: usize, q: &AskQuestion) -> Result<(), String> {
    let at = format!("questions[{qi}]");
    check_text(&format!("{at}.header"), &q.header, Some(MAX_HEADER_CHARS))?;
    check_text(&format!("{at}.question"), &q.question, None)?;
    let count = q.options.len();
    if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&count) {
        return Err(format!(
            "{at}.options must have {MIN_OPTIONS}-{MAX_OPTIONS} entries (got {count})"
        ));
    }
    for (oi, opt) in q.options.iter().enumerate() {
        let field = format!("{at}.options[{oi}].label");
        check_text(&field, &opt.label, Some(MAX_LABEL_CHARS))?;
        let label = opt.label.trim();
        if let Some(first) = q.options[..oi]
            .iter()
            .position(|earlier| earlier.label.trim() == label)
        {
            return Err(format!(
                "{field} {label:?} duplicates {at}.options[{first}].label; labels must be unique within a question"
            ));
        }
    }
    Ok(())
}

/// Non-empty after trimming and, when `max` is set, at most `max` characters
/// (chars, not bytes: `ç` is one).
fn check_text(field: &str, value: &str, max: Option<usize>) -> Result<(), String> {
    if value.trim().is_empty() {
        return Err(format!("{field} must not be empty"));
    }
    if let Some(max) = max {
        let chars = value.chars().count();
        if chars > max {
            return Err(format!(
                "{field} must be at most {max} characters (got {chars})"
            ));
        }
    }
    Ok(())
}

/// Refusal when no human can answer this turn: a sub-agent (depth > 0) or a
/// turn that must deliver structured output.
pub const NO_HUMAN_MESSAGE: &str = "harness::ask needs a human to answer and none is attached to \
     this turn (sub-agent or structured-output turn); report blocked with your question instead";

/// Refusal for a second `harness::ask` in the same step.
pub const ONE_PER_STEP_MESSAGE: &str =
    "only one harness::ask per step; put all your questions (up to 4) in one call";

/// Decide an in-turn `harness::ask`: the request to show, or the refusal
/// text for [`refused`]. Checked in order: someone to answer, one ask per
/// step, the argument shape, then [`validate`].
pub fn decide(
    depth: u32,
    has_output_contract: bool,
    already_asked: bool,
    args: &Value,
) -> Result<AskRequest, String> {
    if depth > 0 || has_output_contract {
        return Err(NO_HUMAN_MESSAGE.into());
    }
    if already_asked {
        return Err(ONE_PER_STEP_MESSAGE.into());
    }
    let req: AskRequest = serde_json::from_value(args.clone())
        .map_err(|e| format!("invalid harness::ask arguments: {e}"))?;
    validate(&req)?;
    Ok(req)
}

/// The result of an accepted `harness::ask`: the card data for the UI in
/// `details` and, for the model, the instruction to stop and wait.
pub fn awaiting_result(
    session_id: &str,
    turn_id: &str,
    call_id: &str,
    req: &AskRequest,
) -> ResultData {
    ResultData {
        content: vec![ContentBlock::text(AWAITING_TEXT)],
        is_error: false,
        details: serde_json::json!(AskResponse {
            status: AskStatus::AwaitingAnswer,
            question_id: call_id.to_owned(),
            session_id: session_id.to_owned(),
            turn_id: turn_id.to_owned(),
            questions: req.questions.clone(),
        }),
    }
}

/// What the model reads after a successful ask.
const AWAITING_TEXT: &str = "The questions are now shown to the user as a card of options. \
     Their answer arrives as their next message. \
     Do not repeat the questions in text. End your turn now.";

/// The `is_error` result for an `ask` that cannot be shown (invalid
/// arguments, no human to answer, a second ask in the step).
pub fn refused(reason: &str) -> ResultData {
    ResultData {
        content: vec![ContentBlock::text(reason)],
        is_error: true,
        details: Value::Null,
    }
}

/// Why a direct `harness::ask` call fails: only the turn loop can show a card
/// and end the turn on it.
pub const DIRECT_CALL_MESSAGE: &str =
    "harness::ask only works inside an agent turn: call it from a model turn, not directly";

/// The registered handler, reached only by a direct call (outside a turn,
/// or before the turn loop intercepts it).
pub async fn direct_handle(_req: AskRequest) -> Result<AskResponse, HarnessError> {
    Err(HarnessError::InvalidRequest(DIRECT_CALL_MESSAGE.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn option(label: &str) -> AskOption {
        AskOption {
            label: label.into(),
            description: None,
        }
    }

    fn question(header: &str, labels: &[&str]) -> AskQuestion {
        AskQuestion {
            header: header.into(),
            question: "Which one?".into(),
            multi_select: false,
            options: labels.iter().map(|label| option(label)).collect(),
        }
    }

    fn valid_question() -> AskQuestion {
        question("Approach", &["Pause", "End"])
    }

    fn request(questions: Vec<AskQuestion>) -> AskRequest {
        AskRequest { questions }
    }

    fn contract_example() -> serde_json::Value {
        json!({ "questions": [ {
            "header": "Abordagem",
            "question": "Quando o agente pergunta, o turno pausa ou termina?",
            "multi_select": false,
            "options": [
                { "label": "Pausa o turno", "description": "Como AskUserQuestion" },
                { "label": "Encerra o turno", "description": "Resposta vira a próxima mensagem" }
            ] } ] })
    }

    #[test]
    fn accepts_the_contract_example() {
        let req: AskRequest = serde_json::from_value(contract_example()).unwrap();
        assert_eq!(req.questions.len(), 1);
        assert_eq!(req.questions[0].header, "Abordagem");
        assert_eq!(
            req.questions[0].options[1].description.as_deref(),
            Some("Resposta vira a próxima mensagem")
        );
        assert_eq!(validate(&req), Ok(()));
    }

    #[test]
    fn multi_select_defaults_false_and_description_is_optional() {
        let req: AskRequest = serde_json::from_value(json!({ "questions": [ {
            "header": "Canais",
            "question": "Onde avisar?",
            "options": [ { "label": "Console" }, { "label": "Slack" } ]
        } ] }))
        .unwrap();
        assert!(!req.questions[0].multi_select);
        assert_eq!(req.questions[0].options[0].description, None);
        let wire = serde_json::to_value(&req).unwrap();
        assert!(!wire["questions"][0]["options"][0]
            .as_object()
            .unwrap()
            .contains_key("description"));
        assert_eq!(validate(&req), Ok(()));
    }

    #[test]
    fn accepts_the_boundaries() {
        // 4 questions, 4 options, a 16-char header and an 80-char label, all
        // multi-byte: limits count characters, not bytes.
        let mut q = question(&"ç".repeat(16), &["a", "b", "c"]);
        q.options.push(option(&"é".repeat(80)));
        let req = request(vec![q.clone(), q.clone(), q.clone(), q]);
        assert_eq!(validate(&req), Ok(()));
    }

    #[test]
    fn rejects_zero_questions() {
        let err = validate(&request(vec![])).unwrap_err();
        assert!(err.contains("1-4"), "{err}");
        assert!(err.contains("got 0"), "{err}");
    }

    #[test]
    fn rejects_five_questions() {
        let err = validate(&request(vec![valid_question(); 5])).unwrap_err();
        assert!(err.contains("1-4"), "{err}");
        assert!(err.contains("got 5"), "{err}");
    }

    #[test]
    fn rejects_one_option() {
        let req = request(vec![valid_question(), question("Solo", &["only"])]);
        let err = validate(&req).unwrap_err();
        assert!(err.contains("questions[1].options"), "{err}");
        assert!(err.contains("2-4"), "{err}");
        assert!(err.contains("got 1"), "{err}");
    }

    #[test]
    fn rejects_five_options() {
        let req = request(vec![question("Many", &["a", "b", "c", "d", "e"])]);
        let err = validate(&req).unwrap_err();
        assert!(err.contains("questions[0].options"), "{err}");
        assert!(err.contains("2-4"), "{err}");
        assert!(err.contains("got 5"), "{err}");
    }

    #[test]
    fn rejects_a_17_char_header() {
        let req = request(vec![
            valid_question(),
            question(&"ç".repeat(17), &["a", "b"]),
        ]);
        let err = validate(&req).unwrap_err();
        assert!(err.contains("questions[1].header"), "{err}");
        assert!(err.contains("16"), "{err}");
    }

    #[test]
    fn rejects_an_empty_header() {
        let err = validate(&request(vec![question("  ", &["a", "b"])])).unwrap_err();
        assert!(err.contains("questions[0].header"), "{err}");
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn rejects_empty_question_text() {
        let mut q = valid_question();
        q.question = String::new();
        let err = validate(&request(vec![valid_question(), q])).unwrap_err();
        assert!(err.contains("questions[1].question"), "{err}");
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn rejects_an_empty_label() {
        let err = validate(&request(vec![question("Pick", &["a", ""])])).unwrap_err();
        assert!(err.contains("questions[0].options[1].label"), "{err}");
        assert!(err.contains("empty"), "{err}");
    }

    #[test]
    fn rejects_an_81_char_label() {
        let long = "é".repeat(81);
        let req = request(vec![valid_question(), question("Pick", &["a", &long])]);
        let err = validate(&req).unwrap_err();
        assert!(err.contains("questions[1].options[1].label"), "{err}");
        assert!(err.contains("80"), "{err}");
    }

    #[test]
    fn rejects_duplicate_labels_within_a_question() {
        let req = request(vec![question("Pick", &["Same", "Other", "Same"])]);
        let err = validate(&req).unwrap_err();
        assert!(err.contains("questions[0].options[2].label"), "{err}");
        assert!(err.contains("unique"), "{err}");
    }

    #[test]
    fn allows_the_same_label_in_different_questions() {
        let req = request(vec![
            question("First", &["Yes", "No"]),
            question("Second", &["Yes", "No"]),
        ]);
        assert_eq!(validate(&req), Ok(()));
    }

    fn only_text(result: &ResultData) -> &str {
        assert_eq!(result.content.len(), 1, "{:?}", result.content);
        match &result.content[0] {
            ContentBlock::Text { text } => text,
            other => panic!("expected one text block, got {other:?}"),
        }
    }

    #[test]
    fn awaiting_result_carries_the_card_in_details() {
        // The second question omits multi_select and descriptions: the wire
        // shape fills the default and leaves descriptions out.
        let req: AskRequest = serde_json::from_value(json!({ "questions": [
            {
                "header": "Abordagem",
                "question": "Quando o agente pergunta, o turno pausa ou termina?",
                "multi_select": false,
                "options": [
                    { "label": "Pausa o turno", "description": "Como AskUserQuestion" },
                    { "label": "Encerra o turno", "description": "Resposta vira a próxima mensagem" }
                ]
            },
            {
                "header": "Canais",
                "question": "Onde avisar?",
                "options": [ { "label": "Console" }, { "label": "Slack" } ]
            }
        ] }))
        .unwrap();

        let result = awaiting_result("sess_1", "turn_3", "call_7", &req);

        assert!(!result.is_error);
        assert_eq!(
            result.details,
            json!({
                "status": "awaiting_answer",
                "question_id": "call_7",
                "session_id": "sess_1",
                "turn_id": "turn_3",
                "questions": [
                    {
                        "header": "Abordagem",
                        "question": "Quando o agente pergunta, o turno pausa ou termina?",
                        "multi_select": false,
                        "options": [
                            { "label": "Pausa o turno", "description": "Como AskUserQuestion" },
                            { "label": "Encerra o turno", "description": "Resposta vira a próxima mensagem" }
                        ]
                    },
                    {
                        "header": "Canais",
                        "question": "Onde avisar?",
                        "multi_select": false,
                        "options": [ { "label": "Console" }, { "label": "Slack" } ]
                    }
                ]
            })
        );
    }

    #[test]
    fn awaiting_result_tells_the_model_to_wait_for_the_answer() {
        let req = request(vec![valid_question()]);
        let result = awaiting_result("sess_1", "turn_3", "call_7", &req);
        let text = only_text(&result).to_lowercase();
        for phrase in [
            "shown to the user",
            "card of options",
            "next message",
            "do not repeat the questions",
            "end your turn now",
        ] {
            assert!(text.contains(phrase), "missing {phrase:?} in {text:?}");
        }
    }

    #[test]
    fn refused_is_an_error_carrying_the_reason() {
        let reason = "no human is available to answer; report blocked";
        let result = refused(reason);
        assert!(result.is_error);
        assert_eq!(only_text(&result), reason);
        assert_eq!(result.details, json!(null));
    }

    #[tokio::test]
    async fn a_direct_call_is_refused_outside_an_agent_turn() {
        let err = direct_handle(request(vec![valid_question()]))
            .await
            .unwrap_err();
        assert_eq!(
            err,
            HarnessError::InvalidRequest(
                "harness::ask only works inside an agent turn: call it from a model turn, not directly"
                    .into()
            )
        );
        assert!(
            err.to_string().starts_with("harness/invalid_request: "),
            "{err}"
        );
        assert!(
            err.to_string().contains("only works inside an agent turn"),
            "{err}"
        );
    }

    const NO_HUMAN: &str = "harness::ask needs a human to answer and none is attached to this \
         turn (sub-agent or structured-output turn); report blocked with your question instead";

    #[test]
    fn decide_refuses_a_sub_agent_before_anything_else() {
        // Even an already-asked step with broken arguments gets the depth
        // refusal: it is checked first.
        assert_eq!(
            decide(1, false, true, &json!({ "questions": "nope" })),
            Err(NO_HUMAN.to_string())
        );
        assert_eq!(
            decide(1, false, false, &contract_example()),
            Err(NO_HUMAN.to_string())
        );
    }

    #[test]
    fn decide_refuses_a_structured_output_turn() {
        assert_eq!(
            decide(0, true, false, &contract_example()),
            Err(NO_HUMAN.to_string())
        );
    }

    #[test]
    fn decide_refuses_a_second_ask_in_the_step() {
        assert_eq!(
            decide(0, false, true, &contract_example()),
            Err(
                "only one harness::ask per step; put all your questions (up to 4) in one call"
                    .to_string()
            )
        );
    }

    #[test]
    fn decide_names_the_serde_error_for_malformed_arguments() {
        let err = decide(0, false, false, &json!({ "questions": "not a list" })).unwrap_err();
        assert!(err.starts_with("invalid harness::ask arguments: "), "{err}");
        assert!(err.contains("invalid type"), "{err}");
        let err = decide(0, false, false, &json!({})).unwrap_err();
        assert!(err.contains("missing field `questions`"), "{err}");
    }

    #[test]
    fn decide_returns_the_validation_message_for_broken_limits() {
        assert_eq!(
            decide(0, false, false, &json!({ "questions": [] })),
            Err("questions must have 1-4 entries (got 0)".to_string())
        );
    }

    #[test]
    fn decide_accepts_a_valid_request_at_the_top_level() {
        let req = decide(0, false, false, &contract_example()).unwrap();
        assert_eq!(
            req,
            serde_json::from_value::<AskRequest>(contract_example()).unwrap()
        );
    }
}

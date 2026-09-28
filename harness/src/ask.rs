//! `harness::ask` — structured questions the agent puts to the user (KAN-7).
//!
//! The model calls `harness::ask` instead of writing a question in markdown;
//! the console renders the questions as a card of clickable options and the
//! user's pick arrives as their next message. This module owns the request
//! shape and its validation. Pure functions only: no engine, no I/O.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

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
}

//! The Decisions API wire format as pure translation: judge-contract
//! evaluations out (`POST /v1/decisions`), validated judge-contract answers
//! back, and the supported part of `GET /v1/models` as model cards.
use judge_contract::{
    validate_answer, Answer, Content, ErrorCode, Evaluation, ModelCard, ProviderError, Question,
    ScoreLevel, Usage,
};
use judge_provider::Failure;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Refused question ids listed in a refusal diagnostic, which stays bounded.
const MAX_REFUSED_IDS: usize = 32;
/// gpt-6-luna's maximum input tokens: one evaluation is all input, no output.
const CONTEXT_WINDOW: u32 = 922_000;

#[derive(Serialize)]
struct DecisionRequest<'a> {
    model: &'a str,
    input: String,
    questions: Vec<DecisionQuestion<'a>>,
}
#[derive(Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DecisionQuestion<'a> {
    Predicate {
        name: &'a str,
        instructions: String,
    },
    Choice {
        name: &'a str,
        instructions: String,
        choices: Vec<DecisionChoice<'a>>,
    },
    Score {
        name: &'a str,
        instructions: String,
        levels: Vec<DecisionLevel>,
    },
}
#[derive(Serialize)]
struct DecisionChoice<'a> {
    value: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
}
#[derive(Serialize)]
struct DecisionLevel {
    label: String,
}

// Replies are beta-API DTOs: unknown fields are ignored, unknown answer types
// and negative or fractional counters fail parsing (`invalid_response`).
#[derive(Deserialize)]
struct Decision {
    model: String,
    answers: Vec<DecisionAnswer>,
    usage: Option<DecisionUsage>,
}
#[derive(Deserialize)]
struct DecisionUsage {
    input_tokens: Option<u64>,
    output_tokens: Option<u64>,
}
#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DecisionAnswer {
    Predicate {
        name: Option<String>,
        probability: f64,
    },
    Choice {
        name: Option<String>,
        choice: Value,
        probabilities: Vec<Probability<Value>>,
        confidence: f64,
    },
    Score {
        name: Option<String>,
        score: f64,
        probabilities: Vec<Probability<i64>>,
        confidence: f64,
    },
    Refusal {
        name: Option<String>,
    },
}
#[derive(Deserialize)]
struct Probability<T> {
    value: T,
    probability: f64,
}
impl DecisionAnswer {
    fn name(&self) -> Option<&String> {
        match self {
            Self::Predicate { name, .. }
            | Self::Choice { name, .. }
            | Self::Score { name, .. }
            | Self::Refusal { name } => name.as_ref(),
        }
    }
}
#[derive(Deserialize)]
struct Models {
    data: Vec<Model>,
}
#[derive(Deserialize)]
struct Model {
    id: String,
    created: Option<i64>,
}

/// One validated reply: the echoed model, every answer of the evaluation
/// (its local ones included) and the reported usage.
pub(crate) struct Decoded {
    pub model: String,
    pub answers: BTreeMap<String, Answer>,
    pub usage: Usage,
}

/// A one-option Choice is answered here: the API takes 2 to 255 choices.
fn local_answer(question: &Question) -> Option<Answer> {
    match question {
        Question::Choice { criteria, .. } if criteria.len() == 1 => {
            let key = criteria.keys().next()?;
            Some(Answer::Choice {
                choice: key.clone(),
                probabilities: BTreeMap::from([(key.clone(), 1.0)]),
                confidence: 1.0,
            })
        }
        _ => None,
    }
}

/// Whether `evaluation` has a question that needs a Decisions call.
pub(crate) fn remote(evaluation: &Evaluation) -> bool {
    evaluation
        .questions
        .values()
        .any(|question| local_answer(question).is_none())
}

pub(crate) fn local_answers(evaluation: &Evaluation) -> BTreeMap<String, Answer> {
    evaluation
        .questions
        .iter()
        .filter_map(|(id, question)| Some((id.clone(), local_answer(question)?)))
        .collect()
}

/// Trimmed text, or compact JSON for structured content; `None` when blank.
fn text(content: &Content) -> Option<String> {
    match content {
        Content::Text(text) => nonblank(text),
        Content::Object(object) => Some(compact(object)),
        Content::Array(values) => Some(compact(values)),
        Content::Null => None,
    }
}
fn level_text(level: &ScoreLevel) -> Option<String> {
    match level {
        ScoreLevel::Text(text) => nonblank(text),
        ScoreLevel::Object(object) => Some(compact(object)),
        ScoreLevel::Array(values) => Some(compact(values)),
    }
}
fn compact(value: &impl Serialize) -> String {
    serde_json::to_string(value).expect("JSON values serialize")
}
fn nonblank(text: &str) -> Option<String> {
    Some(text.trim())
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn question<'a>(name: &'a str, question: &'a Question) -> DecisionQuestion<'a> {
    match question {
        Question::Noul {
            instructions,
            criteria,
        } => {
            let mut instructions =
                text(instructions).unwrap_or_else(|| "Is this true of the input?".into());
            let meaning = |key: &str| text(criteria.as_ref()?.get(key)?);
            if let Some(yes) = meaning("true") {
                instructions.push_str(&format!("\n\nTrue means: {yes}"));
            }
            if let Some(no) = meaning("false") {
                instructions.push_str(&format!("\nFalse means: {no}"));
            }
            DecisionQuestion::Predicate { name, instructions }
        }
        Question::Choice {
            instructions,
            criteria,
        } => DecisionQuestion::Choice {
            name,
            instructions: text(instructions)
                .unwrap_or_else(|| "Which choice best fits the input?".into()),
            choices: criteria
                .iter()
                .map(|(value, description)| DecisionChoice {
                    value,
                    description: text(description),
                })
                .collect(),
        },
        Question::Score {
            instructions,
            criteria,
        } => DecisionQuestion::Score {
            name,
            instructions: text(instructions)
                .unwrap_or_else(|| "Which level best describes the input?".into()),
            levels: criteria
                .iter()
                .enumerate()
                .map(|(index, level)| DecisionLevel {
                    label: level_text(level).unwrap_or_else(|| index.to_string()),
                })
                .collect(),
        },
    }
}

/// The body for `evaluation`'s remote questions, each named by its question
/// id. String state is the input text; object or array state is compact JSON.
pub(crate) fn encode(
    model: &str,
    evaluation: &Evaluation,
    max_request_bytes: usize,
) -> Result<Vec<u8>, ErrorCode> {
    let request = DecisionRequest {
        model,
        input: match &evaluation.state {
            Value::String(text) => text.clone(),
            state => state.to_string(),
        },
        questions: evaluation
            .questions
            .iter()
            .filter(|(_, asked)| local_answer(asked).is_none())
            .map(|(name, asked)| question(name, asked))
            .collect(),
    };
    let body = serde_json::to_vec(&request).map_err(|_| ErrorCode::InvalidRequest)?;
    if body.len() > max_request_bytes {
        return Err(ErrorCode::PayloadTooLarge);
    }
    Ok(body)
}

/// Match a reply to the questions sent, by name and in any order, convert and
/// validate every answer, and add the local ones. Never fills in a missing
/// probability. Any refusal fails the whole evaluation.
pub(crate) fn decode(evaluation: &Evaluation, bytes: &[u8]) -> Result<Decoded, Failure> {
    let invalid = || Failure::from(ErrorCode::InvalidResponse);
    let decision: Decision = serde_json::from_slice(bytes).map_err(|_| invalid())?;
    if decision.model.trim().is_empty() {
        return Err(invalid());
    }
    let usage = decision.usage.map_or_else(Usage::default, |usage| Usage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
    });
    let mut sent = BTreeMap::new();
    for answer in decision.answers {
        let name = answer.name().ok_or_else(invalid)?.clone();
        let asked = evaluation.questions.get(&name).ok_or_else(invalid)?;
        if local_answer(asked).is_some() || sent.insert(name, answer).is_some() {
            return Err(invalid());
        }
    }
    let local = local_answers(evaluation);
    if sent.len() + local.len() != evaluation.questions.len() {
        return Err(invalid());
    }
    let refused: Vec<String> = sent
        .iter()
        .filter(|(_, answer)| matches!(answer, DecisionAnswer::Refusal { .. }))
        .map(|(name, _)| name.clone())
        .collect();
    if !refused.is_empty() {
        return Err(refusal(&evaluation.id, refused, &usage));
    }
    let mut answers = local;
    for (name, answer) in sent {
        let asked = &evaluation.questions[&name];
        let answer = convert(asked, answer).ok_or_else(invalid)?;
        validate_answer(asked, &answer)?;
        answers.insert(name, answer);
    }
    Ok(Decoded {
        model: decision.model,
        answers,
        usage,
    })
}

/// API values are kept as they are (no renormalizing, no argmax fix-up);
/// `validate_answer` then rejects options, levels or keys outside the
/// question, missing entries and invalid distributions.
fn convert(question: &Question, answer: DecisionAnswer) -> Option<Answer> {
    Some(match (question, answer) {
        (Question::Noul { .. }, DecisionAnswer::Predicate { probability, .. }) => {
            Answer::Noul { noul: probability }
        }
        (
            Question::Choice { .. },
            DecisionAnswer::Choice {
                choice,
                probabilities,
                confidence,
                ..
            },
        ) => Answer::Choice {
            // A boolean value is a different option from its text.
            choice: choice.as_str()?.to_owned(),
            probabilities: distribution(probabilities, |value| Some(value.as_str()?.to_owned()))?,
            confidence,
        },
        (
            Question::Score { criteria, .. },
            DecisionAnswer::Score {
                score,
                probabilities,
                confidence,
                ..
            },
        ) => Answer::Score {
            score,
            probabilities: distribution(probabilities, |value| Some(value.to_string()))?,
            confidence,
            legend: criteria
                .iter()
                .enumerate()
                .map(|(index, level)| (index.to_string(), level.clone()))
                .collect(),
        },
        _ => return None,
    })
}
fn distribution<T>(
    entries: Vec<Probability<T>>,
    key: impl Fn(T) -> Option<String>,
) -> Option<BTreeMap<String, f64>> {
    let mut probabilities = BTreeMap::new();
    for entry in entries {
        if probabilities
            .insert(key(entry.value)?, entry.probability)
            .is_some()
        {
            return None;
        }
    }
    Some(probabilities)
}

fn refusal(evaluation: &str, refused: Vec<String>, usage: &Usage) -> Failure {
    let count = refused.len();
    Failure {
        code: ErrorCode::InvalidResponse,
        http_status: None,
        provider_error: Some(ProviderError {
            detail: Some(json!({
                "refused": &refused[..count.min(MAX_REFUSED_IDS)],
                "usage": usage,
            })),
            message: Some(format!(
                "OpenAI refused {count} question(s) in evaluation {evaluation}"
            )),
            truncated: count > MAX_REFUSED_IDS,
        }),
        retry_after_ms: None,
    }
}

/// Cards for the listed models this worker supports; none when the key
/// cannot see them.
pub(crate) fn model_cards(bytes: &[u8]) -> Result<Vec<ModelCard>, ErrorCode> {
    let models: Models = serde_json::from_slice(bytes).map_err(|_| ErrorCode::InvalidResponse)?;
    Ok(models
        .data
        .into_iter()
        .filter(|model| crate::SUPPORTED_MODELS.contains(&model.id.as_str()))
        .map(|model| ModelCard {
            name: model.id,
            description: "OpenAI Decisions (beta)".into(),
            release_date: model
                .created
                .map(|seconds| civil_from_days(seconds.div_euclid(86_400)))
                .unwrap_or_default(),
            context_window: Some(CONTEXT_WINDOW),
            max_options: None,
        })
        .collect())
}

/// `YYYY-MM-DD` of a day count since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> String {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evaluation(state: Value, questions: Value) -> Evaluation {
        serde_json::from_value(json!({"id": "ticket", "state": state, "questions": questions}))
            .unwrap()
    }
    fn body(evaluation: &Evaluation) -> Value {
        serde_json::from_slice(&encode("gpt-6-luna", evaluation, usize::MAX).unwrap()).unwrap()
    }
    fn reply(answers: Value) -> Vec<u8> {
        json!({
            "model": "gpt-6-luna", "answers": answers,
            "usage": {"input_tokens": 785, "input_tokens_details": {"cached_tokens": 0},
                      "output_tokens": 0, "total_tokens": 785}
        })
        .to_string()
        .into_bytes()
    }
    fn decoded(evaluation: &Evaluation, answers: Value) -> Result<Decoded, ErrorCode> {
        decode(evaluation, &reply(answers)).map_err(|failure| failure.code)
    }
    fn mixed() -> Evaluation {
        evaluation(
            json!("Export fails in Safari but works in Chrome."),
            json!({
                "urgent": {"type": "noul", "instructions": "Is this urgent?"},
                "team": {"type": "choice", "criteria": {"web": "Browser bugs", "api": null, "infra": "Servers"}},
                "severity": {"type": "score", "criteria": ["Cosmetic", "Workaround available", "Fully blocked"]}
            }),
        )
    }
    fn mixed_answers() -> Value {
        json!([
            {"type": "predicate", "name": "urgent", "probability": 0.7},
            {"type": "choice", "name": "team", "choice": "web", "probabilities": [
                {"value": "web", "probability": 0.9}, {"value": "api", "probability": 0.08},
                {"value": "infra", "probability": 0.02}], "confidence": 0.85},
            {"type": "score", "name": "severity", "score": 1.1, "probabilities": [
                {"value": 0, "label": "Cosmetic", "probability": 0.1},
                {"value": 1, "label": "Workaround available", "probability": 0.7},
                {"value": 2, "label": "Fully blocked", "probability": 0.2}], "confidence": 0.55}
        ])
    }

    #[test]
    fn noul_choice_and_score_become_predicate_choice_and_score_and_back() {
        let evaluation = mixed();
        assert_eq!(
            body(&evaluation),
            json!({
                "model": "gpt-6-luna",
                "input": "Export fails in Safari but works in Chrome.",
                "questions": [
                    {"type": "score", "name": "severity", "instructions": "Which level best describes the input?",
                     "levels": [{"label": "Cosmetic"}, {"label": "Workaround available"}, {"label": "Fully blocked"}]},
                    {"type": "choice", "name": "team", "instructions": "Which choice best fits the input?",
                     "choices": [{"value": "api"}, {"value": "infra", "description": "Servers"},
                                 {"value": "web", "description": "Browser bugs"}]},
                    {"type": "predicate", "name": "urgent", "instructions": "Is this urgent?"}
                ]
            })
        );
        let decoded = decoded(&evaluation, mixed_answers()).unwrap();
        assert_eq!(decoded.model, "gpt-6-luna");
        assert_eq!(
            decoded.usage,
            Usage {
                input_tokens: Some(785),
                output_tokens: Some(0)
            }
        );
        assert_eq!(
            serde_json::to_value(&decoded.answers).unwrap(),
            json!({
                "urgent": {"type": "noul", "noul": 0.7},
                "team": {"type": "choice", "choice": "web",
                         "probabilities": {"web": 0.9, "api": 0.08, "infra": 0.02}, "confidence": 0.85},
                "severity": {"type": "score", "score": 1.1,
                             "probabilities": {"0": 0.1, "1": 0.7, "2": 0.2}, "confidence": 0.55,
                             "legend": {"0": "Cosmetic", "1": "Workaround available", "2": "Fully blocked"}}
            })
        );
    }

    #[test]
    fn text_object_and_array_content_reach_the_body_as_text_or_compact_json() {
        let questions = json!({
            "urgent": {"type": "noul", "instructions": {"ask": "urgent?"},
                       "criteria": {"true": ["outage", "data loss"], "false": "  routine  "}},
            "team": {"type": "choice", "instructions": ["pick", "one"],
                     "criteria": {"web": {"scope": "browser"}, "api": ["rest", "grpc"]}},
            "severity": {"type": "score", "instructions": "  Rate it.  ",
                         "criteria": [{"impact": "none"}, ["blocked"]]}
        });
        let object = body(&evaluation(
            json!({"message": "Sign-in is blocked", "tags": [1, 2]}),
            questions.clone(),
        ));
        assert_eq!(
            object["input"],
            r#"{"message":"Sign-in is blocked","tags":[1,2]}"#
        );
        assert_eq!(
            object["questions"],
            json!([
                {"type": "score", "name": "severity", "instructions": "Rate it.",
                 "levels": [{"label": r#"{"impact":"none"}"#}, {"label": r#"["blocked"]"#}]},
                {"type": "choice", "name": "team", "instructions": r#"["pick","one"]"#,
                 "choices": [{"value": "api", "description": r#"["rest","grpc"]"#},
                             {"value": "web", "description": r#"{"scope":"browser"}"#}]},
                {"type": "predicate", "name": "urgent",
                 "instructions": "{\"ask\":\"urgent?\"}\n\nTrue means: [\"outage\",\"data loss\"]\nFalse means: routine"}
            ])
        );
        // An array is one compact JSON input, never a list of input messages.
        let array = body(&evaluation(json!(["a", {"b": 1}]), questions.clone()));
        assert_eq!(array["input"], r#"["a",{"b":1}]"#);
        let text = body(&evaluation(json!("  raw text  "), questions));
        assert_eq!(text["input"], "  raw text  ");
    }

    #[test]
    fn null_or_blank_instructions_use_defaults_and_blank_levels_their_index() {
        for instructions in [Value::Null, json!(""), json!("  ")] {
            let sent = body(&evaluation(
                json!("state"),
                json!({
                    "a": {"type": "noul", "instructions": instructions, "criteria": {"true": "  ", "false": null}},
                    "b": {"type": "choice", "instructions": instructions, "criteria": {"x": "", "y": "  "}},
                    "c": {"type": "score", "instructions": instructions, "criteria": ["", "  ", "high"]}
                }),
            ));
            assert_eq!(
                sent["questions"],
                json!([
                    {"type": "predicate", "name": "a", "instructions": "Is this true of the input?"},
                    {"type": "choice", "name": "b", "instructions": "Which choice best fits the input?",
                     "choices": [{"value": "x"}, {"value": "y"}]},
                    {"type": "score", "name": "c", "instructions": "Which level best describes the input?",
                     "levels": [{"label": "0"}, {"label": "1"}, {"label": "high"}]}
                ])
            );
        }
    }

    #[test]
    fn one_option_choices_are_answered_locally_alone_or_beside_remote_questions() {
        let only = evaluation(
            json!("state"),
            json!({"pick": {"type": "choice", "criteria": {"state::get": {"id": "state::get"}}}}),
        );
        assert!(!remote(&only));
        assert_eq!(
            serde_json::to_value(local_answers(&only)).unwrap(),
            json!({"pick": {"type": "choice", "choice": "state::get",
                            "probabilities": {"state::get": 1.0}, "confidence": 1.0}})
        );
        validate_answer(&only.questions["pick"], &local_answers(&only)["pick"]).unwrap();

        let mut both = mixed();
        both.questions
            .insert("pick".into(), only.questions["pick"].clone());
        assert!(remote(&both));
        let sent = body(&both);
        assert_eq!(sent["questions"].as_array().unwrap().len(), 3);
        assert!(!sent.to_string().contains("state::get"));
        let decoded = decoded(&both, mixed_answers()).unwrap();
        // Every question of the evaluation is answered: 3 remote + 1 local.
        assert_eq!(decoded.answers.len(), 4);
        assert_eq!(
            serde_json::to_value(&decoded.answers["pick"]).unwrap(),
            serde_json::to_value(&local_answers(&only)["pick"]).unwrap()
        );
        // The local question is not an expected name in the reply.
        let mut extra = mixed_answers();
        extra.as_array_mut().unwrap().push(
            json!({"type": "choice", "name": "pick", "choice": "state::get",
            "probabilities": [{"value": "state::get", "probability": 1.0}], "confidence": 1.0}),
        );
        assert_eq!(decoded_code(&both, extra), Some(ErrorCode::InvalidResponse));
    }

    fn decoded_code(evaluation: &Evaluation, answers: Value) -> Option<ErrorCode> {
        decoded(evaluation, answers).err()
    }

    #[test]
    fn answers_match_by_name_in_any_order_and_names_must_be_exact() {
        let evaluation = mixed();
        let mut reversed = mixed_answers();
        reversed.as_array_mut().unwrap().reverse();
        assert_eq!(decoded(&evaluation, reversed).unwrap().answers.len(), 3);

        let answers = mixed_answers();
        let mut duplicate = answers.clone();
        duplicate[2] = answers[0].clone();
        let mut missing = answers.clone();
        missing.as_array_mut().unwrap().pop();
        let mut null = answers.clone();
        null[0]["name"] = Value::Null;
        let mut absent = answers.clone();
        absent[0].as_object_mut().unwrap().remove("name");
        let mut unknown = answers.clone();
        unknown[0]["name"] = json!("other");
        let mut swapped = answers.clone();
        swapped[0]["name"] = json!("team");
        swapped[1]["name"] = json!("urgent");
        let mut boolean = answers.clone();
        boolean[1]["choice"] = json!(true);
        let mut boolean_value = answers.clone();
        boolean_value[1]["probabilities"][0]["value"] = json!(true);
        let mut outside = answers.clone();
        outside[1]["choice"] = json!("mobile");
        let mut unknown_type = answers.clone();
        unknown_type[0]["type"] = json!("verdict");
        for bad in [
            duplicate,
            missing,
            null,
            absent,
            unknown,
            swapped,
            boolean,
            boolean_value,
            outside,
            unknown_type,
        ] {
            assert_eq!(
                decoded_code(&evaluation, bad.clone()),
                Some(ErrorCode::InvalidResponse),
                "{bad}"
            );
        }
    }

    #[test]
    fn score_values_and_distributions_must_cover_each_option_exactly_once() {
        let evaluation = mixed();
        let answers = mixed_answers();
        let mut out_of_range = answers.clone();
        out_of_range[2]["probabilities"][2]["value"] = json!(3);
        let mut negative = answers.clone();
        negative[2]["probabilities"][0]["value"] = json!(-1);
        let mut fractional = answers.clone();
        fractional[2]["probabilities"][0]["value"] = json!(0.5);
        let mut score = answers.clone();
        score[2]["score"] = json!(2.01);
        let mut missing_level = answers.clone();
        missing_level[2]["probabilities"]
            .as_array_mut()
            .unwrap()
            .pop();
        let mut duplicate_level = answers.clone();
        duplicate_level[2]["probabilities"][2]["value"] = json!(1);
        let mut missing_option = answers.clone();
        missing_option[1]["probabilities"]
            .as_array_mut()
            .unwrap()
            .pop();
        let mut duplicate_option = answers.clone();
        duplicate_option[1]["probabilities"][2]["value"] = json!("web");
        let mut over = answers.clone();
        over[1]["probabilities"][0]["probability"] = json!(1.5);
        let mut sum = answers.clone();
        sum[1]["probabilities"][0]["probability"] = json!(0.5);
        let mut usage = reply(answers.clone());
        usage = String::from_utf8(usage)
            .unwrap()
            .replace("\"output_tokens\":0", "\"output_tokens\":-1")
            .into_bytes();
        assert_eq!(
            decode(&evaluation, &usage).err().map(|f| f.code),
            Some(ErrorCode::InvalidResponse)
        );
        for bad in [
            out_of_range,
            negative,
            fractional,
            score,
            missing_level,
            duplicate_level,
            missing_option,
            duplicate_option,
            over,
            sum,
        ] {
            assert_eq!(
                decoded_code(&evaluation, bad.clone()),
                Some(ErrorCode::InvalidResponse),
                "{bad}"
            );
        }
    }

    #[test]
    fn a_rounded_sixteen_option_distribution_summing_to_094_is_invalid() {
        let criteria: serde_json::Map<String, Value> =
            (0..16).map(|i| (format!("f{i}"), json!(null))).collect();
        let evaluation = evaluation(
            json!("state"),
            json!({"c0": {"type": "choice", "criteria": criteria}}),
        );
        let probabilities: Vec<Value> = (0..16)
            .map(|i| json!({"value": format!("f{i}"), "probability": if i == 0 { 0.94 } else { 0.0 }}))
            .collect();
        let answer = |probabilities: Vec<Value>| {
            json!([{"type": "choice", "name": "c0", "choice": "f0",
                    "probabilities": probabilities, "confidence": 0.94}])
        };
        assert_eq!(
            decoded_code(&evaluation, answer(probabilities.clone())),
            Some(ErrorCode::InvalidResponse)
        );
        let mut rounded = probabilities;
        rounded[1]["probability"] = json!(0.05);
        assert!(decoded(&evaluation, answer(rounded)).is_ok());
    }

    #[test]
    fn the_documented_choice_and_score_examples_pass() {
        let evaluation = evaluation(
            json!("I was charged twice for my order."),
            json!({
                "department": {"type": "choice", "instructions": "Which department should handle this complaint?",
                               "criteria": {"billing": "Payments, invoices, and refunds.", "technical": "Problems using the product.",
                                            "shipping": "Delivery and tracking.", "other": "Requests outside these categories."}},
                "severity": {"type": "score", "instructions": "How severe is this issue?",
                             "criteria": ["Cosmetic", "Workaround available", "Fully blocked"]}
            }),
        );
        let decoded = decoded(
            &evaluation,
            json!([
                {"type": "choice", "name": "department", "choice": "billing", "probabilities": [
                    {"value": "billing", "probability": 0.95}, {"value": "technical", "probability": 0.02},
                    {"value": "shipping", "probability": 0.01}, {"value": "other", "probability": 0.02}],
                 "confidence": 0.93},
                {"type": "score", "name": "severity", "score": 1.1, "probabilities": [
                    {"value": 0, "label": "Cosmetic", "probability": 0.1},
                    {"value": 1, "label": "Workaround available", "probability": 0.7},
                    {"value": 2, "label": "Fully blocked", "probability": 0.2}],
                 "confidence": 0.55}
            ]),
        )
        .unwrap();
        let Answer::Choice { confidence, .. } = &decoded.answers["department"] else {
            panic!("choice");
        };
        // The API's confidence is kept; it matches the contract's formula to 2 decimals.
        assert_eq!(*confidence, 0.93);
        let parity = judge_contract::confidence::choice(&[0.02, 0.95, 0.02, 0.01]);
        assert!((parity - 0.93).abs() < 0.005, "{parity}");
        let Answer::Score {
            score, confidence, ..
        } = &decoded.answers["severity"]
        else {
            panic!("score");
        };
        assert_eq!((*score, *confidence), (1.1, 0.55));
        assert!((judge_contract::confidence::score(&[0.1, 0.7, 0.2]) - 0.55).abs() < 0.005);
    }

    #[test]
    fn a_choice_that_is_not_the_argmax_passes_unchanged() {
        let evaluation = mixed();
        let mut answers = mixed_answers();
        answers[1]["choice"] = json!("api");
        let decoded = decoded(&evaluation, answers).unwrap();
        let Answer::Choice {
            choice,
            probabilities,
            ..
        } = &decoded.answers["team"]
        else {
            panic!("choice");
        };
        assert_eq!(choice, "api");
        assert_eq!(probabilities["web"], 0.9);
    }

    #[test]
    fn a_blank_model_echo_is_invalid() {
        let evaluation = mixed();
        for model in ["", "  "] {
            let mut body: Value = serde_json::from_slice(&reply(mixed_answers())).unwrap();
            body["model"] = json!(model);
            let failure = decode(&evaluation, body.to_string().as_bytes()).err();
            assert_eq!(failure.map(|f| f.code), Some(ErrorCode::InvalidResponse));
        }
    }

    #[test]
    fn missing_usage_is_unknown_not_invalid() {
        let mut body: Value = serde_json::from_slice(&reply(mixed_answers())).unwrap();
        body.as_object_mut().unwrap().remove("usage");
        let decoded = decode(&mixed(), body.to_string().as_bytes()).unwrap();
        assert_eq!(decoded.usage, Usage::default());
    }

    #[test]
    fn any_refusal_fails_the_evaluation_with_bounded_ids_and_its_usage() {
        let evaluation = mixed();
        let mut answers = mixed_answers();
        answers[1] = json!({"type": "refusal", "name": "team"});
        let failure = decode(&evaluation, &reply(answers)).err().unwrap();
        assert_eq!(failure.code, ErrorCode::InvalidResponse);
        assert_eq!(failure.http_status, None);
        let error = failure.provider_error.unwrap();
        assert_eq!(
            error.message.as_deref(),
            Some("OpenAI refused 1 question(s) in evaluation ticket")
        );
        assert_eq!(
            error.detail,
            Some(json!({"refused": ["team"], "usage": {"input_tokens": 785, "output_tokens": 0}}))
        );
        assert!(!error.truncated);

        let questions: serde_json::Map<String, Value> = (0..40)
            .map(|i| (format!("q{i:02}"), json!({"type": "noul"})))
            .collect();
        let many = self::evaluation(json!("state"), Value::Object(questions));
        let refusals: Vec<Value> = (0..40)
            .map(|i| json!({"type": "refusal", "name": format!("q{i:02}")}))
            .collect();
        let error = decode(&many, &reply(Value::Array(refusals)))
            .err()
            .unwrap()
            .provider_error
            .unwrap();
        assert_eq!(
            error.message.as_deref(),
            Some("OpenAI refused 40 question(s) in evaluation ticket")
        );
        assert_eq!(
            error.detail.unwrap()["refused"].as_array().unwrap().len(),
            32
        );
        assert!(error.truncated);
    }

    #[test]
    fn model_cards_keep_supported_models_with_their_release_date() {
        let cards = model_cards(
            json!({"object": "list", "data": [
                {"id": "gpt-6-luna", "object": "model", "created": 1789406102, "owned_by": "system", "shutdown_date": null},
                {"id": "gpt-5.6-luna", "object": "model", "created": 1780000000, "owned_by": "system"}
            ]})
            .to_string()
            .as_bytes(),
        )
        .unwrap();
        assert_eq!(
            serde_json::to_value(cards).unwrap(),
            json!([{"name": "gpt-6-luna", "description": "OpenAI Decisions (beta)",
                    "release_date": "2026-09-14", "context_window": 922000}])
        );
        let undated = model_cards(br#"{"data":[{"id":"gpt-6-luna"}]}"#).unwrap();
        assert_eq!(undated[0].release_date, "");
        assert!(model_cards(br#"{"data":[]}"#).unwrap().is_empty());
        assert_eq!(
            model_cards(br#"{"models":[]}"#).err(),
            Some(ErrorCode::InvalidResponse)
        );
    }

    #[test]
    fn civil_from_days_matches_known_dates() {
        for (days, date) in [
            (0, "1970-01-01"),
            (-1, "1969-12-31"),
            (11_016, "2000-02-29"),
            (20_710, "2026-09-14"),
            (2_932_896, "9999-12-31"),
        ] {
            assert_eq!(civil_from_days(days), date);
        }
    }
}

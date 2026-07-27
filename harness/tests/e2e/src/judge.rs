use std::collections::{HashMap, HashSet};

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::context::E2eContext;
use crate::report::CriterionSource;
use crate::scenarios::{CriterionSpec, ScenarioObservation, ScenarioSpec};

const JUDGE_SYSTEM_PROMPT: &str = "You are an impartial software-agent quality evaluator. \
Score only the supplied answer and evidence against the supplied rubric and reference. \
Do not reward claims that are not supported by the evidence. Return exactly one JSON object, \
without Markdown or explanatory text.";
pub const JUDGE_PROTOCOL: &str = "plain-json";
const MAX_JUDGE_ATTEMPTS: u8 = 3;

#[derive(Debug, Clone)]
pub struct JudgeConfig {
    pub model: String,
    pub provider: String,
}

#[derive(Debug)]
pub struct JudgeAward {
    pub id: String,
    pub awarded: u8,
    pub reason: String,
}

pub struct JudgeOutcome {
    pub awards: Vec<JudgeAward>,
    pub usage: Option<Value>,
    pub attempts: u8,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JudgeResponse {
    criteria: Vec<JudgeCriterion>,
    summary: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct JudgeCriterion {
    id: String,
    awarded: u8,
    reason: String,
}

pub async fn evaluate(
    context: &E2eContext,
    config: &JudgeConfig,
    spec: &ScenarioSpec,
    observation: &ScenarioObservation,
    objective_evidence: &Value,
) -> Result<JudgeOutcome> {
    let criteria: Vec<_> = spec
        .criteria
        .iter()
        .filter(|criterion| criterion.source == CriterionSource::Judge)
        .collect();
    if criteria.is_empty() {
        bail!("scenario {} has no judge criteria", spec.id);
    }
    let reference = spec
        .judge_reference
        .as_ref()
        .ok_or_else(|| anyhow!("scenario {} has no judge reference", spec.id))?;
    let rubric: Vec<_> = criteria
        .iter()
        .map(|criterion| {
            json!({
                "id": criterion.id,
                "possible": criterion.weight,
                "description": criterion.description,
            })
        })
        .collect();
    let input = json!({
        "task_prompt": spec.prompt,
        "assistant_answer": crate::scenarios::common::final_response(&observation.output),
        "transcript": observation.transcript,
        "objective_evidence": objective_evidence,
        "reference": reference,
        "rubric": rubric,
    });
    let response_template = json!({
        "criteria": criteria.iter().map(|criterion| {
            json!({
                "id": criterion.id,
                "awarded": 0,
                "reason": "brief evidence-based justification",
            })
        }).collect::<Vec<_>>(),
        "summary": "brief overall assessment",
    });
    let prompt = format!(
        "Evaluate this case:\n{}\n\n\
Your response must satisfy this JSON Schema:\n{}\n\n\
Include every rubric id exactly once. For each criterion, `awarded` must be an \
integer from zero through that criterion's `possible` value, not a percentage. \
Use this exact object shape and replace only the scores and explanatory text:\n{}",
        serde_json::to_string(&input).context("serialize judge input")?,
        serde_json::to_string(&response_schema()).context("serialize judge response schema")?,
        serde_json::to_string(&response_template).context("serialize judge response template")?,
    );
    let mut attempt_prompt = prompt.clone();
    let mut usages = Vec::with_capacity(MAX_JUDGE_ATTEMPTS as usize);

    for attempt in 1..=MAX_JUDGE_ATTEMPTS {
        let response = invoke(context, config, &attempt_prompt)
            .await
            .with_context(|| format!("invoke E2E judge attempt {attempt}"))?;
        usages.push(response.get("usage").cloned());
        let response_text = assistant_text(&response);

        match parse_response(&response_text).and_then(|parsed| validate_response(&criteria, parsed))
        {
            Ok(awards) => {
                return Ok(JudgeOutcome {
                    awards,
                    usage: combined_usage(&usages),
                    attempts: attempt,
                });
            }
            Err(error) if attempt < MAX_JUDGE_ATTEMPTS => {
                attempt_prompt = repair_prompt(&prompt, &error, &response_text, attempt);
            }
            Err(error) => {
                bail!(
                    "judge returned an invalid rubric result after {attempt} attempts: \
{error:#}; response: {}",
                    response_excerpt(&response_text)
                );
            }
        }
    }

    unreachable!("judge attempt loop always returns")
}

async fn invoke(context: &E2eContext, config: &JudgeConfig, prompt: &str) -> Result<Value> {
    context
        .trigger_value("router::complete", judge_request(config, prompt))
        .await
}

fn judge_request(config: &JudgeConfig, prompt: &str) -> Value {
    json!({
        "model": config.model,
        "provider": config.provider,
        "system_prompt": JUDGE_SYSTEM_PROMPT,
        "messages": [{
            "role": "user",
            "content": [{
                "type": "text",
                "text": prompt,
            }],
            "timestamp": now_ms() as i64,
        }],
        "max_output_tokens": 2_048,
    })
}

fn repair_prompt(base_prompt: &str, error: &anyhow::Error, response: &str, attempt: u8) -> String {
    format!(
        "{base_prompt}\n\nYour response from attempt {attempt} was invalid.\n\
Validation error: {error:#}\n\
Previous response:\n{response}\n\nReturn a corrected JSON object only."
    )
}

fn combined_usage(attempts: &[Option<Value>]) -> Option<Value> {
    if attempts.iter().all(Option::is_none) {
        return None;
    }
    if attempts.len() == 1 {
        return attempts[0].clone();
    }
    Some(json!({ "attempts": attempts }))
}

fn parse_response(text: &str) -> Result<JudgeResponse> {
    let start = text
        .find('{')
        .ok_or_else(|| anyhow!("judge response contains no JSON object"))?;
    let end = text
        .rfind('}')
        .filter(|end| *end >= start)
        .ok_or_else(|| anyhow!("judge response contains no complete JSON object"))?;
    serde_json::from_str(&text[start..=end]).context("judge returned invalid JSON")
}

fn validate_response(
    criteria: &[&CriterionSpec],
    response: JudgeResponse,
) -> Result<Vec<JudgeAward>> {
    if response.summary.trim().is_empty() {
        bail!("judge summary cannot be empty");
    }
    let expected: HashMap<_, _> = criteria
        .iter()
        .map(|criterion| (criterion.id, criterion.weight))
        .collect();
    let mut seen = HashSet::new();
    let mut awards = Vec::with_capacity(response.criteria.len());
    for result in response.criteria {
        if !seen.insert(result.id.clone()) {
            bail!("judge repeated criterion {}", result.id);
        }
        let possible = expected
            .get(result.id.as_str())
            .ok_or_else(|| anyhow!("judge returned unknown criterion {}", result.id))?;
        if result.awarded > *possible {
            bail!(
                "judge awarded {} of {} points for {}",
                result.awarded,
                possible,
                result.id
            );
        }
        if result.reason.trim().is_empty() {
            bail!("judge returned no reason for {}", result.id);
        }
        awards.push(JudgeAward {
            id: result.id,
            awarded: result.awarded,
            reason: result.reason,
        });
    }
    for id in expected.keys() {
        if !seen.contains(*id) {
            bail!("judge omitted criterion {id}");
        }
    }
    Ok(awards)
}

fn assistant_text(response: &Value) -> String {
    response
        .pointer("/message/content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

fn response_excerpt(response: &str) -> String {
    const MAX_CHARS: usize = 2_000;
    let mut excerpt: String = response.chars().take(MAX_CHARS).collect();
    if response.chars().count() > MAX_CHARS {
        excerpt.push('…');
    }
    excerpt
}

fn response_schema() -> Value {
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["criteria", "summary"],
        "properties": {
            "criteria": {
                "type": "array",
                "items": {
                    "type": "object",
                    "additionalProperties": false,
                    "required": ["id", "awarded", "reason"],
                    "properties": {
                        "id": { "type": "string" },
                        "awarded": { "type": "integer", "minimum": 0, "maximum": 100 },
                        "reason": { "type": "string" }
                    }
                }
            },
            "summary": { "type": "string" }
        }
    })
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn criteria() -> Vec<CriterionSpec> {
        vec![
            CriterionSpec {
                id: "correctness",
                source: CriterionSource::Judge,
                weight: 70,
                description: "correct",
            },
            CriterionSpec {
                id: "clarity",
                source: CriterionSource::Judge,
                weight: 30,
                description: "clear",
            },
        ]
    }

    #[test]
    fn accepts_exact_bounded_criterion_set() {
        let specs = criteria();
        let refs: Vec<_> = specs.iter().collect();
        let awards = validate_response(
            &refs,
            JudgeResponse {
                criteria: vec![
                    JudgeCriterion {
                        id: "correctness".into(),
                        awarded: 60,
                        reason: "mostly correct".into(),
                    },
                    JudgeCriterion {
                        id: "clarity".into(),
                        awarded: 30,
                        reason: "clear".into(),
                    },
                ],
                summary: "good".into(),
            },
        )
        .unwrap();
        assert_eq!(awards.iter().map(|award| award.awarded).sum::<u8>(), 90);
    }

    #[test]
    fn parses_a_json_object_even_when_the_provider_ignores_response_format() {
        let response =
            parse_response("```json\n{\"criteria\":[],\"summary\":\"provider fallback\"}\n```")
                .unwrap();
        assert_eq!(response.summary, "provider fallback");
    }

    #[test]
    fn portable_request_does_not_require_native_structured_output() {
        let request = judge_request(
            &JudgeConfig {
                model: "judge".into(),
                provider: "provider".into(),
            },
            "evaluate",
        );
        assert!(request.get("response_format").is_none());
        assert_eq!(request["max_output_tokens"], 2_048);
    }

    #[test]
    fn rejects_missing_unknown_duplicate_and_excessive_scores() {
        let specs = criteria();
        let refs: Vec<_> = specs.iter().collect();
        for criteria in [
            vec![JudgeCriterion {
                id: "correctness".into(),
                awarded: 60,
                reason: "ok".into(),
            }],
            vec![
                JudgeCriterion {
                    id: "correctness".into(),
                    awarded: 60,
                    reason: "ok".into(),
                },
                JudgeCriterion {
                    id: "unknown".into(),
                    awarded: 10,
                    reason: "no".into(),
                },
            ],
            vec![
                JudgeCriterion {
                    id: "correctness".into(),
                    awarded: 60,
                    reason: "ok".into(),
                },
                JudgeCriterion {
                    id: "correctness".into(),
                    awarded: 10,
                    reason: "again".into(),
                },
            ],
            vec![
                JudgeCriterion {
                    id: "correctness".into(),
                    awarded: 71,
                    reason: "too high".into(),
                },
                JudgeCriterion {
                    id: "clarity".into(),
                    awarded: 20,
                    reason: "ok".into(),
                },
            ],
        ] {
            assert!(validate_response(
                &refs,
                JudgeResponse {
                    criteria,
                    summary: "summary".into(),
                },
            )
            .is_err());
        }
    }
}

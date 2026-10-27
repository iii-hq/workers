//! Level 1 of the proof: the base's failed calls replayed. The step that
//! made one, and the step after it that saw its error, are rebuilt from the
//! transcript by the base and by the candidate build (`harness-replay`,
//! `harness::turn_loop::replay`) and sampled on both: a change to what the
//! model is told reaches the first, a change to how a refusal reads reaches
//! the second. A sample is a mistake when one of its calls would be refused before
//! it runs: arguments that are not valid JSON, or that do not match the
//! target's request schema after the Harness's own deterministic repair.
//! Each step gets as many samples on each side, so pooling the counts into
//! one Fisher exact test is sound.

use std::collections::{BTreeMap, BTreeSet};

use harness::policy::{plan_calls, CallKind};
use harness::types::message::AgentMessage;
use harness::types::turn::ExposeMode;
use serde_json::{json, Value};

use crate::contract::{ErrorSiteV1, ValidationOutcomeV1};
use crate::evidence;

/// Answers sampled on each side, spread over the steps.
const SAMPLES: u32 = 60;
const MIN_PER_STEP: u32 = 4;
const MAX_PER_STEP: u32 = 16;
/// Changed steps sampled at most.
pub const MAX_POINTS: usize = 15;
/// What one sampled answer is reserved at: a DeepSeek Flash step of the
/// suite costs about $0.007.
pub const ESTIMATED_USD_PER_SAMPLE: f64 = 0.01;
/// Fewer base mistakes than this tell nothing either way.
const MIN_MISTAKES: u32 = 5;
const ALPHA: f64 = 0.05;
/// The least relative drop that counts as fewer: half, like the counted
/// criterion's minimum effect.
const EFFECT: f64 = 0.5;

/// One step to replay.
#[derive(Debug, Clone)]
pub struct Point {
    pub site_id: String,
    pub run_id: String,
    pub turn_id: String,
    pub step: u64,
    /// The step after the failed call, which saw its error.
    pub after: bool,
    /// The session path before the step's assistant entry.
    pub entries: Vec<Value>,
    /// The fingerprint the recorded step carries (`origin.req`).
    pub recorded: Value,
    /// The answer that made the failed call.
    pub cause: Value,
}

/// The steps around `sites`' failed calls, each once, in site order: the
/// one that made the call, then the next one, when the run went on.
pub fn points(bundle: &Value, sites: &[ErrorSiteV1]) -> Vec<Point> {
    let runs = evidence::runs(bundle);
    let mut seen = BTreeSet::new();
    let mut found = Vec::new();
    for site in sites {
        let Some(entries) = runs
            .iter()
            .find(|(scenario, run)| {
                scenario["scenario_id"] == site.scenario_id.as_str()
                    && run["run_id"] == site.run_id.as_str()
            })
            .and_then(|(_, run)| run["transcript"]["messages"].as_array())
        else {
            continue;
        };
        let Some(failed) = entries
            .iter()
            .position(|entry| entry["entry_id"] == site.entry_id.as_str())
        else {
            continue;
        };
        let call_id = &entries[failed]["message"]["function_call_id"];
        let Some(made) = entries[..failed].iter().rposition(|entry| {
            entry["message"]["content"]
                .as_array()
                .is_some_and(|blocks| blocks.iter().any(|block| &block["id"] == call_id))
        }) else {
            continue;
        };
        let next = entries[failed..]
            .iter()
            .position(|entry| step_of(entry["entry_id"].as_str().unwrap_or_default()).is_some())
            .map(|offset| failed + offset);
        for (at, after) in [(Some(made), false), (next, true)] {
            let Some(at) = at else { continue };
            let entry_id = entries[at]["entry_id"].as_str().unwrap_or_default();
            let Some((turn_id, step)) = step_of(entry_id) else {
                continue;
            };
            if seen.insert((site.run_id.clone(), entry_id.to_string())) {
                found.push(Point {
                    site_id: site.site_id.clone(),
                    run_id: site.run_id.clone(),
                    turn_id,
                    step,
                    after,
                    entries: entries[..at].to_vec(),
                    recorded: entries[at]["origin"]["req"].clone(),
                    cause: entries[made]["message"].clone(),
                });
            }
        }
    }
    found
}

/// `e_<turn>_<step>_assistant`.
fn step_of(entry_id: &str) -> Option<(String, u64)> {
    let (turn, step) = entry_id
        .strip_prefix("e_")?
        .strip_suffix("_assistant")?
        .rsplit_once('_')?;
    Some((turn.to_string(), step.parse().ok()?))
}

/// The `harness-replay` input for `point`.
pub fn input(point: &Point, model: &str, provider: &str) -> Value {
    json!({"entries": point.entries, "turn_id": point.turn_id, "step": point.step,
        "model": model, "provider": provider})
}

/// The parts of a rebuilt request's fingerprint equal to the recorded one.
pub fn exact(rebuilt: &Value, recorded: &Value) -> Vec<String> {
    [
        ("system", "system_sha"),
        ("tools", "tools_sha"),
        ("messages", "messages_sha"),
    ]
    .into_iter()
    .filter(|(_, key)| recorded.get(key).is_some() && rebuilt.get(key) == recorded.get(key))
    .map(|(part, _)| part.to_string())
    .collect()
}

fn calls(answer: &Value) -> Vec<(String, Value)> {
    match serde_json::from_value(answer.clone()) {
        Ok(AgentMessage::Assistant(assistant)) => plan_calls(&assistant, ExposeMode::AgentTrigger)
            .into_iter()
            .filter(|call| call.kind == CallKind::Trigger)
            .map(|call| (call.function_id, call.arguments))
            .collect(),
        _ => Vec::new(),
    }
}

/// The functions `answer` calls.
pub fn targets(answer: &Value) -> Vec<String> {
    calls(answer)
        .into_iter()
        .map(|(target, _)| target)
        .collect()
}

/// Why one of `answer`'s calls would be refused before it runs, if one
/// would. A call to a function `schemas` lacks is judged on its JSON alone.
pub fn mistake(answer: &Value, schemas: &BTreeMap<String, Value>) -> Option<String> {
    for (target, arguments) in calls(answer) {
        if harness::trigger::arguments_degraded(&arguments) {
            return Some(format!("{target}: arguments are not valid JSON"));
        }
        let Some(schema) = schemas.get(&target) else {
            continue;
        };
        let Ok(compiled) = jsonschema::JSONSchema::compile(schema) else {
            continue;
        };
        let arguments = harness::reconcile::parse_untyped(schema, &compiled, &arguments)
            .map_or(arguments, |repaired| repaired.arguments);
        let arguments = harness::reconcile::coerce(schema, &arguments)
            .map_or(arguments, |repaired| repaired.arguments);
        if !compiled.is_valid(&arguments) {
            return Some(format!(
                "{target}: arguments do not match its request schema"
            ));
        }
    }
    None
}

/// Answers per step on each side.
pub fn per_step(points: usize) -> u32 {
    SAMPLES
        .div_ceil(points.max(1) as u32)
        .clamp(MIN_PER_STEP, MAX_PER_STEP)
}

/// Fisher's exact test on `(mistakes, samples)` of each side, one-sided
/// both ways: (the candidate makes fewer, the candidate makes more).
pub fn fisher(base: (u32, u32), candidate: (u32, u32)) -> (f64, f64) {
    let ((x, n1), (y, n2)) = (candidate, base);
    let (k, n) = (x + y, n1 + n2);
    let ln_fact: Vec<f64> = (0..=n)
        .scan(0.0, |sum, i| {
            *sum += f64::from(i.max(1)).ln();
            Some(*sum)
        })
        .collect();
    let ln_choose =
        |n: u32, k: u32| ln_fact[n as usize] - ln_fact[k as usize] - ln_fact[(n - k) as usize];
    let p = |i: u32| (ln_choose(n1, i) + ln_choose(n2, k - i) - ln_choose(n, k)).exp();
    let (low, high) = (k.saturating_sub(n2), k.min(n1));
    let fewer: f64 = (low..=x).map(p).sum();
    let more: f64 = (x..=high).map(p).sum();
    (fewer.min(1.0), more.min(1.0))
}

/// What the pooled counts show.
pub fn verdict(base: (u32, u32), candidate: (u32, u32)) -> (ValidationOutcomeV1, String) {
    use ValidationOutcomeV1::*;
    let (fewer, more) = fisher(base, candidate);
    let counts = format!(
        "replayed where calls failed, the base made a refused call in {}/{} answers and the \
         candidate in {}/{}",
        base.0, base.1, candidate.0, candidate.1
    );
    let rate = |(mistakes, samples): (u32, u32)| f64::from(mistakes) / f64::from(samples.max(1));
    if more <= ALPHA {
        (Regression, format!("{counts}: more (Fisher, p={more:.4})"))
    } else if base.0 < MIN_MISTAKES {
        (
            Inconclusive,
            format!("{counts}: the base rarely repeats the mistake, so this tells nothing"),
        )
    } else if fewer <= ALPHA && rate(candidate) <= rate(base) * (1.0 - EFFECT) {
        (
            ValidatedImprovement,
            format!("{counts}: fewer by half or more (Fisher, p={fewer:.4})"),
        )
    } else {
        (
            NoImprovement,
            format!("{counts}: not fewer by half (Fisher, p={fewer:.4})"),
        )
    }
}

/// What a token objective's rebuilt requests show, in input tokens a run
/// at the recorded trajectory. Exact, so a change of at least `min_effect`
/// either way is shown.
pub fn size_verdict(
    base: f64,
    candidate: f64,
    min_effect: f64,
    steps: u32,
    changed: u32,
) -> (ValidationOutcomeV1, String) {
    use ValidationOutcomeV1::*;
    let said = format!(
        "rebuilt at {steps} recorded step(s), the base sends ~{base:.0} input tokens a run and \
         the candidate ~{candidate:.0} ({:+.1}%)",
        100.0 * (candidate / base.max(1.0) - 1.0)
    );
    if changed == 0 {
        (
            Inconclusive,
            format!("{said}: the candidate sends every recorded step the same request"),
        )
    } else if base - candidate >= min_effect {
        (
            ValidatedImprovement,
            format!("{said}: fewer by the minimum effect ({min_effect:.0}) or more"),
        )
    } else if candidate - base >= min_effect {
        (
            Regression,
            format!("{said}: more by the minimum effect ({min_effect:.0}) or more"),
        )
    } else {
        (
            NoImprovement,
            format!("{said}: within the minimum effect ({min_effect:.0})"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fisher_matches_the_hypergeometric_tails() {
        // 3/10 against 9/10, the hypergeometric tails by hand.
        let (fewer, more) = fisher((9, 10), (3, 10));
        assert!((fewer - 0.009_883).abs() < 1e-5, "{fewer}");
        assert!((more - 0.999_643).abs() < 1e-5, "{more}");
        let (fewer, more) = fisher((5, 20), (5, 20));
        assert!(fewer > 0.5 && more > 0.5);
    }

    #[test]
    fn half_the_mistakes_shown_is_an_improvement_and_more_shown_a_regression() {
        assert_eq!(
            verdict((20, 60), (6, 60)).0,
            ValidationOutcomeV1::ValidatedImprovement
        );
        assert_eq!(
            verdict((20, 60), (14, 60)).0,
            ValidationOutcomeV1::NoImprovement
        );
        assert_eq!(
            verdict((6, 60), (20, 60)).0,
            ValidationOutcomeV1::Regression
        );
        assert_eq!(
            verdict((2, 60), (0, 60)).0,
            ValidationOutcomeV1::Inconclusive
        );
    }

    #[test]
    fn rebuilt_sizes_show_any_change_of_the_minimum_effect() {
        use ValidationOutcomeV1::*;
        assert_eq!(
            size_verdict(300_000.0, 280_000.0, 1000.0, 15, 15).0,
            ValidatedImprovement
        );
        assert_eq!(
            size_verdict(300_000.0, 299_500.0, 1000.0, 15, 15).0,
            NoImprovement
        );
        assert_eq!(
            size_verdict(300_000.0, 310_000.0, 1000.0, 15, 15).0,
            Regression
        );
        assert_eq!(
            size_verdict(300_000.0, 300_000.0, 1000.0, 15, 0).0,
            Inconclusive
        );
    }

    #[test]
    fn samples_spread_over_the_steps_within_bounds() {
        assert_eq!(per_step(1), MAX_PER_STEP);
        assert_eq!(per_step(6), 10);
        assert_eq!(per_step(40), MIN_PER_STEP);
    }

    fn update(files: Value) -> Value {
        json!({"role": "assistant", "timestamp": 1, "model": "m", "provider": "p",
            "stop_reason": "function_call", "content": [{"type": "function_call", "id": "c1",
            "function_id": "agent_trigger", "arguments": {"function": "coder::update-file",
            "description": "Edit", "payload": {"files": files}}}]})
    }

    #[test]
    fn a_call_off_its_schema_or_cut_short_is_a_mistake_and_a_valid_one_is_not() {
        let schemas = BTreeMap::from([(
            "coder::update-file".to_string(),
            json!({"type": "object", "required": ["files"], "properties": {"files": {
                "type": "array", "items": {"type": "object", "required": ["path"]}}}}),
        )]);
        assert_eq!(mistake(&update(json!([{"path": "a.py"}])), &schemas), None);
        assert_eq!(
            mistake(&update(json!([{"ops": []}])), &schemas).as_deref(),
            Some("coder::update-file: arguments do not match its request schema")
        );
        let mut cut = update(json!([]));
        cut["content"][0]["arguments"] = json!({"_raw": "{\"function\": \"coder::upd"});
        assert!(mistake(&cut, &schemas).unwrap().contains("not valid JSON"));
        assert_eq!(targets(&update(json!([]))), ["coder::update-file"]);
    }

    #[test]
    fn a_failed_call_is_replayed_where_it_was_made_and_where_its_error_was_read() {
        let made = update(json!([{"ops": []}]));
        let mut fixed = update(json!([{"path": "a.py"}]));
        fixed["content"][0]["id"] = json!("c2");
        let transcript = json!({"messages": [
            {"entry_id": "u", "message": {"role": "user", "content": []}},
            {"entry_id": "e_t_1_0_assistant", "origin": {"req": {"n": 1}}, "message": made},
            {"entry_id": "e_t_1_call_c1", "message": {"role": "function_result",
                "function_call_id": "c1", "is_error": true}},
            {"entry_id": "e_t_1_1_assistant", "origin": {"req": {"n": 3}}, "message": fixed},
        ]});
        let bundle = json!({"detail": {"reports": [{"report": {"scenarios": [
            {"scenario_id": "s", "runs": [{"run_id": "r", "transcript": transcript}]}]}}]}});
        let site: ErrorSiteV1 = serde_json::from_value(json!({"site_id": "x",
            "execution_id": "e", "scenario_id": "s", "run_id": "r", "entry_id": "e_t_1_call_c1",
            "function_id": "coder::update-file", "arguments": "", "error": "", "before": "",
            "after": "", "step": ""}))
        .unwrap();
        let found = points(&bundle, &[site]);
        let steps: Vec<(u64, bool, usize)> = found
            .iter()
            .map(|point| (point.step, point.after, point.entries.len()))
            .collect();
        assert_eq!(steps, [(0, false, 1), (1, true, 3)]);
        assert!(found.iter().all(|point| point.cause == made));
        assert_eq!(found[1].recorded, json!({"n": 3}));
    }

    #[test]
    fn a_step_is_named_by_its_assistant_entry() {
        assert_eq!(
            step_of("e_t_ab12_10_assistant"),
            Some(("t_ab12".into(), 10))
        );
        assert_eq!(step_of("e_t_ab12_call_00_x"), None);
    }
}

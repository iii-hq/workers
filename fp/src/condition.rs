//! `fp::condition` — [`util::when`]'s guard, shaped for a trigger binding.
//!
//! A binding's `conditions` entry is called with an envelope, not with the
//! target's own arguments:
//!
//! ```jsonc
//! { "event": { … }, "condition_config": { … }, "binding": { … }, "context": { … } }
//! ```
//!
//! That is the difference that makes this function possible at all. The
//! engine's own `condition_function_id` hands the raw event over as the WHOLE
//! payload, so a predicate has nowhere to read its threshold from and every
//! reusable function fails on the shape. Here the per-binding half arrives in
//! `condition_config`, so ONE function serves every binding that wants a
//! comparison:
//!
//! ```jsonc
//! "conditions": [ { "function_id": "fp::condition",
//!                   "config": { "path": "/new_value/findings", "op": ">=", "to": 3 } } ]
//! ```
//!
//! Without it, every predicate means writing and deploying a worker.
//!
//! The comparison is [`util::when`]'s, unchanged — same ops, same pointer
//! rules, same "a miss fails the guard rather than erroring". What is new is
//! only the envelope and the typed decision the binding expects back.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::util::{when, WhenOp};

pub const CONDITION_ID: &str = "fp::condition";
pub const CONDITION_DESC: &str =
    "Trigger-binding guard: compare a JSON pointer inside the fired event and answer the \
     condition contract ({ decision: \"allow\" | \"skip\", reason }). Config is \
     { path?, op, to?, negate? } with the fp::when ops (==, !=, >, >=, <, <=, exists, \
     not_empty); a pointer that resolves to nothing SKIPS rather than erroring, so \
     \"not there yet\" is an ordinary answer. Use it as a binding's `conditions` entry so a \
     reaction fires only on events that matter — it is not a fan-in gate (that is \
     state::barrier), it filters one event at a time.";

/// The caller-authored half, out of the binding's `conditions[].config`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ConditionConfig {
    /// JSON pointer into the EVENT (default: the whole event). Almost always
    /// wanted — comparing a whole event object is rarely what you mean.
    #[serde(default)]
    pub path: Option<String>,
    /// One of "==", "!=", ">", ">=", "<", "<=", "exists", "not_empty".
    pub op: WhenOp,
    /// Right-hand side for the comparison ops; rejected for exists/not_empty.
    #[serde(default)]
    pub to: Option<Value>,
    /// Invert the verdict. The only way to say "fire when this is ABSENT",
    /// since a pointer miss fails the guard and there is no `not_exists` op.
    /// Note the footgun that comes with it: a path that never resolves then
    /// fires on EVERY event.
    #[serde(default)]
    pub negate: bool,
}

/// The condition envelope. `binding` and `context` are accepted and ignored so
/// the same function also works as a direct call.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ConditionInput {
    pub event: Value,
    #[serde(default)]
    pub condition_config: Option<ConditionConfig>,
}

/// The typed answer a binding's condition contract expects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    Allow { reason: String },
    Skip { reason: String },
}

/// Evaluate one event against the config. `Err` is a WIRING bug (a missing
/// `to`, an ordering op on a string) — it will fail identically on every fire,
/// and the harness records the reason on the delivery rather than swallowing
/// it. A pointer miss is not that: it is `skip`, because "the value is not
/// there yet" is the ordinary case a guard exists to express.
pub fn decide(event: &Value, cfg: &ConditionConfig) -> Result<Decision, String> {
    let pointer = cfg.path.as_deref().unwrap_or("");
    let passed = when(event, cfg.path.as_deref(), cfg.op, cfg.to.as_ref())?;
    let verdict = passed != cfg.negate;

    // The reason lands on the delivery record, so it carries what was actually
    // seen: "why did this not fire" has to be answerable without a rerun.
    let seen = match event.pointer(pointer) {
        Some(v) => summarize(v),
        None => "nothing".to_string(),
    };
    let where_ = if pointer.is_empty() {
        "the event".to_string()
    } else {
        format!("`{pointer}`")
    };
    let negated = if cfg.negate { " (negated)" } else { "" };
    let reason = format!(
        "{CONDITION_ID}: {where_} is {seen}, {op}{negated}",
        op = describe(cfg.op, cfg.to.as_ref())
    );

    Ok(if verdict {
        Decision::Allow { reason }
    } else {
        Decision::Skip { reason }
    })
}

/// A short rendering of the observed value — enough to diagnose, short enough
/// to sit in a transcript entry.
fn summarize(v: &Value) -> String {
    const MAX: usize = 80;
    let rendered = match v {
        Value::String(s) => format!("{s:?}"),
        other => other.to_string(),
    };
    if rendered.chars().count() > MAX {
        let head: String = rendered.chars().take(MAX).collect();
        format!("{head}…")
    } else {
        rendered
    }
}

fn describe(op: WhenOp, to: Option<&Value>) -> String {
    let symbol = match op {
        WhenOp::Eq => "==",
        WhenOp::Ne => "!=",
        WhenOp::Gt => ">",
        WhenOp::Ge => ">=",
        WhenOp::Lt => "<",
        WhenOp::Le => "<=",
        WhenOp::Exists => return "expected: exists".to_string(),
        WhenOp::NotEmpty => return "expected: not_empty".to_string(),
    };
    match to {
        Some(v) => format!("expected {symbol} {}", summarize(v)),
        None => format!("expected {symbol} ?"),
    }
}

/// The registered entry point: pull the config out of the envelope and decide.
pub fn evaluate(input: ConditionInput) -> Result<Decision, String> {
    let cfg = input.condition_config.ok_or_else(|| {
        format!(
            "{CONDITION_ID} needs `condition_config` {{ path?, op, to?, negate? }} — as a binding \
             it goes in `conditions: [{{ function_id, config }}]`"
        )
    })?;
    decide(&input.event, &cfg)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(path: &str, op: WhenOp, to: Option<Value>) -> ConditionConfig {
        ConditionConfig {
            path: Some(path.into()),
            op,
            to,
            negate: false,
        }
    }

    fn event() -> Value {
        json!({ "scope": "run", "key": "f1", "new_value": { "findings": 4, "status": "done" } })
    }

    #[test]
    fn a_satisfied_comparison_allows() {
        let d = decide(
            &event(),
            &cfg("/new_value/findings", WhenOp::Ge, Some(json!(3))),
        )
        .unwrap();
        assert!(matches!(d, Decision::Allow { .. }));
    }

    #[test]
    fn an_unsatisfied_comparison_skips_and_says_what_it_saw() {
        let d = decide(
            &event(),
            &cfg("/new_value/findings", WhenOp::Ge, Some(json!(9))),
        )
        .unwrap();
        let Decision::Skip { reason } = d else {
            panic!("expected skip");
        };
        // The delivery record is the only place a mis-wired binding explains
        // itself, so the observed value has to be in the reason.
        assert!(reason.contains("/new_value/findings"), "{reason}");
        assert!(reason.contains('4'), "{reason}");
        assert!(reason.contains(">= 9"), "{reason}");
    }

    #[test]
    fn a_pointer_miss_skips_rather_than_erroring() {
        // "not there yet" is the ordinary case for a guard on a state key that
        // has not been written — it must not look like a wiring bug.
        let d = decide(
            &event(),
            &cfg("/new_value/missing", WhenOp::Eq, Some(json!(1))),
        )
        .unwrap();
        let Decision::Skip { reason } = d else {
            panic!("expected skip");
        };
        assert!(reason.contains("nothing"), "{reason}");
    }

    #[test]
    fn negate_inverts_the_verdict() {
        let mut c = cfg("/new_value/status", WhenOp::Eq, Some(json!("done")));
        assert!(matches!(
            decide(&event(), &c).unwrap(),
            Decision::Allow { .. }
        ));
        c.negate = true;
        let d = decide(&event(), &c).unwrap();
        assert!(matches!(d, Decision::Skip { .. }));
        if let Decision::Skip { reason } = d {
            assert!(reason.contains("negated"), "{reason}");
        }
    }

    #[test]
    fn negate_is_how_absence_fires() {
        // The gap it exists for: there is no `not_exists` op, and a miss fails
        // the guard, so without negate "fire when absent" is inexpressible.
        let c = ConditionConfig {
            path: Some("/new_value/done_marker".into()),
            op: WhenOp::Exists,
            to: None,
            negate: true,
        };
        assert!(matches!(
            decide(&event(), &c).unwrap(),
            Decision::Allow { .. }
        ));
    }

    #[test]
    fn a_config_bug_errors_instead_of_deciding() {
        // Permanent wiring bugs fail identically on every fire; the harness
        // records the reason. Deciding either way would hide them.
        let missing_to = decide(&event(), &cfg("/new_value/findings", WhenOp::Ge, None));
        assert!(missing_to.is_err());
        let bad_kind = decide(
            &event(),
            &cfg("/new_value/status", WhenOp::Gt, Some(json!(1))),
        );
        assert!(bad_kind.is_err());
    }

    #[test]
    fn exists_and_not_empty_read_the_event() {
        assert!(matches!(
            decide(&event(), &cfg("/new_value/status", WhenOp::Exists, None)).unwrap(),
            Decision::Allow { .. }
        ));
        assert!(matches!(
            decide(&event(), &cfg("/new_value/status", WhenOp::NotEmpty, None)).unwrap(),
            Decision::Allow { .. }
        ));
    }

    #[test]
    fn an_omitted_path_tests_the_whole_event() {
        let c = ConditionConfig {
            path: None,
            op: WhenOp::Exists,
            to: None,
            negate: false,
        };
        let d = decide(&event(), &c).unwrap();
        let Decision::Allow { reason } = d else {
            panic!("expected allow");
        };
        assert!(reason.contains("the event"), "{reason}");
    }

    #[test]
    fn the_envelope_requires_an_event_and_config() {
        assert!(serde_json::from_value::<ConditionInput>(json!({
            "condition_config": { "op": "exists" }
        }))
        .is_err());

        let input: ConditionInput = serde_json::from_value(json!({ "event": { "n": 1 } })).unwrap();
        let err = evaluate(input).unwrap_err();
        assert!(err.contains("condition_config"), "{err}");
    }

    #[test]
    fn the_envelope_tolerates_the_extra_keys_a_binding_sends() {
        // `binding` and `context` ride along in the harness envelope; ignoring
        // them is what lets the same function serve a direct call.
        let input: ConditionInput = serde_json::from_value(json!({
            "event": { "n": 5 },
            "condition_config": { "path": "/n", "op": ">=", "to": 3 },
            "binding": { "id": "sub_1", "fires": 0 },
            "context": { "owner_session_id": "s_1" },
        }))
        .unwrap();
        assert!(matches!(evaluate(input).unwrap(), Decision::Allow { .. }));
    }

    #[test]
    fn the_decision_serializes_to_the_condition_contract() {
        let v = serde_json::to_value(Decision::Skip {
            reason: "nope".into(),
        })
        .unwrap();
        assert_eq!(v["decision"], json!("skip"));
        assert_eq!(v["reason"], json!("nope"));
    }
}

//! Fire-time gating: the typed contract for the conditions a binding declares
//! (architecture/trigger-bindings.md § The condition contract).
//!
//! One rule shapes this module: a declared condition that ERRORS skips the
//! delivery and says why. The engine's own condition contract treats an
//! erroring condition as "proceed is false" silently, so one typo'd function
//! id starves a binding forever with no signal anywhere; here the reason
//! lands on the delivery record.
//!
//! There are no built-in reaction gates anymore — they existed to bound
//! trigger-spawned agent chains, and a binding no longer starts an agent.
//! Nothing throttles a wake or call binding besides its own `lifecycle` and
//! conditions.

use serde::Deserialize;
use serde_json::{json, Value};

use crate::bindings::Binding;
use crate::deps::Deps;

/// Why a fire did not deliver. Carried onto the delivery record so "it never
/// fired" is always answerable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skip {
    pub gate: &'static str,
    pub reason: String,
    /// Whether this blocked fire also retires the binding (a spent
    /// lifecycle); a plain condition skip leaves it armed.
    pub retire: bool,
}

impl Skip {
    fn new(gate: &'static str, reason: impl Into<String>, retire: bool) -> Self {
        Self {
            gate,
            reason: reason.into(),
            retire,
        }
    }
}

/// What a condition function returns.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "snake_case", tag = "decision")]
pub enum Decision {
    /// Deliver. A returned `payload` REPLACES the event for the remaining
    /// conditions and for the dispatched call — this is how a barrier hands
    /// its accumulated results downstream.
    Allow {
        #[serde(default)]
        payload: Option<Value>,
    },
    /// Do not deliver. A recurring binding stays armed.
    Skip {
        #[serde(default)]
        reason: Option<String>,
    },
}

/// Parse a condition's answer. An unparseable answer is a `skip` with the
/// reason attached rather than a silent pass: a condition that cannot say
/// what it decided has not decided anything.
pub fn parse_decision(raw: &Value) -> Result<Decision, String> {
    // A bare boolean is accepted as sugar — plenty of predicates already
    // answer `true`/`false`, and rejecting them would push authors into
    // wrapper functions for no gain.
    if let Some(b) = raw.as_bool() {
        return Ok(if b {
            Decision::Allow { payload: None }
        } else {
            Decision::Skip {
                reason: Some("condition returned false".into()),
            }
        });
    }
    serde_json::from_value(raw.clone()).map_err(|e| format!("undecipherable condition result: {e}"))
}

/// Run every declared condition in order, short-circuiting on the first skip.
/// Returns the (possibly condition-substituted) event to deliver.
pub async fn evaluate(deps: &Deps, binding: &Binding, event: Value) -> Result<Value, Skip> {
    let mut event = event;
    for condition in &binding.conditions {
        let payload = json!({
            "event": event,
            "condition_config": condition.config.clone().unwrap_or(Value::Null),
            "binding": { "id": binding.id, "fires": binding.fires },
            "context": { "owner_session_id": binding.owner.session_id },
        });
        let timeout_ms = deps.cfg().await.dispatch_timeout_ms;
        let raw = deps
            .iii
            .trigger(iii_sdk::protocol::TriggerRequest {
                function_id: condition.function_id.clone(),
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await;

        let decision = match raw {
            Ok(v) => parse_decision(&v),
            // An erroring condition is a SKIP with the error attached — never a
            // silent pass (which would run an ungated reaction) and never a
            // silent stall (the engine's behaviour).
            Err(e) => Err(format!("condition call failed: {e}")),
        };

        match decision {
            Ok(Decision::Allow { payload: None }) => {}
            Ok(Decision::Allow {
                payload: Some(next),
            }) => event = next,
            Ok(Decision::Skip { reason }) => {
                return Err(Skip::new(
                    "condition",
                    format!(
                        "`{}` skipped this fire: {}",
                        condition.function_id,
                        reason.unwrap_or_else(|| "no reason given".into())
                    ),
                    false,
                ))
            }
            Err(e) => {
                return Err(Skip::new(
                    "condition-error",
                    format!("`{}`: {e}", condition.function_id),
                    false,
                ))
            }
        }
    }
    Ok(event)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decisions_parse_from_the_typed_shape_and_bare_booleans() {
        assert!(matches!(
            parse_decision(&json!({ "decision": "allow" })).unwrap(),
            Decision::Allow { payload: None }
        ));
        assert!(matches!(
            parse_decision(&json!({ "decision": "allow", "payload": { "n": 1 } })).unwrap(),
            Decision::Allow { payload: Some(_) }
        ));
        assert!(matches!(
            parse_decision(&json!({ "decision": "skip", "reason": "not yet" })).unwrap(),
            Decision::Skip { .. }
        ));
        assert!(matches!(
            parse_decision(&json!(true)).unwrap(),
            Decision::Allow { .. }
        ));
        assert!(matches!(
            parse_decision(&json!(false)).unwrap(),
            Decision::Skip { .. }
        ));
    }

    #[test]
    fn an_undecipherable_answer_is_an_error_not_a_pass() {
        // The engine's contract passes anything that is not literally `false`;
        // ours refuses to guess, and the caller turns this into a recorded skip.
        assert!(parse_decision(&json!({ "ok": true })).is_err());
        assert!(parse_decision(&json!("yes")).is_err());
        assert!(parse_decision(&json!(null)).is_err());
    }
}

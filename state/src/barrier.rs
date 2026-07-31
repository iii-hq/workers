//! `state::barrier` — fan-in as an ordinary function.
//!
//! A barrier answers one question for a trigger binding: "has everything I am
//! waiting for arrived?" It records each arrival idempotently, answers `skip`
//! while the set is incomplete, and answers `allow` EXACTLY ONCE — on the
//! arrival that completes it — carrying every arrival's payload.
//!
//! It exists because the alternative is worse. Without it, a coordinator
//! watching N children either wakes once per child (N paid turns to learn one
//! fact) or hand-rolls a counter whose read-modify-write races. Here the
//! counting is a condition on the binding: the coordinator's own wake fires
//! once, when the work is actually done.
//!
//! Written to the condition contract, so it plugs into a binding as
//! `conditions: [{ function_id: "state::barrier", config: { id, expect } }]`.
//! Nothing about it is harness-specific — it is a function, and the decision
//! logic below is pure so it can be tested without a store.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use std::collections::HashSet;

/// Where a barrier's accumulated arrivals live. One key per barrier id.
pub const BARRIER_SCOPE: &str = "state_barrier";
/// Hard cap on one barrier's persisted arrival set.
const MAX_BARRIER_ARRIVALS: u64 = 10_000;

/// What the barrier is waiting for: a COUNT, or a named set. A named set is
/// stricter and better: it catches the arrival that was never going to come.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum Expect {
    Count(#[schemars(range(min = 1, max = 10000))] u64),
    Keys(#[schemars(length(min = 1, max = 10000))] Vec<String>),
}

impl Expect {
    fn total(&self) -> Result<usize, String> {
        let total = match self {
            Expect::Count(n) if *n > MAX_BARRIER_ARRIVALS => {
                return Err(format!(
                    "`expect` cannot exceed {MAX_BARRIER_ARRIVALS} arrivals"
                ));
            }
            Expect::Count(n) => usize::try_from(*n)
                .map_err(|_| "`expect` count does not fit this platform".to_string())?,
            Expect::Keys(keys)
                if u64::try_from(keys.len()).unwrap_or(u64::MAX) > MAX_BARRIER_ARRIVALS =>
            {
                return Err(format!(
                    "`expect` cannot exceed {MAX_BARRIER_ARRIVALS} arrivals"
                ));
            }
            Expect::Keys(keys) => keys.len(),
        };
        if total == 0 {
            return Err("`expect` must contain at least one arrival".to_string());
        }
        Ok(total)
    }

    /// Whether an arrival key belongs to this barrier. A count accepts any
    /// key; a named set accepts only its members.
    fn accepts(&self, key: &str) -> bool {
        match self {
            Expect::Count(_) => true,
            Expect::Keys(keys) => keys.iter().any(|k| k == key),
        }
    }
}

/// The caller-authored half: what this barrier is and how to read an arrival
/// out of the event.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct BarrierConfig {
    /// Barrier identity. Two bindings sharing an id share the barrier — that
    /// is how several predecessors feed one downstream.
    pub id: String,
    /// A count, or the exact set of arrival keys.
    pub expect: Expect,
    /// JSON pointer into the event for the arrival key (default `/key`, the
    /// state event's own key). A missing pointer is an error, not a silent
    /// unnamed arrival: two arrivals that both resolve to nothing would
    /// collapse into one and the barrier would never complete.
    #[serde(default)]
    pub key_from: Option<String>,
    /// JSON pointer into the event for the value to accumulate (default: the
    /// whole event).
    #[serde(default)]
    pub carry: Option<String>,
}

impl BarrierConfig {
    pub const DEFAULT_KEY_FROM: &'static str = "/key";

    fn key_pointer(&self) -> &str {
        self.key_from.as_deref().unwrap_or(Self::DEFAULT_KEY_FROM)
    }
}

/// The typed decision a condition returns.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum Decision {
    Allow {
        #[serde(skip_serializing_if = "Option::is_none")]
        payload: Option<Value>,
        reason: String,
    },
    Skip {
        reason: String,
    },
}

/// The persisted barrier: who has arrived, what they carried, and whether the
/// completing arrival has already been answered.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BarrierState {
    #[serde(default)]
    pub arrived: Map<String, Value>,
    #[serde(default)]
    pub complete: bool,
}

/// One arrival, applied to the barrier's state. PURE — the storage layer's job
/// is only to run this against the current value and write the result back
/// atomically. Returns the new state and the decision to answer with.
pub fn arrive(
    current: Option<&Value>,
    cfg: &BarrierConfig,
    event: &Value,
) -> Result<(BarrierState, Decision), String> {
    let total = cfg
        .expect
        .total()
        .map_err(|e| format!("barrier `{}`: {e}", cfg.id))?;

    if let Expect::Keys(keys) = &cfg.expect {
        let mut seen = HashSet::with_capacity(keys.len());
        if let Some(duplicate) = keys.iter().find(|key| !seen.insert(key.as_str())) {
            return Err(format!(
                "barrier `{}`: duplicate expected arrival key `{duplicate}`",
                cfg.id
            ));
        }
    }

    let mut state: BarrierState = match current {
        Some(Value::Null) | None => BarrierState::default(),
        Some(v) => serde_json::from_value(v.clone())
            .map_err(|e| format!("barrier `{}` state is unreadable: {e}", cfg.id))?,
    };

    let pointer = cfg.key_pointer();
    let key = match event.pointer(pointer) {
        Some(Value::String(s)) => s.clone(),
        Some(other) if !other.is_null() => other.to_string(),
        _ => {
            return Err(format!(
                "barrier `{}`: no arrival key at `{pointer}` in the event — set `key_from` to a \
                 pointer that resolves, or arrivals cannot be told apart",
                cfg.id
            ));
        }
    };

    // Answered already: every later arrival is a no-op, including a redelivery
    // of the one that completed it.
    if state.complete {
        return Ok((
            state,
            Decision::Skip {
                reason: format!("barrier `{}` already complete", cfg.id),
            },
        ));
    }

    if !cfg.expect.accepts(&key) {
        return Ok((
            state,
            Decision::Skip {
                reason: format!(
                    "barrier `{}`: `{key}` is not one of the expected arrivals — check the \
                     producer's key against `expect`",
                    cfg.id
                ),
            },
        ));
    }

    let carried = match &cfg.carry {
        Some(p) => event.pointer(p).cloned().unwrap_or(Value::Null),
        None => event.clone(),
    };
    // Idempotent: the same arrival twice counts once. At-least-once delivery
    // makes this the normal case, not an edge case.
    let first_time = state.arrived.insert(key.clone(), carried).is_none();

    if state.arrived.len() >= total {
        state.complete = true;
        return Ok((
            state.clone(),
            Decision::Allow {
                payload: Some(json!({ "results": state.arrived })),
                reason: format!("barrier `{}` complete ({}/{total})", cfg.id, total),
            },
        ));
    }

    let n = state.arrived.len();
    let reason = if first_time {
        let waiting_on = match &cfg.expect {
            Expect::Count(_) => format!("{} more", total - n),
            Expect::Keys(keys) => format!(
                "{:?}",
                keys.iter()
                    .filter(|key| !state.arrived.contains_key(*key))
                    .collect::<Vec<_>>()
            ),
        };
        format!(
            "barrier `{}`: {n}/{total} arrived, waiting on {waiting_on}",
            cfg.id
        )
    } else {
        format!(
            "barrier `{}`: `{key}` already counted, still {n}/{total}",
            cfg.id
        )
    };
    Ok((state, Decision::Skip { reason }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(expect: Expect) -> BarrierConfig {
        BarrierConfig {
            id: "run-x".into(),
            expect,
            key_from: None,
            carry: None,
        }
    }

    fn event(key: &str, value: Value) -> Value {
        json!({ "scope": "run", "key": key, "new_value": value })
    }

    fn state_value(s: &BarrierState) -> Value {
        serde_json::to_value(s).unwrap()
    }

    #[test]
    fn a_count_barrier_skips_until_the_last_arrival() {
        let c = cfg(Expect::Count(3));
        let (s1, d1) = arrive(None, &c, &event("a", json!(1))).unwrap();
        let Decision::Skip { reason } = d1 else {
            panic!("the first arrival must skip");
        };
        assert!(reason.contains("waiting on 2 more"), "{reason}");
        assert!(!reason.contains("<unnamed>"), "{reason}");
        let (s2, d2) = arrive(Some(&state_value(&s1)), &c, &event("b", json!(2))).unwrap();
        assert!(matches!(d2, Decision::Skip { .. }));

        let (s3, d3) = arrive(Some(&state_value(&s2)), &c, &event("c", json!(3))).unwrap();
        let Decision::Allow { payload, .. } = d3 else {
            panic!("the third arrival must complete the barrier: {d3:?}");
        };
        assert!(s3.complete);
        let results = payload.unwrap();
        assert_eq!(results["results"].as_object().unwrap().len(), 3);
        // Every arrival's carried value is there, not just the last.
        assert_eq!(results["results"]["a"]["new_value"], json!(1));
        assert_eq!(results["results"]["c"]["new_value"], json!(3));
    }

    #[test]
    fn it_allows_exactly_once() {
        // The property the whole thing exists for: at-least-once delivery must
        // not produce two downstreams.
        let c = cfg(Expect::Count(1));
        let (s, d) = arrive(None, &c, &event("a", json!(1))).unwrap();
        assert!(matches!(d, Decision::Allow { .. }));
        let (_, again) = arrive(Some(&state_value(&s)), &c, &event("a", json!(1))).unwrap();
        match again {
            Decision::Skip { reason } => assert!(reason.contains("already complete")),
            other => panic!("a re-arrival after completion must skip, got {other:?}"),
        }
        // …including a DIFFERENT arrival landing late.
        let (_, late) = arrive(Some(&state_value(&s)), &c, &event("z", json!(9))).unwrap();
        assert!(matches!(late, Decision::Skip { .. }));
    }

    #[test]
    fn a_duplicate_arrival_counts_once() {
        let c = cfg(Expect::Count(2));
        let (s1, _) = arrive(None, &c, &event("a", json!(1))).unwrap();
        // Same key again — a redelivery, not progress.
        let (s2, d2) = arrive(Some(&state_value(&s1)), &c, &event("a", json!(1))).unwrap();
        assert_eq!(s2.arrived.len(), 1);
        match d2 {
            Decision::Skip { reason } => assert!(reason.contains("already counted")),
            other => panic!("expected a duplicate skip, got {other:?}"),
        }
        // The real second arrival still completes it.
        let (_, d3) = arrive(Some(&state_value(&s2)), &c, &event("b", json!(2))).unwrap();
        assert!(matches!(d3, Decision::Allow { .. }));
    }

    #[test]
    fn a_named_set_names_what_is_missing() {
        let c = cfg(Expect::Keys(vec!["a".into(), "b".into()]));
        let (s1, d1) = arrive(None, &c, &event("a", json!(1))).unwrap();
        match d1 {
            Decision::Skip { reason } => assert!(reason.contains("\"b\""), "{reason}"),
            other => panic!("expected skip naming the missing key, got {other:?}"),
        }
        let (_, d2) = arrive(Some(&state_value(&s1)), &c, &event("b", json!(2))).unwrap();
        assert!(matches!(d2, Decision::Allow { .. }));
    }

    #[test]
    fn duplicate_named_expectations_are_rejected() {
        let c = cfg(Expect::Keys(vec!["a".into(), "a".into()]));
        let err = arrive(None, &c, &event("a", json!(1))).unwrap_err();
        assert!(err.contains("duplicate expected arrival key `a`"), "{err}");
    }

    #[test]
    fn empty_and_oversized_expectations_are_rejected() {
        for expect in [Expect::Count(0), Expect::Keys(vec![])] {
            let err = arrive(None, &cfg(expect), &event("a", json!(1))).unwrap_err();
            assert!(err.contains("at least one arrival"), "{err}");
        }

        let err = arrive(None, &cfg(Expect::Count(u64::MAX)), &event("a", json!(1))).unwrap_err();
        assert!(err.contains("cannot exceed"), "{err}");
    }

    #[test]
    fn expectation_schema_exposes_the_runtime_bounds() {
        let schema = serde_json::to_value(schemars::schema_for!(Expect)).unwrap();
        let branches = schema["anyOf"].as_array().unwrap();
        assert!(
            branches.iter().any(|branch| {
                branch["minimum"].as_f64() == Some(1.0)
                    && branch["maximum"].as_f64() == Some(10_000.0)
            }),
            "{schema}"
        );
        assert!(
            branches.iter().any(|branch| {
                branch["minItems"] == json!(1) && branch["maxItems"] == json!(10_000)
            }),
            "{schema}"
        );
    }

    #[test]
    fn an_unexpected_arrival_is_refused_by_name() {
        // A named barrier catches the producer whose key never matched — the
        // wiring bug that otherwise starves the join at N-1 forever.
        let c = cfg(Expect::Keys(vec!["a".into(), "b".into()]));
        let (s, d) = arrive(None, &c, &event("typo", json!(1))).unwrap();
        assert!(
            s.arrived.is_empty(),
            "an unexpected key must not be counted"
        );
        match d {
            Decision::Skip { reason } => assert!(reason.contains("`typo`"), "{reason}"),
            other => panic!("expected a named refusal, got {other:?}"),
        }
    }

    #[test]
    fn an_oversized_named_set_is_refused() {
        // One past the cap — the named-set arm of the limit, the count arm
        // has its own test.
        let keys: Vec<String> = (0..=MAX_BARRIER_ARRIVALS).map(|i| i.to_string()).collect();
        let err = arrive(None, &cfg(Expect::Keys(keys)), &event("0", json!(1))).unwrap_err();
        assert!(err.contains("cannot exceed"), "{err}");
    }

    #[test]
    fn a_missing_arrival_key_errors_rather_than_collapsing() {
        // Two arrivals that both resolve to nothing would land on one key and
        // the barrier would never complete. Refuse instead.
        let c = BarrierConfig {
            key_from: Some("/nope".into()),
            ..cfg(Expect::Count(2))
        };
        let err = arrive(None, &c, &event("a", json!(1))).unwrap_err();
        assert!(err.contains("no arrival key"), "{err}");
    }

    #[test]
    fn carry_selects_what_the_downstream_receives() {
        let c = BarrierConfig {
            carry: Some("/new_value".into()),
            ..cfg(Expect::Count(1))
        };
        let (_, d) = arrive(None, &c, &event("a", json!({ "ok": true }))).unwrap();
        let Decision::Allow { payload, .. } = d else {
            panic!("expected completion");
        };
        assert_eq!(payload.unwrap()["results"]["a"], json!({ "ok": true }));
    }

    #[test]
    fn a_non_string_arrival_key_is_stringified() {
        let c = BarrierConfig {
            key_from: Some("/new_value/n".into()),
            ..cfg(Expect::Count(2))
        };
        let (s1, _) = arrive(None, &c, &event("a", json!({ "n": 1 }))).unwrap();
        let (s2, d2) = arrive(Some(&state_value(&s1)), &c, &event("b", json!({ "n": 2 }))).unwrap();
        assert_eq!(s2.arrived.len(), 2);
        assert!(matches!(d2, Decision::Allow { .. }));
    }

    #[test]
    fn unreadable_state_errors_instead_of_restarting_the_count() {
        // Silently starting over would let a completed barrier fire twice.
        let c = cfg(Expect::Count(2));
        let err = arrive(Some(&json!("not a barrier")), &c, &event("a", json!(1))).unwrap_err();
        assert!(err.contains("unreadable"), "{err}");
    }
}

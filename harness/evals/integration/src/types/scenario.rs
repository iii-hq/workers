//! Authored integration-scenario V1, its strict compiled runtime form, and
//! the stable result contract.
//!
//! The checked-in contract intentionally contains only scenario intent.
//! [`crate::expand::compile_scenario`] derives run-scoped ids, the exact
//! `harness::send` payload, recorder configuration, router matchers/frames,
//! and the common completion invariants before the stack starts.

mod authored;
mod compiled;
mod expectations;
mod invariants;
mod result;

pub use authored::*;
pub use compiled::*;
pub use expectations::*;
pub use invariants::*;
pub use result::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn authored_defaults_are_safe() {
        let timeouts = DeadlinesV1::default();
        assert_eq!(timeouts.readiness_ms, 60_000);
        assert_eq!(timeouts.scenario_ms, 60_000);
        assert_eq!(timeouts.teardown_ms, 15_000);

        // Assertions are explicit: an empty expectation set grades nothing,
        // and the compiler rejects it for missing the mandatory floor.
        let expectations = ExpectationsV1::default();
        assert!(!expectations.turn_completes);
        assert!(!expectations.script_fully_consumed);
        assert!(!expectations.no_duplicates);
        assert_eq!(expectations.terminal.status, TerminalStatusV1::Completed);
        assert!(expectations.lifecycle.allow_identical_duplicates);
    }

    #[test]
    fn invariant_registry_round_trips_typed_parameters() {
        let spec = InvariantSpecV1::target_calls(TargetCallsInvariantV1 {
            function_id: "run::record".into(),
            count: 1,
            payload: Some(serde_json::json!({ "value": "expected" })),
            payload_subset: None,
        });
        assert_eq!(spec.id, InvariantKind::TargetCalls.as_str());
        assert_eq!(
            InvariantKind::from_id(&spec.id),
            Some(InvariantKind::TargetCalls)
        );
        assert_eq!(spec.parameters["count"], 1);
        assert_eq!(spec.parameters["function_id"], "run::record");
    }

    #[test]
    fn invariant_registry_rejects_unknown_ids_but_preserves_parameters() {
        assert_eq!(InvariantKind::from_id("nonsense.id"), None);

        let malformed = InvariantSpecV1 {
            id: InvariantKind::TargetCalls.as_str().into(),
            parameters: serde_json::Map::new(),
        };
        assert_eq!(
            InvariantKind::from_id(&malformed.id),
            Some(InvariantKind::TargetCalls)
        );
        assert!(malformed.parameters.is_empty());
    }
}

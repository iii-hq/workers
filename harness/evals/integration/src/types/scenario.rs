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

        let expectations = ExpectationsV1::default();
        assert!(expectations.no_duplicates);
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
        let Ok(DecodedInvariantV1::TargetCalls(parameters)) = spec.decode() else {
            panic!("target.calls must dispatch through its typed invariant kind");
        };
        assert_eq!(parameters["count"], 1);
        assert_eq!(parameters["function_id"], "run::record");
    }

    #[test]
    fn invariant_registry_rejects_unknown_ids_but_preserves_parameters() {
        let unknown = InvariantSpecV1 {
            id: "nonsense.id".into(),
            parameters: serde_json::Map::new(),
        };
        assert!(matches!(
            unknown.decode(),
            Err(InvariantDecodeError::UnknownId(_))
        ));

        let malformed = InvariantSpecV1 {
            id: InvariantKind::TargetCalls.as_str().into(),
            parameters: serde_json::Map::new(),
        };
        let Ok(DecodedInvariantV1::TargetCalls(parameters)) = malformed.decode() else {
            panic!("known ids must decode without constraining their parameter map");
        };
        assert!(parameters.is_empty());
    }
}

//! Authored integration-scenario V1, its strict compiled runtime form, and
//! the stable result contract.
//!
//! The checked-in contract intentionally contains only scenario intent.
//! [`crate::expand::compile_scenario`] derives run-scoped ids, the exact
//! `harness::send` payload, recorder configuration, and router
//! matchers/frames before the stack starts. Run outcomes are checked by the
//! runner-owned floor and each scenario's `verify` function over
//! [`crate::evidence_data::RunEvidence`].

mod authored;
mod compiled;
mod result;

pub use authored::*;
pub use compiled::*;
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
    }
}

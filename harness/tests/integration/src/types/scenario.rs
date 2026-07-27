//! Strict scenario runtime and result contracts.

mod compiled;
mod result;

pub use compiled::*;
pub use result::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_defaults_are_safe() {
        let timeouts = DeadlinesV1::default();
        assert_eq!(timeouts.readiness_ms, 60_000);
        assert_eq!(timeouts.scenario_ms, 25_000);
        assert_eq!(timeouts.teardown_ms, 15_000);
        // The await deadline must stay well clear of the slowest passing
        // scenario (~4s in CI) so a slow runner does not read as a failure.
        assert!(timeouts.scenario_ms >= 5 * 4_000);
    }
}
